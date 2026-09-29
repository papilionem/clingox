//! Regression test: `TheoryAtoms::term`, `TheoryTerm`'s `Display`, and the
//! derived `Clone`, `PartialEq` and `Drop` all recursed once per level of a
//! theory term; a deeply left-nested term (`1+1+1+...`, clingo happily grounds
//! it) then overflowed the stack, aborting the whole process, well before any
//! depth a real program plausibly builds by hand -- but well within what a
//! generated or externally-supplied program can reach (depth 500 already aborts
//! a debug build's default 8 MB stack, and clingo grounds depth 10000 without
//! trouble). Resolving, `Display`, `Clone`, `PartialEq` and `Drop` are all
//! iterative; the acceptance shape is depth 100000 resolving, printing,
//! cloning, comparing and dropping on a 2 MB thread in a debug build.
//!
//! **A stack overflow is process-level, not a catchable panic**: Rust's own
//! guard-page handler prints "thread has overflowed its stack" and calls
//! `abort()` for the whole process, whatever thread it happened on. Run
//! directly against the unfixed code, this test would therefore kill the entire
//! test binary -- every other test sharing that process -- with no per-test
//! failure report at all. As `api_solve_events.rs`'s own child- process guard
//! does for the same reason, the dangerous work runs in a **child process**
//! that re-executes this test binary; the parent checks the child's exit status
//! and a completion marker on its stdout, so a regression here is reported as
//! one failing test, not a silent process kill.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::print_stderr,
    reason = "progress markers for work that risks a process-level stack overflow, to localize \
              a crash by depth and stage if the fix regresses"
)]

use std::io::Write;
use std::process::Command;

use clingox::{Control, Part};

const CHILD_SCENARIO: &str = "CLINGOX_TEST_DEEP_THEORY_TERM_CHILD";
const DONE: &str = "deep-theory-term-child-done";

/// A depth-`n` left-nested addition, e.g. `1+1+1` for `n = 3`: `#theory t
/// { term { + : 1, binary, left }; &a/0 : term, head }. &a { <expr> }.`.
fn deep_theory(n: usize) -> String {
    let expr = vec!["1"; n].join("+");
    format!("#theory t {{ term {{ + : 1, binary, left }}; &a/0 : term, head }}. &a {{ {expr} }}.")
}

/// Grounds a depth-`n` theory term and resolves it, printing progress to
/// stderr so a crash's location shows in the child's captured output.
fn resolve_clone_compare_drop(ctl: &Control, n: usize) {
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let id = atom.elements().unwrap()[0].tuple().unwrap()[0];

    let term = atoms.term(id).unwrap();
    eprintln!("resolved depth {n}");

    let text = term.to_string();
    eprintln!("displayed depth {n}: {} chars", text.len());
    assert!(
        text.starts_with('('),
        "a left-nested sum prints parenthesised: {}",
        &text[..20.min(text.len())]
    );

    let cloned = term.clone();
    eprintln!("cloned depth {n}");
    assert_eq!(term, cloned, "a term must compare equal to its own clone");
    eprintln!("compared depth {n}");

    drop(term);
    drop(cloned);
    eprintln!("dropped depth {n}");
}

/// Stack for the deep-recursion threads. 512 MiB on 64-bit targets; a 32-bit
/// process has 2 to 4 GiB of address space in all, so 128 MiB there, which is
/// still more than clingo needs at these depths (8 to 64 MB at depth 100 000
/// on 64-bit, whose frames are larger).
const DEEP_STACK: usize = if cfg!(target_pointer_width = "64") {
    512 << 20
} else {
    128 << 20
};

/// Grounds a depth-`n` term. clingo's own grounder recurses once per level
/// (at depth 100000 it needs between 8 and 64 MB of stack), which is outside
/// clingox, so grounding runs on a thread with a 512 MB stack.
fn ground_deep(n: usize) -> Control {
    std::thread::Builder::new()
        .stack_size(DEEP_STACK)
        .spawn(move || {
            let mut ctl = Control::new().unwrap();
            ctl.add_base(&deep_theory(n)).unwrap();
            ctl.ground(&[Part::base()]).unwrap();
            eprintln!("grounded depth {n}");
            ctl
        })
        .unwrap()
        .join()
        .unwrap()
}

/// Runs clingox's own work at the acceptance depth on a 2 MB stack, the
/// shape that fix targets. clingo's `term_to_string` is not called at this
/// depth (clingox's debug cross-check stops at a bounded depth).
fn run_on_a_2mb_thread(n: usize) {
    let ctl = ground_deep(n);
    // The control comes back from the 2 MB thread before it is dropped:
    // freeing a deep program is clingo's own recursion too.
    let ctl = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            resolve_clone_compare_drop(&ctl, n);
            ctl
        })
        .unwrap()
        .join()
        .unwrap();
    // Dropped on a large stack too: a test thread has only 2 MB, which
    // clingo's recursive cleanup of a deep program exceeds under ASan.
    std::thread::Builder::new()
        .stack_size(DEEP_STACK)
        .spawn(move || drop(ctl))
        .unwrap()
        .join()
        .unwrap();
}

/// The child half: only runs when `CHILD_SCENARIO` is set, so `cargo test`
/// alone never runs the dangerous work directly.
#[test]
fn deep_theory_term_100000_child() {
    let Ok(depth) = std::env::var(CHILD_SCENARIO) else {
        return;
    };
    let n: usize = depth.parse().unwrap();
    run_on_a_2mb_thread(n);
    let mut out = std::io::stdout();
    writeln!(out, "{DONE}").expect("stdout is a pipe to the parent test");
}

/// The parent half: re-executes this test binary for the depth-100000
/// acceptance shape and checks it exits cleanly with the completion marker,
/// so a stack overflow in the child is reported as this one test failing.
#[test]
#[cfg_attr(
    any(target_os = "android", target_os = "ios", target_family = "wasm"),
    ignore = "cannot spawn a child process here; the in-process test below runs instead"
)]
fn deep_theory_term_depth_100000_resolves_displays_clones_compares_and_drops_on_2mb() {
    if std::env::var_os(CHILD_SCENARIO).is_some() {
        return;
    }
    let exe = std::env::current_exe().expect("the test binary knows its path");
    let output = Command::new(&exe)
        .args([
            "--exact",
            "deep_theory_term_100000_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_SCENARIO, "100000")
        .output()
        .expect("the test binary runs again");
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !stdout.contains(DONE) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let report: String = stderr
            .lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "depth 100000 must resolve, display, clone, compare and drop on a 2 MB thread \
             without crashing the process: exit {:?} (a stack overflow aborts with no normal \
             panic message)\n{report}",
            output.status
        );
    }
}

/// The same acceptance shape, run in process, for targets that cannot spawn
/// a child (as in `api_solve_events.rs`).
/// A regression here ends the whole test binary instead of failing one
/// test, which is still a failure.
#[test]
#[cfg_attr(
    not(any(target_os = "android", target_os = "ios", target_family = "wasm")),
    ignore = "the child-process guard above covers this where processes can be spawned"
)]
fn deep_theory_term_depth_100000_in_process() {
    // Without OS threads (the default WebAssembly build) no stack size can be
    // chosen, and clingo's own grounder overflows the fixed 64 KB stack long
    // before clingox's code is reached, so there is nothing to test here.
    if cfg!(all(target_family = "wasm", not(target_feature = "atomics"))) {
        return;
    }
    run_on_a_2mb_thread(100_000);
}

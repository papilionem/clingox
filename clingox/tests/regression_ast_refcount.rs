//! clingo's AST reference count is a 32-bit
//! `unsigned` that `incRef` incremented without a check, and `std::mem::forget`
//! is safe, so 2^32 forgotten clones wrapped the count to zero; one more clone
//! and drop then freed a node a live `Ast` still pointed to. The patch (U47)
//! makes `incRef` abort the process at the maximum count, as `std::rc::Rc`
//! does, so the safe API can never reach a use after free.
//!
//! The overflow test takes over a minute (it forgets 2^32 - 1 clones) and runs
//! in a child process that must die of SIGABRT: not exit cleanly, not SIGSEGV,
//! and not print what a freed node now holds. It is `#[ignore]`d and run with
//! `--ignored` explicitly. It must stay out of the
//! sanitizer sweep (`xtask/src/sanitize.rs` excludes it): under `ASan` it would
//! run for hours, and the abort is the behaviour under test, not a finding.
//!
//! A build against a system clingo has no patch (RULES 8); the overflow test
//! returns early there, because without it the child would end in a use after
//! free.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use clingox::ast::{self, Ast, Span};

const ENTRY: &str = "child_entry";

fn node(name: &str) -> Ast {
    let span = Span::new("<t>", 1, 1, "<t>", 1, 2).unwrap();
    ast::variable(&span, name).unwrap()
}

/// The child side: runs the case named by the environment, or does nothing.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    assert_eq!(case, "overflow");
    let victim = node("Victim"); // count 1
    // Count 1 + (u32::MAX - 1) is the maximum a 32-bit counter holds.
    for _ in 0..u32::MAX - 1 {
        std::mem::forget(victim.clone());
    }
    println!("CHILD-AT-MAX");
    // One more reference would wrap the count: the patch aborts here.
    let extra = victim.clone();
    println!("CHILD-SURVIVED");
    drop(extra);
    let others: Vec<Ast> = (0..64).map(|i| node(&format!("Other{i}"))).collect();
    println!("CHILD-READS {victim}");
    std::mem::forget(victim);
    drop(others);
}

#[test]
#[ignore = "forgets 2^32 clones: over a minute; run with --ignored"]
#[cfg(all(unix, target_pointer_width = "64"))]
fn a_reference_count_at_its_maximum_aborts_instead_of_wrapping() {
    if !clingox_sys::VENDORED {
        eprintln!("SKIPPED: a system clingo has no reference count patch (RULES 8)");
        return;
    }
    assert!(
        clingox_sys::PATCHES.contains(&"U47"),
        "the vendored build applies U47: {:?}",
        clingox_sys::PATCHES
    );
    let Some(out) =
        child::run_child_with(ENTRY, "overflow", std::time::Duration::from_mins(20), None)
    else {
        return;
    };
    let report = format!("{out:?}");
    assert!(
        out.stdout.contains("CHILD-AT-MAX"),
        "the child never reached the maximum count: {report}"
    );
    assert!(
        !out.stdout.contains("CHILD-SURVIVED") && !out.stdout.contains("CHILD-READS"),
        "the clone past the maximum returned: {report}"
    );
    assert_eq!(out.code, None, "not a clean or ordinary exit: {report}");
    assert_eq!(
        out.signal,
        Some(6),
        "killed by SIGABRT, not SIGSEGV (11) or anything else: {report}"
    );
}

/// The check must not disturb ordinary use: clone and drop stay balanced.
#[test]
fn clone_and_drop_stay_balanced() {
    let victim = node("Victim");
    let clones: Vec<Ast> = (0..1000).map(|_| victim.clone()).collect();
    assert!(clones.iter().all(|c| c == &victim));
    drop(clones);
    let others: Vec<Ast> = (0..64).map(|i| node(&format!("Other{i}"))).collect();
    assert_eq!(victim.to_string(), "Victim");
    drop(others);
    let again = victim.clone();
    drop(victim);
    assert_eq!(again.to_string(), "Victim");
}

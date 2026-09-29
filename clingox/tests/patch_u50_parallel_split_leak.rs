//! Patch U50 (`clingox-sys/patches/U50-parallel-split-leak.patch`): a parallel
//! search in splitting mode leaked the guiding paths that had been split off
//! and not yet taken by an idle thread when the search was stopped
//! (UPSTREAM-ISSUES U50). Found by the leak sanitizer in the printer tests,
//! which stop a `-t4` search from a failing model printer.
//!
//! The leak is invisible without a leak detector, so the tests assert nothing
//! about it: `cargo xtask sanitize` runs this file under `LeakSanitizer`, and
//! without the patch the process exits with a report for `Clasp::Solver::split`
//! (checked: 1500 rounds leaked 4424 bytes in 220 allocations; a race decides
//! whether a round leaks, so the loop is long). The loop runs a splitting search
//! on eight threads, cancels it after a delay that changes from round to round,
//! and drops the control, so the shared data of the parallel solve is destroyed
//! with whatever the interrupt left in its work queue.
//!
//! A system clingo has no patch (RULES 8): the tests return early there. On a
//! build without threads there is no parallel search.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test helpers fail loudly on unexpected errors, and say why they skip"
)]

use std::thread;
use std::time::Duration;

use clingox::{Control, Part};

/// 11 pigeons in 10 holes: unsatisfiable, and hard enough that the threads
/// are still splitting the search when it is cancelled.
const HARD: &str =
    "p(1..11). h(1..10). 1 { at(P,H) : h(H) } 1 :- p(P). :- at(P,H), at(Q,H), P < Q.";

#[test]
fn the_vendored_build_applies_u50() {
    if !clingox_sys::VENDORED {
        eprintln!("SKIPPED: a system clingo has no split patch (RULES 8)");
        return;
    }
    assert!(
        clingox_sys::PATCHES.contains(&"U50"),
        "the vendored build applies U50: {:?}",
        clingox_sys::PATCHES
    );
}

#[test]
fn cancelling_a_splitting_search_frees_the_queued_paths() {
    if !clingox_sys::HAS_THREADS {
        eprintln!("SKIPPED: this build has no threads");
        return;
    }
    for round in 0..500_u64 {
        let mut ctl = Control::with_args(["--parallel-mode=8,split"]).unwrap();
        ctl.add_base(HARD).unwrap();
        ctl.ground(&[Part::base()]).unwrap();
        {
            let mut handle = ctl.solve_async(&[]).unwrap();
            // 0.2 to 3 ms: while the threads are handing paths to each other.
            thread::sleep(Duration::from_micros(200 + (round % 20) * 150));
            handle.cancel().unwrap();
        }
        drop(ctl);
    }
}

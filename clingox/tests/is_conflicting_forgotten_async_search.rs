//! Regression test:
//! `Control::is_conflicting`, `Control::enable_cleanup` and
//! `Control::enable_enumeration_assumption` all take `&self` and read
//! straight through to clingo without going through `Control::observed` (or
//! any other path that runs `finish_search`, DESIGN S4's own rule for
//! every entry point). A [`clingox::AsyncSolveHandle`] left running by
//! `std::mem::forget` (safe and documented: the control closes it at its
//! next call) is therefore still running on clasp's own threads while
//! these three calls read clasp's internal state directly: `TSan` reports a
//! race in `SharedContext::ok`. The file is meant to be run under `TSan`
//! (`cargo xtask sanitize`).
//!
//! `is_conflicting` finishes any leftover search
//! first (S4), and stays callable on a poisoned control; the same for the
//! enable getters, for uniformity.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::time::{Duration, Instant};

use clingox::{Control, Part};

fn has_threads() -> bool {
    clingox_sys::HAS_THREADS
}

/// A pigeonhole program sized to keep clasp busy for tens of milliseconds.
const HARD: &str =
    "p(1..12). h(1..11). 1 { in(P,H) : h(H) } 1 :- p(P). :- in(P,H), in(Q,H), P < Q.";

/// The original `TSan` repro: a forgotten, still-running async search,
/// read through the three `&self` entry points repeatedly while it runs.
/// This is the shape run under `cargo xtask sanitize`'s thread sanitizer
/// (`san.sh thread`); see the module doc for where that report lives.
#[test]
fn is_conflicting_after_forgetting_a_running_async_search() {
    if !has_threads() {
        return;
    }
    let mut ctl = Control::with_args(["-t", "2"]).unwrap();
    ctl.add_base(HARD).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let handle = ctl.solve_async(&[]).unwrap();
    std::mem::forget(handle);

    // The search is still running on clasp's own threads here: S4 says
    // every entry point, `&self` included, closes it first.
    let start = Instant::now();
    let mut reads = 0u64;
    while start.elapsed() < Duration::from_millis(300) {
        let _ = ctl.is_conflicting();
        let _ = ctl.enable_cleanup();
        let _ = ctl.enable_enumeration_assumption();
        reads += 1;
    }
    assert!(
        reads > 0,
        "the loop above must run at least once to exercise the race"
    );

    // The control must still be readable afterward: a real bug here is a
    // data race, which does not reliably corrupt state in a plain
    // (non-TSan) run, so this is a sanity floor, not the regression check
    // itself -- see the module doc for the sanitizer evidence. Dropping
    // `ctl` here (rather than solving it to completion) closes the leftover
    // search by cancelling it, not by waiting out the pigeonhole instance's
    // own, deliberately hard, unsatisfiability proof.
    assert!(!format!("{ctl:?}").contains("poisoned"), "{ctl:?}");
    drop(ctl);
}

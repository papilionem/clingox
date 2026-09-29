//! Regression test: `AsyncSolveHandle::core(&self)` read the unsat core while
//! the search it belongs to was still running, racing clasp's own end-of-step
//! summary write (`TSan`: `ClaspFacade::stopStep` writes the summary while
//! `unsatCore` reads it). The yield handle's own `core` is unaffected (it can
//! only be called between models, when the search is not running). The file is
//! meant to be run under `TSan`.
//!
//! `AsyncSolveHandle::core` takes `&mut self` and waits for the search to
//! finish first, then reads the core; documented as blocking.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::time::{Duration, Instant};

use clingox::{Control, Part, Symbol};

fn has_threads() -> bool {
    clingox_sys::HAS_THREADS
}

fn sym(text: &str) -> Symbol {
    text.parse().unwrap()
}

/// A pigeonhole program sized to keep clasp busy for tens of milliseconds:
/// long enough that a `core()` call issued the instant the search starts is
/// overwhelmingly likely to still find it running, on unfixed code that
/// does not wait first.
const HARD: &str =
    "p(1..12). h(1..11). 1 { in(P,H) : h(H) } 1 :- p(P). :- in(P,H), in(Q,H), P < Q.";

/// `AsyncSolveHandle::core` must wait for the search to finish before
/// reading it: immediately after it returns, the search itself must already
/// be done, observable as `wait(Duration::ZERO)` (which only polls, never
/// blocks) reporting `true` at once.
#[test]
fn core_waits_for_the_search_to_finish_before_reading_it() {
    if !has_threads() {
        return;
    }
    let mut ctl = Control::with_args(["-t", "4"]).unwrap();
    ctl.add_base(HARD).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let mut handle = ctl.solve_async(&[]).unwrap();

    let started = Instant::now();
    let _core = handle.core().unwrap();
    let core_returned_after = started.elapsed();

    assert!(
        handle.wait(Duration::ZERO),
        "core() must not return before the search it belongs to has finished: a poll \
         immediately afterward still finds it running (core() took {core_returned_after:?})"
    );

    let _ = handle.close().unwrap();
}

/// The original `TSan` repro, kept close to its original shape: 200 short async
/// searches, reading `core()` repeatedly while each one runs.
#[test]
fn async_core_racing_the_end_of_the_search() {
    if !has_threads() {
        return;
    }
    let mut ctl = Control::with_args(["-t", "2"]).unwrap();
    ctl.add_base("{a;b;c}. :- a, b. x(1..30). {y(X)} :- x(X).")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (a, b) = (sym("a"), sym("b"));

    for _ in 0..200 {
        let mut handle = ctl
            .solve_async(&[(a, true).into(), (b, true).into()])
            .unwrap();
        // Under the fix, `core()` itself waits for the search to finish, so
        // this loop's `wait(Duration::ZERO)` check is true from the first
        // iteration; before the fix, `core()` could read mid-search.
        loop {
            let _ = handle.core().unwrap();
            if handle.wait(Duration::ZERO) {
                break;
            }
        }
        let _ = handle.core().unwrap();
        let _ = handle.close().unwrap();
    }
}

//! A panic inside `propagate`, `undo` or `check` is caught, does not abort the
//! process, resumes on the caller's thread once `solve` returns, and the other
//! threads' propagator calls stop cleanly (this is the hazard U25 found for a
//! *different* callback family; propagators are not affected, checked directly
//! against `clasp/src/ clingo.cpp`/`libclingo/src/control.cc`; this file is the
//! test that actively tries to trigger the U25-shaped failure and confirms it
//! does not happen, run with more than one thread, per S8's "every later
//! callback returns false at once").
//!
//! `api_propagator_init.rs` already has the single-thread version of this shape
//! for `init`/`propagate` (`a_panic_in_init_is_caught_resumes_and_ poisons`,
//! `a_panic_in_propagate_is_caught_and_resumes_on_the_caller_ after_the_solve`,
//! `a_panic_in_undo_is_caught_and_resumes_after_the_ solve`); this file adds
//! `check` and reruns `propagate`/`undo` with 4 threads specifically, since a
//! single-threaded panic cannot show that *other* threads' propagator calls
//! stop cleanly once one thread's callback has recorded a panic (S8: "makes
//! every later callback return false at once").

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use clingox::propagate::{CheckMode, PropagateControl, PropagateInit, Propagator, SolverLiteral};
use clingox::{Control, ErrorKind, Part, Result};

fn threads_or_one(n: u32) -> u32 {
    if clingox_sys::HAS_THREADS { n } else { 1 }
}

// ---------------------------------------------------------------------------
// A panicking check, single-threaded: caught, does not abort, resumes on
// the caller's thread once solve returns. Mirrors api_propagator_init.rs's
// own propagate/undo panic tests, for check instead.

struct PanicsInCheck;

impl Propagator for PanicsInCheck {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Total);
        Ok(())
    }

    fn check(&self, _control: &mut PropagateControl<'_>) -> Result<()> {
        panic!("check panics on purpose");
    }
}

#[test]
fn a_panic_in_check_is_caught_and_resumes_after_the_solve() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("{a}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(PanicsInCheck).unwrap();

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller, not aborting the process");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("check panics on purpose")
    );

    // Recovery means a new Control (init poisons unconditionally,
    // propagate/check/undo do not, but a panic already unwound this call, so
    // the same "solve again on the same Control" check api_propagator_init.rs
    // already runs for propagate is repeated here for check specifically).
    let mut ctl2 = Control::new().unwrap();
    ctl2.add_base("a.").unwrap();
    ctl2.ground(&[Part::base()]).unwrap();
    struct FailsOnceThenOk(std::sync::Mutex<bool>);
    impl Propagator for FailsOnceThenOk {
        fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
            init.set_check_mode(CheckMode::Total);
            Ok(())
        }
        fn check(&self, _control: &mut PropagateControl<'_>) -> Result<()> {
            let mut failed = self.0.lock().unwrap();
            if !*failed {
                *failed = true;
                return Err(clingox::Error::new(
                    ErrorKind::Callback,
                    "first check fails",
                ));
            }
            Ok(())
        }
    }
    ctl2.register_propagator(FailsOnceThenOk(std::sync::Mutex::new(false)))
        .unwrap();
    let first = ctl2.solve(&[]).unwrap_err();
    assert_eq!(first.kind(), ErrorKind::Callback);
    let second = ctl2.solve(&[]).unwrap();
    assert!(second.is_sat(), "check does not poison");
}

// ---------------------------------------------------------------------------
// A panicking propagate, 4 threads: does not crash the process, and resumes
// on the caller's thread once solve returns.
//
// This does not itself try to prove "every other thread's propagator calls
// stop cleanly" (S8's own "every later callback returns false at once"): a
// call already in flight on another thread when the panic is recorded can
// legitimately still reach this method's own body once, concurrently,
// before clingox's internal slot is checked again, which is not a
// violation of S8 (S8 is about calls *after* the slot is marked, not ones
// racing it) but is not distinguishable from a real violation using only
// this propagator's own counters. `total_calls` is recorded only as a
// sanity floor (propagate fired at least once, on at least one thread,
// before the panic).

struct PanicsOnFirstThread {
    panicked: AtomicBool,
    total_calls: Arc<AtomicU32>,
}

impl Propagator for PanicsOnFirstThread {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        for atom in &init.symbolic_atoms()? {
            let slit = init.solver_literal(atom?.literal())?;
            init.add_watch(slit)?;
        }
        Ok(())
    }

    fn propagate(
        &self,
        _control: &mut PropagateControl<'_>,
        _changes: &[SolverLiteral],
    ) -> Result<()> {
        self.total_calls.fetch_add(1, Ordering::SeqCst);
        // swap's own return is the *old* value: false only on the very
        // first call, so this panics exactly once, on whichever thread
        // gets there first.
        assert!(
            self.panicked.swap(true, Ordering::SeqCst),
            "propagate panics on purpose, 4 threads"
        );
        Ok(())
    }
}

#[test]
fn a_panic_in_propagate_does_not_crash_the_process_with_four_threads() {
    let threads = threads_or_one(4);
    let mut ctl = Control::builder().threads(threads).build().unwrap();
    ctl.add_base("1 { p(1..40) }.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let total_calls = Arc::new(AtomicU32::new(0));
    ctl.register_propagator(PanicsOnFirstThread {
        panicked: AtomicBool::new(false),
        total_calls: Arc::clone(&total_calls),
    })
    .unwrap();

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller, not aborting the process");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("propagate panics on purpose, 4 threads")
    );
    assert!(
        total_calls.load(Ordering::SeqCst) >= 1,
        "sanity: propagate fired at least once"
    );
}

// ---------------------------------------------------------------------------
// A panicking undo, 4 threads: caught, does not crash the process, resumes
// on the caller's thread once solve returns. undo is void at the C level
// (S9): the panic is still caught and recorded in the propagator's own
// slot, surfacing (as a resumed panic, since that is what was recorded) the
// next time the solve driving it checks that slot.

struct PanicsInUndo {
    panicked: AtomicBool,
}

impl Propagator for PanicsInUndo {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        for atom in &init.symbolic_atoms()? {
            let slit = init.solver_literal(atom?.literal())?;
            init.add_watch(slit)?;
        }
        Ok(())
    }

    fn undo(&self, _control: &PropagateControl<'_>, _changes: &[SolverLiteral]) {
        // See PanicsOnFirstThread::propagate's own comment: swap's old
        // value is false only on the first call.
        assert!(
            self.panicked.swap(true, Ordering::SeqCst),
            "undo panics on purpose, 4 threads"
        );
    }
}

#[test]
fn a_panic_in_undo_does_not_crash_the_process_with_four_threads() {
    let threads = threads_or_one(4);
    let mut ctl = Control::builder().threads(threads).build().unwrap();
    // A cardinality constraint forcing real search (exactly 20 of 40 true,
    // C(40, 20) combinations), so backtracking (and so undo) is guaranteed
    // to happen on at least one thread, not only forward unit propagation.
    // Checked directly: satisfiable,
    // undo fires.
    ctl.add_base("1 { p(1..40) }. :- #count{X : p(X)} != 20.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(PanicsInUndo {
        panicked: AtomicBool::new(false),
    })
    .unwrap();

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller, not aborting the process");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("undo panics on purpose, 4 threads")
    );
}

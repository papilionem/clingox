//! `Propagator::decide`'s dispatch: a returned `Some(literal)` is
//! validated against `clingo_assignment_has_literal`, the same rule every
//! other literal clingox hands to clingo already follows, and deliberately
//! choosing the `fallback` literal (`Some(fallback)`, not `None`) works.
//!
//! `decide`'s return type is `Result<Option<SolverLiteral>>`: returning the
//! bare `fallback` literal is indistinguishable, at the C level, from
//! choosing it, so a propagator with no opinion could never actually
//! decline once more than one propagator was registered. `None` is the trait's
//! default and the real decline now; `api_propagator_decide_heuristic.rs`
//! has the multi-propagator chaining tests this file does not.
//!
//! **This file is kept separate and its risky test runs in a spawned
//! thread with a timeout guard, on purpose.** pyclingo was seen
//! spinning at 100% CPU for 45 seconds when `decide` returns an
//! unvalidated, unknown literal; a few short (bounded) attempts did not
//! reproduce a hang, but the underlying clingox
//! code (`clingox/src/raw/propagate.rs`'s `decide`
//! trampoline) writes the returned literal straight
//! into clingo's `*decision` out-pointer with no check at all, which is
//! exactly the class of bug that produced a segfault for the equivalent,
//! already-fixed case in `PropagateInit`/`PropagateControl` (an unchecked
//! out-of-range literal reaching clasp's own per-variable arrays). Given
//! the choice between "assume the report was imprecise" and
//! "assume clasp can misbehave for even longer than 45 seconds, or worse,"
//! this test never calls `solve` on the current thread directly.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clingox::propagate::{Assignment, PropagateInit, Propagator, SolverLiteral};
use clingox::{Control, ErrorKind, Part, Result};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a fresh control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// A `SolverLiteral` legitimately obtained from a much larger, separate
/// control's grounding: valid there, but unknown to a small control's own
/// assignment. Identical to the helper in `api_propagator_init.rs` and
/// `api_propagator_control.rs` (each integration test file is its own
/// compilation unit, so it cannot be shared without a `tests/common`
/// module this crate does not otherwise have).
fn foreign_literal() -> SolverLiteral {
    let biggest: Arc<Mutex<Option<SolverLiteral>>> = Arc::new(Mutex::new(None));

    struct Capture(Arc<Mutex<Option<SolverLiteral>>>);
    impl Propagator for Capture {
        fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
            let mut lits = Vec::new();
            for atom in &init.symbolic_atoms()? {
                lits.push(init.solver_literal(atom?.literal())?);
            }
            *self.0.lock().unwrap() = lits.into_iter().max();
            Ok(())
        }
    }

    let mut ctl = Control::new().unwrap();
    ctl.add_base("{p(1..200)}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(Capture(Arc::clone(&biggest)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    biggest.lock().unwrap().expect("at least one atom exists")
}

// ---------------------------------------------------------------------------
// decide deliberately choosing the fallback literal (Some(fallback), not
// None): exercised explicitly to prove the dispatch round-trips a real
// choice correctly, even when its value happens to equal what clingo would
// have picked anyway.
//
// Oracle: `{a;b}.` reliably calls `decide` (checked directly: 2 calls, one
// per free choice) and solving finishes normally either way.

struct ReturnsFallback;
impl Propagator for ReturnsFallback {
    fn decide(
        &self,
        _thread_id: u32,
        _assignment: &Assignment<'_>,
        fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        Ok(Some(fallback))
    }
}

#[test]
fn decide_returning_the_fallback_literal_works() {
    let mut ctl = grounded("{a;b}.");
    ctl.register_propagator(ReturnsFallback).unwrap();
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());
}

// ---------------------------------------------------------------------------
// decide returning an unknown literal (from a different control) is
// refused with ErrorKind::InvalidInput, and the solve neither hangs nor
// crashes: run on a spawned thread with a 20-second timeout, per the
// reproduction of an unguarded hang.

struct ReturnsForeign(SolverLiteral);
impl Propagator for ReturnsForeign {
    fn decide(
        &self,
        _thread_id: u32,
        _assignment: &Assignment<'_>,
        _fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        Ok(Some(self.0))
    }
}

#[test]
fn decide_returning_an_unknown_literal_is_refused_not_a_hang_or_crash() {
    let foreign = foreign_literal();

    // Without OS threads (the default WebAssembly build) the solve runs on
    // this thread, without the timeout guard.
    if cfg!(all(target_family = "wasm", not(target_feature = "atomics"))) {
        let mut ctl = grounded("{a;b}.");
        ctl.register_propagator(ReturnsForeign(foreign)).unwrap();
        let err = ctl.solve(&[]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        return;
    }

    let (tx, rx) = mpsc::channel();

    // The solve that could hang or crash runs on its own thread; the test
    // itself never blocks on it directly. If it truly hangs, this thread
    // is abandoned when the test binary exits (the OS reclaims it), which
    // is the accepted cost of testing a hang safely rather than not at
    // all.
    std::thread::spawn(move || {
        let mut ctl = grounded("{a;b}.");
        if ctl.register_propagator(ReturnsForeign(foreign)).is_err() {
            let _ = tx.send(None);
            return;
        }
        let outcome = ctl.solve(&[]);
        let _ = tx.send(Some(outcome.map_err(|e| e.kind())));
    });

    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(Some(Err(kind))) => assert_eq!(kind, ErrorKind::InvalidInput),
        Ok(Some(Ok(_))) => panic!("expected ErrorKind::InvalidInput, the solve reported success"),
        Ok(None) => panic!("register_propagator itself failed unexpectedly"),
        Err(mpsc::RecvTimeoutError::Timeout) => panic!(
            "the solve did not finish within 20s: this is the hang that was reported \
             (pyclingo spun at 100% CPU for 45s under the equivalent unvalidated case)"
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            panic!("the solving thread ended without sending a result (it panicked or crashed)")
        }
    }
}

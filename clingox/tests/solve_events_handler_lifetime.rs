//! When a solve-event handler is dropped, and what a handler may borrow.
//!
//! The control keeps the handler for the search. A blocking search must drop
//! it before `solve_with_events` returns, so a borrow the handler holds ends
//! with the call. A yield handle can be leaked (`mem::forget` is safe,
//! DESIGN S4), which ends the handle's borrows while the search is still
//! open, so the yielding form takes only `'static` handlers; its handler is
//! dropped when the search closes, including a leaked search closed by the
//! next call on the control.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use clingox::{
    Control, InterruptHandle, Part, Result, SolveEventHandler, SolveOptions, SolveResult,
};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::with_args(["--models=0"]).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// Writes through its borrow when dropped.
struct TouchOnDrop<'a>(&'a mut i32);

impl SolveEventHandler for TouchOnDrop<'_> {}

impl Drop for TouchOnDrop<'_> {
    fn drop(&mut self) {
        *self.0 += 1;
    }
}

/// The original reproduction: with the handler kept past the call, the
/// write below was followed by a second write through the handler's stale
/// borrow when the control was dropped, leaving 101.
#[test]
fn a_borrowing_handler_is_dropped_before_solve_with_events_returns() {
    let mut ctl = grounded("{a;b}.");
    let mut n = 0_i32;
    let _ = ctl
        .solve_with_events(SolveOptions::new(), TouchOnDrop(&mut n))
        .unwrap();
    assert_eq!(n, 1, "the handler was dropped once, inside the call");
    n = 100;
    drop(ctl);
    assert_eq!(
        n, 100,
        "nothing wrote through the handler's borrow afterwards"
    );
}

/// Counts how often it is dropped.
struct CountDrops(Arc<AtomicUsize>);

impl SolveEventHandler for CountDrops {}

impl Drop for CountDrops {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_handler_is_dropped_before_solve_with_events_returns_an_error() {
    struct Fails(Arc<AtomicUsize>);
    impl SolveEventHandler for Fails {
        fn on_model(
            &mut self,
            _model: &mut clingox::ExtendableModel<'_>,
        ) -> Result<ControlFlow<()>> {
            Err(clingox::Error::callback(std::io::Error::other("stop")))
        }
    }
    impl Drop for Fails {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let mut ctl = grounded("{a;b}.");
    assert!(
        ctl.solve_with_events(SolveOptions::new(), Fails(Arc::clone(&drops)))
            .is_err()
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn a_yield_handler_is_dropped_when_the_handle_closes() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut ctl = grounded("{a;b}.");
    let mut handle = ctl
        .solve_yield_with_events(&[], CountDrops(Arc::clone(&drops)))
        .unwrap();
    while handle.next_model().unwrap().is_some() {}
    assert_eq!(drops.load(Ordering::SeqCst), 0, "the search still owns it");
    let _ = handle.close().unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn a_yield_handler_is_dropped_when_the_handle_is_dropped() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut ctl = grounded("{a;b}.");
    let mut handle = ctl
        .solve_yield_with_events(&[], CountDrops(Arc::clone(&drops)))
        .unwrap();
    assert!(handle.next_model().unwrap().is_some());
    drop(handle);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

/// A leaked handle leaves its search open (S4); the next call on the control
/// closes it, and that drops the handler.
#[test]
fn a_leaked_yield_handle_s_handler_is_dropped_by_the_next_call() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut ctl = grounded("{a;b}.");
    let mut handle = ctl
        .solve_yield_with_events(&[], CountDrops(Arc::clone(&drops)))
        .unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert!(ctl.solve(&[]).unwrap().is_exhausted());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

/// Contract negative control 1, second clause: the interrupt-phase step on
/// the finish event must run before the user's `on_finish`, so an interrupt
/// sent from `on_finish` finds the search finished and returns `false`
/// (`InterruptHandle::interrupt`'s documented contract), and never reaches
/// the next solve.
#[test]
fn an_interrupt_from_on_finish_finds_the_search_already_finished() {
    struct InterruptOnFinish {
        stop: InterruptHandle,
        delivered: Arc<AtomicBool>,
    }
    impl SolveEventHandler for InterruptOnFinish {
        fn on_finish(&mut self, _result: SolveResult) -> Result<ControlFlow<()>> {
            self.delivered
                .store(self.stop.interrupt(), Ordering::SeqCst);
            Ok(ControlFlow::Continue(()))
        }
    }

    let delivered = Arc::new(AtomicBool::new(true));
    let mut ctl = grounded("{a;b}.");
    let handler = InterruptOnFinish {
        stop: ctl.interrupt_handle(),
        delivered: Arc::clone(&delivered),
    };
    let mut handle = ctl.solve_yield_with_events(&[], handler).unwrap();
    while handle.next_model().unwrap().is_some() {}
    assert!(handle.close().unwrap().is_exhausted());
    assert!(
        !delivered.load(Ordering::SeqCst),
        "the search was already finished when on_finish ran"
    );
    assert!(!ctl.solve(&[]).unwrap().is_interrupted());
}

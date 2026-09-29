//! A [`SolveEventHandler`]'s `on_finish` failing or panicking during a yield
//! handle's close-triggered-by-drop: what happens to that failure afterward.
//!
//! `SolveHandle::drop` -> `Control::finish_search` -> `close_solve` ->
//! `close_active` delivers the finish event synchronously inside
//! `clingo_solve_handle_close` (clingo delivers it before the close call
//! returns, `docs/dev/DESIGN.md` S7). `close_active` then promotes the
//! handler's own recorded error or panic into control-level slots
//! (`ControlHandle::take_event_handler`) and drops the handler, but `Drop`
//! itself discards the result of `finish_search` (`clingox/src/solve.rs`):
//! nothing on this path can report an error out of a `Drop` impl.
//!
//! Those control-level slots are read only by the *next* events-aware call
//! (`Control::settle_events`/`get_and_close`, in `clingox/src/control.rs`) run
//! after that call's own operation, first-writer-wins (DESIGN S8). A plain call
//! (`solve`, `ground`, `add`) never reads them at all. The tests below pin what
//! that means for a later, unrelated call: the stale payload must not surface
//! where it does not belong, and must not silently block a genuinely new
//! failure from being reported.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};

use clingox::{
    Control, Error, ErrorKind, Part, Result, SolveEventHandler, SolveHandle, SolveOptions,
};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::with_args(["--models=0"]).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// Fails `on_finish` with a caller-chosen kind.
struct FailsOnFinish(ErrorKind);

impl SolveEventHandler for FailsOnFinish {
    fn on_finish(&mut self, _result: clingox::SolveResult) -> Result<ControlFlow<()>> {
        Err(Error::new(
            self.0,
            "the handler fails on purpose in on_finish",
        ))
    }
}

/// Panics in `on_finish`.
struct PanicsOnFinish;

impl SolveEventHandler for PanicsOnFinish {
    fn on_finish(&mut self, _result: clingox::SolveResult) -> Result<ControlFlow<()>> {
        panic!("on_finish panics on purpose")
    }
}

/// Does nothing: a control on an unrelated, later search.
struct Noop;

impl SolveEventHandler for Noop {}

/// Starts a yield-with-events search, takes one model (so the search is
/// genuinely mid-search, not already exhausted), then drops the handle
/// without closing it: `Drop` cancels and closes it, delivering the finish
/// event to `handler` along the way (mirrors
/// `clingox/tests/solve_events_handler_lifetime.rs::a_yield_handler_is_
/// dropped_when_the_handle_is_dropped`).
fn drop_mid_search<H: SolveEventHandler + Send + 'static>(ctl: &mut Control, handler: H) {
    let mut handle = ctl.solve_yield_with_events(&[], handler).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    drop(handle);
}

/// A later, unrelated plain `solve` must succeed normally: it never reads
/// the event-handler slots at all, so a stale entry left by a dropped
/// handle must not make it fail, and must not silently poison it either.
#[test]
fn a_stale_on_finish_error_from_a_dropped_handle_never_surfaces_in_a_later_plain_solve() {
    let mut ctl = grounded("{a;b}.");
    drop_mid_search(&mut ctl, FailsOnFinish(ErrorKind::InvalidInput));

    let result = ctl.solve(&[]);
    assert!(
        result.is_ok(),
        "a later plain solve must succeed, not resurface a dropped handle's stale error: got {result:?}"
    );
    assert!(
        !format!("{ctl:?}").contains("poisoned"),
        "a handler's own error never poisons the control, stale or not"
    );
}

/// A later, unrelated events-aware search (a fresh yield handle whose own
/// handler never fails) must report its own, successful result, not the
/// dropped handle's stale error.
#[test]
fn a_stale_on_finish_error_from_a_dropped_handle_does_not_surface_in_a_later_unrelated_search() {
    let mut ctl = grounded("{a;b}.");
    drop_mid_search(&mut ctl, FailsOnFinish(ErrorKind::InvalidInput));

    let mut handle = ctl.solve_yield_with_events(&[], Noop).unwrap();
    while handle.next_model().unwrap().is_some() {}
    let result = handle.close();
    assert!(
        result.is_ok_and(|r| r.is_exhausted()),
        "the later, unrelated search's own result must be reported"
    );
}

/// A later `solve_with_events` whose *own* handler also fails must report
/// that handler's own error, not the first, stale one (first-writer-wins
/// promotion must not let a stale entry suppress a new, genuine failure).
#[test]
fn a_stale_on_finish_error_never_suppresses_a_later_handlers_own_failure() {
    let mut ctl = grounded("{a;b}.");
    drop_mid_search(&mut ctl, FailsOnFinish(ErrorKind::InvalidInput));

    let err = ctl
        .solve_with_events(SolveOptions::new(), FailsOnFinish(ErrorKind::Conversion))
        .unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::Conversion,
        "the later handler's own error must be reported, not the dropped handle's stale one"
    );
}

/// As the error tests above, for a stale panic: it must not resume inside a
/// later, unrelated call's own success.
#[test]
fn a_stale_on_finish_panic_from_a_dropped_handle_does_not_resume_inside_a_later_unrelated_search() {
    let mut ctl = grounded("{a;b}.");
    drop_mid_search(&mut ctl, PanicsOnFinish);

    let mut handle = ctl.solve_yield_with_events(&[], Noop).unwrap();
    while handle.next_model().unwrap().is_some() {}
    let result = handle.close();
    assert!(
        result.is_ok_and(|r| r.is_exhausted()),
        "the later, unrelated search's own result must be reported, not a resumed stale panic"
    );
}

/// A panic in `on_finish` delivered during a drop-triggered close must not
/// itself escape `Drop`: nothing here may call `std::panic::resume_unwind`
/// from inside a destructor on this path (S9: a panic a void callback
/// cannot report is recorded, never propagated from the closing call
/// itself). Reaching the assertion below already proves `drop` did not
/// panic.
#[test]
fn a_panic_in_on_finish_during_drop_does_not_itself_panic() {
    let mut ctl = grounded("{a;b}.");
    drop_mid_search(&mut ctl, PanicsOnFinish);
    assert!(
        ctl.solve(&[]).is_ok(),
        "the stale panic must not resurface on a later plain solve either"
    );
}

/// Holds a yield handle and drops it, on purpose, from inside its own
/// `Drop`, so that drop runs while a different panic is already unwinding
/// through the same frame.
struct DropDuringUnwind<'c>(Option<SolveHandle<'c>>);

impl Drop for DropDuringUnwind<'_> {
    fn drop(&mut self) {
        drop(self.0.take());
    }
}

/// As above, but `handle` drops while a *different* panic is already
/// unwinding through the same frame. If `Drop` ever resumed `on_finish`'s
/// panic itself instead of only recording it, this would be a second panic
/// while one is already unwinding, which Rust cannot recover from by
/// unwinding again: the process aborts, and `catch_unwind` below never
/// returns at all.
#[test]
fn dropping_a_handle_with_a_panicking_on_finish_during_an_unrelated_unwind_does_not_abort() {
    let mut ctl = grounded("{a;b}.");
    let mut handle = ctl.solve_yield_with_events(&[], PanicsOnFinish).unwrap();
    assert!(handle.next_model().unwrap().is_some());

    let caught = catch_unwind(AssertUnwindSafe(|| {
        let _guard = DropDuringUnwind(Some(handle));
        panic!("an unrelated panic, already unwinding when `handle` drops");
    }));
    let payload = caught.expect_err("the unrelated panic reaches the caller");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"an unrelated panic, already unwinding when `handle` drops")
    );
    // Reaching here at all (rather than the process aborting) is the point
    // of this test.
    assert!(
        ctl.solve(&[]).is_ok(),
        "a later plain solve must still succeed after the double-drop scenario"
    );
}

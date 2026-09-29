//! Regression test: the first caller to observe a solve-event handler's stored
//! error took it out of its slot (`Slot::take`, first-writer-wins, S8); a later
//! call on the same search (another `next_model`, or `close`) found the slot
//! empty and fell through to clingo's own generic error instead. The handler's
//! error is reported on every later call on that search, not only the first,
//! while the handler itself is still never re-entered once it has failed (S8's
//! own guarantee, which this fix must not weaken).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each handler is defined next to the test that uses it"
)]

use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};

use clingox::{Control, Error, ErrorKind, ExtendableModel, Part, Result, SolveEventHandler};

/// Fails on every model it sees (so a bug that clears the reported error
/// cannot hide behind "the handler would have succeeded next time anyway"),
/// and records every call it actually receives.
#[derive(Clone, Default)]
struct FailsOnEveryModel(Arc<Mutex<Vec<u64>>>);
impl SolveEventHandler for FailsOnEveryModel {
    fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
        self.0.lock().unwrap().push(model.number());
        Err(Error::new(
            ErrorKind::Conversion,
            "every model fails on purpose",
        ))
    }
}

#[test]
fn a_failed_handlers_error_is_reported_on_every_later_call_on_the_same_search() {
    let mut ctl = Control::with_args(["0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let handler = FailsOnEveryModel::default();
    let mut handle = ctl.solve_yield_with_events(&[], handler.clone()).unwrap();

    let mut kinds = Vec::new();
    for _ in 0..4 {
        let r = handle.next_model();
        kinds.push(r.err().map(|e| e.kind()));
    }
    let closed = handle.close();

    let calls = handler.0.lock().unwrap().clone();
    assert_eq!(
        calls.len(),
        1,
        "the handler must never be re-entered once it has failed (S8): {calls:?}"
    );

    for (i, kind) in kinds.iter().enumerate() {
        assert_eq!(
            *kind,
            Some(ErrorKind::Conversion),
            "call {i} on the same search must still report the handler's own error kind, not \
             clingo's generic one: {kinds:?}"
        );
    }
    assert_eq!(
        closed.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Conversion),
        "close() must report it too: {closed:?}"
    );

    // The control itself is not poisoned by a handler's own error (unaffected
    // by this fix): it is still usable for an unrelated, later search.
    assert!(!format!("{ctl:?}").contains("poisoned"), "{ctl:?}");
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

/// The blocking entry point (`solve_with_events`) reaches the same
/// contract: the one call it makes must report the handler's own error, not
/// a later, unrelated call's error going stale in the opposite direction.
/// This pins that a later, *different* search on the same control is
/// unaffected once the failed one has been closed.
#[test]
fn a_later_different_search_is_not_affected_by_an_earlier_ones_reported_error() {
    let mut ctl = Control::with_args(["0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let first = ctl.solve_with_events(clingox::SolveOptions::new(), FailsOnEveryModel::default());
    assert_eq!(
        first.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Conversion)
    );

    // A second, independent search, with a handler that never fails.
    struct NeverFails;
    impl SolveEventHandler for NeverFails {}
    let second = ctl.solve_with_events(clingox::SolveOptions::new(), NeverFails);
    assert!(second.is_ok(), "{second:?}");
}

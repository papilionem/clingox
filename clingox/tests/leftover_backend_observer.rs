//! Regression test: a registered
//! observer's own panic or error, raised while `finish_search` closed a
//! backend a panic had left open (through a `with_backend` closure that
//! itself panicked after adding a rule), was not checked where
//! `finish_search` ran: it surfaced as a generic `ErrorKind::Unknown` at
//! that point (losing the observer's own error), and the panic itself
//! resumed later, on the next unrelated call that happened to check the
//! observer slots, rather than at the call that actually closed the
//! backend. `finish_search` collects the observer slots right after
//! closing a leftover backend or search, with the same poisoning and panic
//! resumption `with_backend` and `ground` already give an observer failure
//! during grounding.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::panic::{AssertUnwindSafe, catch_unwind};

use clingox::backend::{Atom, Head};
use clingox::observer::GroundProgramObserver;
use clingox::{Control, Error, ErrorKind, Result, Symbol};

fn sym(text: &str) -> Symbol {
    text.parse().unwrap()
}

fn poisoned(ctl: &Control) -> bool {
    format!("{ctl:?}").contains("poisoned")
}

struct PanicsOnOutputAtom;
impl GroundProgramObserver for PanicsOnOutputAtom {
    fn output_atom(&mut self, _symbol: Symbol, _atom: Option<Atom>) -> Result<()> {
        panic!("observer panics in output_atom")
    }
}

struct FailsOnOutputAtom;
impl GroundProgramObserver for FailsOnOutputAtom {
    fn output_atom(&mut self, _symbol: Symbol, _atom: Option<Atom>) -> Result<()> {
        Err(Error::new(
            ErrorKind::Conversion,
            "observer fails in output_atom",
        ))
    }
}

/// Leaves a backend open with a fact whose `output_atom` is delayed to the
/// backend's own close (`clingo_backend_end`), by panicking out of
/// `with_backend`'s closure after adding the rule but before the backend
/// closes normally.
fn leave_backend_open(ctl: &mut Control) {
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_backend(|b| -> Result<()> {
            let a = b.add_atom(Some(sym("a")))?;
            b.add_rule(Head::Normal(&[a]), &[])?;
            panic!("the user's closure panics")
        })
    }));
    assert!(
        unwound.is_err(),
        "the setup itself must panic, or this test proves nothing"
    );
}

/// An observer panic while the leftover backend closes must resume exactly
/// at the call that closes it (here, `symbolic_atoms`, an `&self` entry
/// point that still has to finish any leftover search or backend first),
/// not at a later, unrelated call.
#[test]
fn observer_panic_at_a_leftover_backend_close_resumes_at_the_close_not_later() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(PanicsOnOutputAtom, false).unwrap();
    leave_backend_open(&mut ctl);

    let closing = catch_unwind(AssertUnwindSafe(|| ctl.symbolic_atoms().map(|_| ())));
    assert!(
        closing.is_err(),
        "the observer's panic must resume on the call that closes the leftover backend"
    );

    let later = catch_unwind(AssertUnwindSafe(|| ctl.solve(&[])));
    assert!(
        later.is_ok(),
        "a later, unrelated call must not resume a stale panic that already resumed above: \
         {later:?}"
    );
}

/// The same shape for `ground`, another entry point that closes a leftover
/// backend before doing its own work.
#[test]
fn observer_panic_at_a_leftover_backend_close_via_ground_resumes_at_the_close_not_later() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(PanicsOnOutputAtom, false).unwrap();
    leave_backend_open(&mut ctl);

    let closing = catch_unwind(AssertUnwindSafe(|| ctl.ground(&[clingox::Part::base()])));
    assert!(closing.is_err(), "the panic must resume here, at ground");

    let later = catch_unwind(AssertUnwindSafe(|| ctl.solve(&[])));
    assert!(later.is_ok(), "{later:?}");
}

/// An observer *error* (not a panic) at the leftover backend's close must
/// be reported as itself, at the call that closes it, not turned into a
/// generic `Unknown`.
#[test]
fn observer_error_at_a_leftover_backend_close_is_reported_as_itself_at_the_close() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(FailsOnOutputAtom, false).unwrap();
    leave_backend_open(&mut ctl);

    let closing = ctl.statistics().map(|_| ());
    assert_eq!(
        closing.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Conversion),
        "the observer's own error kind must survive, not become Unknown: {closing:?}"
    );
    assert!(
        poisoned(&ctl),
        "an observer error poisons the control (S3): {ctl:?}"
    );

    // The control is poisoned by this error alone: a later call reports
    // Poisoned, not a second, independent report of the same observer
    // failure.
    let later = ctl.solve(&[]);
    assert_eq!(
        later.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Poisoned)
    );
}

/// As above, reached through `ground` instead of an `&self` entry point.
#[test]
fn observer_error_at_a_leftover_backend_close_via_ground_is_reported_as_itself() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(FailsOnOutputAtom, false).unwrap();
    leave_backend_open(&mut ctl);

    let closing = ctl.ground(&[clingox::Part::base()]);
    assert_eq!(
        closing.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Conversion),
        "{closing:?}"
    );
    assert!(poisoned(&ctl));
}

/// `with_backend` itself, opening a fresh backend while a panic is owed
/// from the previous one's close, must be the call that resumes it.
#[test]
fn observer_panic_at_a_leftover_backend_close_via_with_backend_resumes_there() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(PanicsOnOutputAtom, false).unwrap();
    leave_backend_open(&mut ctl);

    let closing = catch_unwind(AssertUnwindSafe(|| ctl.with_backend(|_| Ok(()))));
    assert!(
        closing.is_err(),
        "the panic must resume at this with_backend call"
    );
}

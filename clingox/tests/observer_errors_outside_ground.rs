//! A registered
//! [`GroundProgramObserver`](clingox::observer::GroundProgramObserver) can fail
//! or panic on a callback that clingo makes outside `ground`/ `ground_with`: a
//! backend directive reaches it synchronously (`Control::with_backend`), a fact
//! added through the backend reaches it only when the backend closes
//! (`clingo_backend_end`, delayed output), and `end_step` reaches it when a
//! solve starts, not when grounding finishes.
//!
//! Only `Control::ground`/`ground_with` ever read an observer's recorded error
//! or panic (`resolve_ground`, the sole reader of `take_observer_error`/
//! `take_observer_panic`, `clingox/src/raw/control.rs`). Every other entry
//! point that can still trigger an observer callback -- `with_backend` and
//! `solve` among them -- never looks at those slots, so the observer's own
//! error is lost (clingo's own generic `ErrorKind::Unknown`, "an observer
//! callback returned an error", is reported instead) and a caught panic is
//! recorded but never resumed: it sits in the observer's own slot until the
//! control is dropped. The poisoning behaviour this file's assertions depend on
//! is described in DESIGN S3.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::panic::{AssertUnwindSafe, catch_unwind};

use clingox::backend::{Atom, Head};
use clingox::observer::GroundProgramObserver;
use clingox::{Control, Error, ErrorKind, Part, ProgramLiteral, Result, Symbol};

fn sym(text: &str) -> Symbol {
    text.parse().expect("the term parses")
}

// ---------------------------------------------------------------------------
// Finding A: an observer failing or panicking during `with_backend`.

/// Fails on `rule`, with a caller-chosen kind: `add_rule` calls it
/// synchronously, from inside `with_backend`'s own closure
/// (`libclingo/src/control.cc:1236`'s `Observer::rule` -> `outputRule`).
struct FailsOnRule(ErrorKind);

impl GroundProgramObserver for FailsOnRule {
    fn rule(&mut self, _choice: bool, _head: &[Atom], _body: &[ProgramLiteral]) -> Result<()> {
        Err(Error::new(self.0, "the observer fails on purpose in rule"))
    }
}

/// As [`FailsOnRule`], but panics.
struct PanicsOnRule;

impl GroundProgramObserver for PanicsOnRule {
    fn rule(&mut self, _choice: bool, _head: &[Atom], _body: &[ProgramLiteral]) -> Result<()> {
        panic!("the observer panics on purpose in rule")
    }
}

/// Fails on `output_atom`, with a caller-chosen kind. A fact added through
/// the backend is only reported to the observer once the backend session
/// finishes (`ClingoControl::endAddBackend`, `clingocontrol.cc:893`; see
/// `is_fact` in `clingox/tests/api_backend.rs`, checked the same way against
/// pyclingo 5.8.2), so this observer's failure surfaces only when
/// `with_backend`'s own closure has already returned `Ok`, during the
/// implicit `clingo_backend_end` that follows it.
struct FailsOnOutputAtom(ErrorKind);

impl GroundProgramObserver for FailsOnOutputAtom {
    fn output_atom(&mut self, _symbol: Symbol, _atom: Option<Atom>) -> Result<()> {
        Err(Error::new(
            self.0,
            "the observer fails on purpose in output_atom",
        ))
    }
}

/// As [`FailsOnOutputAtom`], but panics.
struct PanicsOnOutputAtom;

impl GroundProgramObserver for PanicsOnOutputAtom {
    fn output_atom(&mut self, _symbol: Symbol, _atom: Option<Atom>) -> Result<()> {
        panic!("the observer panics on purpose in output_atom")
    }
}

fn add_fact_through_backend(backend: &mut clingox::backend::Backend<'_>) -> Result<()> {
    let a = backend.add_atom(Some(sym("a")))?;
    backend.add_rule(Head::Normal(&[a]), &[])
}

/// A directive during the closure (`add_rule`) fails: `with_backend` must
/// return the observer's own error unchanged, the same kind `ground` would
/// (`clingox/tests/api_observer.rs::an_observer_error_keeps_its_own_kind_
/// whatever_it_is`), not `ErrorKind::Unknown`.
///
/// `InvalidInput` is chosen because it is not one of the kinds `Control::note`
/// poisons "by kind" (DESIGN S3): if this passes only because `Unknown`
/// happens to poison anyway, a kind outside that list still tells the two
/// code paths apart.
#[test]
fn with_backend_returns_the_observers_own_error_unchanged_for_a_directive_in_the_closure() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(FailsOnRule(ErrorKind::InvalidInput), false)
        .unwrap();
    let err = ctl.with_backend(add_fact_through_backend).unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::InvalidInput,
        "the observer's own error kind must pass through `with_backend` unchanged"
    );
    assert!(
        format!("{ctl:?}").contains("poisoned"),
        "an observer error during with_backend must poison the control, mirroring ground's \
         unconditional poisoning of a callback error"
    );
}

/// As above, for a directive delayed to the backend's own close.
#[test]
fn with_backend_returns_the_observers_own_error_unchanged_for_a_directive_at_the_backends_end() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(FailsOnOutputAtom(ErrorKind::Conversion), false)
        .unwrap();
    let err = ctl.with_backend(add_fact_through_backend).unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::Conversion,
        "output_atom's failure at clingo_backend_end must surface as the observer's own error"
    );
    assert!(format!("{ctl:?}").contains("poisoned"));
}

/// A panic in an observer during a directive made inside the closure must
/// resume on the caller's thread once `with_backend` returns, exactly as it
/// does for `ground` (`clingox/tests/api_observer.rs::a_panic_in_an_
/// observer_callback_resumes_on_the_caller_and_poisons`).
#[test]
fn a_panic_in_an_observer_during_with_backend_resumes_on_the_caller_and_poisons() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(PanicsOnRule, false).unwrap();

    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_backend(add_fact_through_backend)
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"the observer panics on purpose in rule")
    );
    assert!(format!("{ctl:?}").contains("poisoned"));
}

/// As above, for a panic delayed to the backend's own close.
#[test]
fn a_panic_in_an_observer_at_the_backends_end_resumes_on_the_caller_and_poisons() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(PanicsOnOutputAtom, false).unwrap();

    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_backend(add_fact_through_backend)
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"the observer panics on purpose in output_atom")
    );
    assert!(format!("{ctl:?}").contains("poisoned"));
}

// ---------------------------------------------------------------------------
// Finding B: `end_step` fails or panics when a solve starts.
//
// `clingox/tests/api_observer.rs::end_step_fires_when_solving_starts_not_
// when_grounding_finishes` already pins that `ground` alone never calls
// `end_step`. pyclingo 5.8.2 confirms that an `end_step` callback
// which raises does so during `solve`, not during `ground`: the exception
// surfaces from the `solve()` call, not from grounding.

struct FailsOnEndStep(ErrorKind);

impl GroundProgramObserver for FailsOnEndStep {
    fn end_step(&mut self) -> Result<()> {
        Err(Error::new(
            self.0,
            "the observer fails on purpose in end_step",
        ))
    }
}

struct PanicsOnEndStep;

impl GroundProgramObserver for PanicsOnEndStep {
    fn end_step(&mut self) -> Result<()> {
        panic!("the observer panics on purpose in end_step")
    }
}

#[test]
fn an_end_step_error_is_reported_by_solve_not_ground_and_poisons() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(FailsOnEndStep(ErrorKind::InvalidInput), false)
        .unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()])
        .expect("end_step has not fired yet: grounding alone must still succeed");

    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::InvalidInput,
        "the observer's own error kind must pass through `solve` unchanged, as it does for ground"
    );
    assert!(format!("{ctl:?}").contains("poisoned"));
}

#[test]
fn a_panic_in_end_step_resumes_on_the_caller_of_solve_and_poisons() {
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(PanicsOnEndStep, false).unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()])
        .expect("end_step has not fired yet: grounding alone must still succeed");

    let caught = catch_unwind(AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller of solve, not of ground");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"the observer panics on purpose in end_step")
    );
    assert!(format!("{ctl:?}").contains("poisoned"));
}

//! Error and panic handling of the propagator callbacks: which failures poison
//! the control, and when the post-`Stop` guard refuses a call.
//!
//! **Poisoning.** `propagate`/`check`/`undo`/`decide` never poison the control,
//! whatever an error's kind or a panic; `init` keeps poisoning unconditionally.
//! The "does not poison" tests in `api_propagator_init.rs`
//! (`a_failed_propagate_does_not_poison_the_control`,
//! `::a_failed_check_does_not_poison_the_control`) use
//! `Error::callback`/`ErrorKind::Callback`, which is never on `Control::note`'s
//! by-kind poisoning table, so none of them can catch a divergence for the
//! other kinds: `raw::propagate::PropagatorSlots::take_error` clears an error's
//! `Poison::ByKind` default so that a `propagate`/`check`/`decide` error of
//! kind `Logic`, `BadAlloc` or `Unknown` does not reach `Control::note` with
//! `Poison::ByKind` (which would poison for exactly those three kinds).
//! Likewise `Control::settle_propagators` must not poison when it resumes a
//! recorded panic from `propagate`/`check`/`undo`/`decide`; `init`'s own panic
//! is the only one that poisons. `init`'s own poisoning (by error or panic) is
//! covered by
//! `api_propagator_init.rs::a_panic_in_init_is_caught_resumes_and_poisons` and
//! `::an_error_from_init_keeps_its_own_kind_and_poisons`; not repeated here.
//!
//! **The post-`Stop` guard.** After a `Flow::Stop` from any
//! `PropagateInit`/`PropagateControl` method, every further call that would
//! reach clingo is refused with `ErrorKind::InvalidInput`. That covers:
//!
//! - `PropagateInit::add_watch_to_thread`, `::remove_watch` and `::
//!   remove_watch_from_thread`;
//! - `PropagateControl::has_watch`, which returns `false` unconditionally after
//!   `Stop` instead of reaching clingo and reporting its own answer;
//! - `Assignment`/`Trail` (returned by `PropagateInit::assignment`/
//!   `PropagateControl::assignment`), which carry a reference to their owning
//!   object's `stopped` flag, so their fallible queries (`level`, `is_fixed`,
//!   `is_true`, `is_false`, `truth_value`, `at`, `decision`, and `Trail`'s
//!   `size`/`begin`/`end`/`at`) refuse after `Stop`. The five infallible
//!   getters (`decision_level`, `root_level`, `has_conflict`, `size`,
//!   `is_total`) and `has_literal` are exempt: every one of them is a
//!   direct-value C function (never the header's "whether the call was
//!   successful" shape the fallible queries above share), and the header's own
//!   "@attention no further calls ... on the assignment" note is attached only
//!   to `add_clause`/`add_weight_constraint`/`propagate`'s own doc blocks,
//!   never to the "Assignment Functions" section itself.
//!
//! **`Assignment::size`.** It counts the solver's whole variable space, not the
//! number of currently assigned literals; the fixture that shows this is pinned
//! here directly against the oracle.
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use clingox::propagate::{
    Assignment, CheckMode, ClauseType, Flow, PropagateControl, PropagateInit, Propagator,
    SolverLiteral, WeightConstraintKind,
};
use clingox::{Control, Error, ErrorKind, Part, Result, Signature};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a fresh control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn program_literal(init: &PropagateInit<'_>, name: &str) -> Result<clingox::ProgramLiteral> {
    let sig = Signature::new(name, 0)?;
    Ok(init
        .symbolic_atoms()?
        .by_signature(sig)
        .next()
        .unwrap_or_else(|| panic!("{name} is an atom"))?
        .literal())
}

// ============================================================================
// Propagate/check/undo/decide never poison,
// whatever an error's kind or a panic.
// ============================================================================

/// Fails with `kind` on the first call, succeeds afterward: a later solve on
/// the *same* control shows whether the earlier failure poisoned it.
fn fail_once(calls: &Mutex<u32>, kind: ErrorKind) -> Result<()> {
    let mut calls = calls.lock().unwrap();
    *calls += 1;
    if *calls == 1 {
        Err(Error::new(kind, "fails once on purpose"))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// propagate

struct FailsPropagateOnceWithKind {
    kind: ErrorKind,
    calls: Mutex<u32>,
}
impl Propagator for FailsPropagateOnceWithKind {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        _control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        fail_once(&self.calls, self.kind)
    }
}

fn assert_propagate_error_does_not_poison(kind: ErrorKind) {
    let mut ctl = grounded("1 { a; b } 1.");
    ctl.register_propagator(FailsPropagateOnceWithKind {
        kind,
        calls: Mutex::new(0),
    })
    .unwrap();

    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), kind);

    let result = ctl.solve(&[]);
    assert!(
        result.is_ok_and(|r| r.is_sat()),
        "a {kind:?} error from propagate must not poison the control \
        ; Control::note currently poisons for Logic/BadAlloc/ \
         Unknown, and propagate's own error keeps Poison::ByKind (only \
         Origin::Init's error is marked non-default)"
    );
}

#[test]
fn a_failed_propagate_of_kind_logic_does_not_poison() {
    assert_propagate_error_does_not_poison(ErrorKind::Logic);
}

#[test]
fn a_failed_propagate_of_kind_unknown_does_not_poison() {
    assert_propagate_error_does_not_poison(ErrorKind::Unknown);
}

#[test]
fn a_failed_propagate_of_kind_bad_alloc_does_not_poison() {
    assert_propagate_error_does_not_poison(ErrorKind::BadAlloc);
}

// ---------------------------------------------------------------------------
// check

struct FailsCheckOnceWithKind {
    kind: ErrorKind,
    calls: Mutex<u32>,
}
impl Propagator for FailsCheckOnceWithKind {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Total);
        Ok(())
    }

    fn check(&self, _control: &mut PropagateControl<'_>) -> Result<()> {
        fail_once(&self.calls, self.kind)
    }
}

fn assert_check_error_does_not_poison(kind: ErrorKind) {
    let mut ctl = grounded("a.");
    ctl.register_propagator(FailsCheckOnceWithKind {
        kind,
        calls: Mutex::new(0),
    })
    .unwrap();

    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), kind);

    let result = ctl.solve(&[]);
    assert!(
        result.is_ok_and(|r| r.is_sat()),
        "a {kind:?} error from check must not poison the control"
    );
}

#[test]
fn a_failed_check_of_kind_logic_does_not_poison() {
    assert_check_error_does_not_poison(ErrorKind::Logic);
}

#[test]
fn a_failed_check_of_kind_unknown_does_not_poison() {
    assert_check_error_does_not_poison(ErrorKind::Unknown);
}

#[test]
fn a_failed_check_of_kind_bad_alloc_does_not_poison() {
    assert_check_error_does_not_poison(ErrorKind::BadAlloc);
}

// ---------------------------------------------------------------------------
// decide

fn fail_once_decide(calls: &Mutex<u32>, kind: ErrorKind) -> Result<Option<SolverLiteral>> {
    let mut calls = calls.lock().unwrap();
    *calls += 1;
    if *calls == 1 {
        Err(Error::new(kind, "fails once on purpose"))
    } else {
        Ok(None)
    }
}

struct FailsDecideOnceWithKind {
    kind: ErrorKind,
    calls: Mutex<u32>,
}
impl Propagator for FailsDecideOnceWithKind {
    fn decide(
        &self,
        _thread_id: u32,
        _assignment: &Assignment<'_>,
        _fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        fail_once_decide(&self.calls, self.kind)
    }
}

fn assert_decide_error_does_not_poison(kind: ErrorKind) {
    let mut ctl = grounded("{a; b}.");
    ctl.register_propagator(FailsDecideOnceWithKind {
        kind,
        calls: Mutex::new(0),
    })
    .unwrap();

    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), kind);

    let result = ctl.solve(&[]);
    assert!(
        result.is_ok_and(|r| r.is_sat()),
        "a {kind:?} error from decide must not poison the control"
    );
}

#[test]
fn a_failed_decide_of_kind_logic_does_not_poison() {
    assert_decide_error_does_not_poison(ErrorKind::Logic);
}

#[test]
fn a_failed_decide_of_kind_unknown_does_not_poison() {
    assert_decide_error_does_not_poison(ErrorKind::Unknown);
}

#[test]
fn a_failed_decide_of_kind_bad_alloc_does_not_poison() {
    assert_decide_error_does_not_poison(ErrorKind::BadAlloc);
}

// ---------------------------------------------------------------------------
// Panics: propagate, check, undo, decide -- none of them may poison either,
// even though each one still resumes on the caller once the driving call
// returns (`propagator_panics.rs`/`api_propagator_init.rs` already pin the
// "resumes, does not crash" half; this file adds the "does not poison"
// half, on the *same* control, which none of the existing tests checks).

struct PanicsOnceInPropagate {
    panicked: AtomicBool,
}
impl Propagator for PanicsOnceInPropagate {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        _control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        assert!(
            self.panicked.swap(true, Ordering::SeqCst),
            "propagate panics once on purpose"
        );
        Ok(())
    }
}

#[test]
fn a_panic_in_propagate_does_not_poison_the_control() {
    let mut ctl = grounded("1 { a; b } 1.");
    ctl.register_propagator(PanicsOnceInPropagate {
        panicked: AtomicBool::new(false),
    })
    .unwrap();

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("propagate panics once on purpose")
    );

    let result = ctl.solve(&[]);
    assert!(
        result.is_ok_and(|r| r.is_sat()),
        "a panic in propagate must not poison the control either (decision \
         1); Control::settle_propagators currently calls poison_with before \
         every resumed panic, regardless of which callback recorded it"
    );
}

struct PanicsOnceInCheck {
    panicked: AtomicBool,
}
impl Propagator for PanicsOnceInCheck {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Total);
        Ok(())
    }

    fn check(&self, _control: &mut PropagateControl<'_>) -> Result<()> {
        assert!(
            self.panicked.swap(true, Ordering::SeqCst),
            "check panics once on purpose"
        );
        Ok(())
    }
}

#[test]
fn a_panic_in_check_does_not_poison_the_control() {
    let mut ctl = grounded("a.");
    ctl.register_propagator(PanicsOnceInCheck {
        panicked: AtomicBool::new(false),
    })
    .unwrap();

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("check panics once on purpose")
    );

    let result = ctl.solve(&[]);
    assert!(
        result.is_ok_and(|r| r.is_sat()),
        "a panic in check must not poison the control either"
    );
}

struct PanicsOnceInUndo {
    panicked: AtomicBool,
}
impl Propagator for PanicsOnceInUndo {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn undo(&self, _control: &PropagateControl<'_>, _changes: &[SolverLiteral]) {
        assert!(
            self.panicked.swap(true, Ordering::SeqCst),
            "undo panics once on purpose"
        );
    }
}

#[test]
fn a_panic_in_undo_does_not_poison_the_control() {
    let mut ctl = grounded("1 { a; b } 1.");
    ctl.configuration().set("solve.models", "0").unwrap();
    ctl.register_propagator(PanicsOnceInUndo {
        panicked: AtomicBool::new(false),
    })
    .unwrap();

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctl.for_each_model(&[], |_| Ok(std::ops::ControlFlow::Continue(())))
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("undo panics once on purpose")
    );

    let result = ctl.solve(&[]);
    assert!(
        result.is_ok_and(|r| r.is_sat()),
        "a panic in undo must not poison the control either"
    );
}

struct PanicsOnceInDecide {
    panicked: AtomicBool,
}
impl Propagator for PanicsOnceInDecide {
    fn decide(
        &self,
        _thread_id: u32,
        _assignment: &Assignment<'_>,
        fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        assert!(
            self.panicked.swap(true, Ordering::SeqCst),
            "decide panics once on purpose"
        );
        Ok(Some(fallback))
    }
}

#[test]
fn a_panic_in_decide_does_not_poison_the_control() {
    let mut ctl = grounded("{a; b}.");
    ctl.register_propagator(PanicsOnceInDecide {
        panicked: AtomicBool::new(false),
    })
    .unwrap();

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("decide panics once on purpose")
    );

    let result = ctl.solve(&[]);
    assert!(
        result.is_ok_and(|r| r.is_sat()),
        "a panic in decide must not poison the control either"
    );
}

// ============================================================================
// The post-Stop guard.
// ============================================================================

// ---------------------------------------------------------------------------
// PropagateInit methods missing the guard entirely.

struct RejectsCallsMissingTheStopGuardOnInit;
impl Propagator for RejectsCallsMissingTheStopGuardOnInit {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let lit = init.add_literal(true)?;
        let first = init.add_clause(&[lit])?;
        assert_eq!(first, Flow::Continue, "sanity");
        let stop = init.add_clause(&[-lit])?;
        assert_eq!(
            stop,
            Flow::Stop,
            "sanity: a direct contradiction forces Stop"
        );

        let err = init.add_watch_to_thread(lit, 0).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "add_watch_to_thread (it must be under the post-Stop guard like every \
             other fallible init method)"
        );
        let err = init.remove_watch(lit).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "remove_watch");
        let err = init.remove_watch_from_thread(lit, 0).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "remove_watch_from_thread"
        );
        Ok(())
    }
}

#[test]
fn add_watch_to_thread_and_remove_watch_are_refused_after_init_stop() {
    let mut ctl = grounded("a.");
    ctl.register_propagator(RejectsCallsMissingTheStopGuardOnInit)
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
}

// ---------------------------------------------------------------------------
// The remaining fallible PropagateInit methods after a Stop.
// One test per method, so removing
// one guard fails exactly one test.

/// Forces `Stop` on `init`, then runs `call` with a literal that is valid
/// otherwise (so only the stop guard can refuse it) and expects
/// `InvalidInput`.
struct CallsAfterInitStop {
    name: &'static str,
    call: fn(&mut PropagateInit<'_>, SolverLiteral, clingox::ProgramLiteral) -> Result<()>,
}
impl Propagator for CallsAfterInitStop {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let lit = init.add_literal(true)?;
        assert_eq!(init.add_clause(&[lit])?, Flow::Continue, "sanity");
        assert_eq!(
            init.add_clause(&[-lit])?,
            Flow::Stop,
            "sanity: a direct contradiction forces Stop"
        );
        let err = (self.call)(init, lit, plit).expect_err(self.name);
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "{} must be refused after Stop",
            self.name
        );
        Ok(())
    }
}

fn assert_refused_after_init_stop(
    name: &'static str,
    call: fn(&mut PropagateInit<'_>, SolverLiteral, clingox::ProgramLiteral) -> Result<()>,
) {
    let mut ctl = grounded("a.");
    ctl.register_propagator(CallsAfterInitStop { name, call })
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
}

#[test]
fn add_watch_is_refused_after_init_stop() {
    assert_refused_after_init_stop("add_watch", |init, lit, _| init.add_watch(lit));
}

#[test]
fn freeze_literal_is_refused_after_init_stop() {
    assert_refused_after_init_stop("freeze_literal", |init, lit, _| init.freeze_literal(lit));
}

#[test]
fn add_literal_is_refused_after_init_stop() {
    assert_refused_after_init_stop("add_literal", |init, _, _| {
        init.add_literal(false).map(drop)
    });
}

#[test]
fn add_weight_constraint_is_refused_after_init_stop() {
    assert_refused_after_init_stop("add_weight_constraint", |init, lit, _| {
        init.add_weight_constraint(
            lit,
            &[(lit, 1)],
            1,
            WeightConstraintKind::Equivalence,
            false,
        )
        .map(drop)
    });
}

#[test]
fn add_minimize_is_refused_after_init_stop() {
    assert_refused_after_init_stop("add_minimize", |init, lit, _| init.add_minimize(lit, 1, 0));
}

#[test]
fn solver_literal_is_refused_after_init_stop() {
    assert_refused_after_init_stop("solver_literal", |init, _, plit| {
        init.solver_literal(plit).map(drop)
    });
}

#[test]
fn symbolic_atoms_is_refused_after_init_stop() {
    assert_refused_after_init_stop("symbolic_atoms", |init, _, _| {
        init.symbolic_atoms().map(drop)
    });
}

#[test]
fn theory_atoms_is_refused_after_init_stop() {
    assert_refused_after_init_stop("theory_atoms", |init, _, _| init.theory_atoms().map(drop));
}

// ---------------------------------------------------------------------------
// Assignment/Trail queries through a stopped PropagateInit.

struct QueriesAssignmentAfterInitStop;
impl Propagator for QueriesAssignmentAfterInitStop {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let lit = init.add_literal(true)?;
        let _ = init.add_clause(&[lit])?;
        let stop = init.add_clause(&[-lit])?;
        assert_eq!(stop, Flow::Stop, "sanity: forces Stop");

        // `assignment()` itself stays callable (it is infallible).
        let a = init.assignment();
        assert!(
            a.size() >= 1,
            "sanity: the added literal is in the assignment"
        );
        assert!(
            a.has_literal(lit),
            "sanity: has_literal is exempt and correct"
        );

        assert_eq!(
            a.level(lit).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "level"
        );
        assert_eq!(
            a.is_fixed(lit).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "is_fixed"
        );
        assert_eq!(
            a.is_true(lit).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "is_true"
        );
        assert_eq!(
            a.is_false(lit).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "is_false"
        );
        assert_eq!(
            a.truth_value(lit).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "truth_value"
        );
        assert_eq!(a.at(0).unwrap_err().kind(), ErrorKind::InvalidInput, "at");
        assert_eq!(
            a.decision(0).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "decision"
        );

        let t = a.trail();
        assert_eq!(
            t.size().unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "trail size"
        );
        assert_eq!(
            t.begin(0).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "trail begin"
        );
        assert_eq!(
            t.end(0).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "trail end"
        );
        assert_eq!(
            t.at(0).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "trail at"
        );

        // The five infallible getters stay callable, per J-notes.md's ruling
        // (pure field reads; the header's own "no further calls" note is
        // never attached to any of them).
        let _ = a.decision_level();
        let _ = a.root_level();
        let _ = a.has_conflict();
        let _ = a.size();
        let _ = a.is_total();

        Ok(())
    }
}

#[test]
fn assignment_and_trail_queries_are_refused_through_a_stopped_init() {
    let mut ctl = grounded("a.");
    ctl.register_propagator(QueriesAssignmentAfterInitStop)
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
}

// ---------------------------------------------------------------------------
// has_watch on PropagateControl still reaches clingo after Stop, instead of
// returning false unconditionally: distinguished from clingo's own (also
// "false for a foreign literal") behaviour by watching a literal that really
// is still watched after Stop (watches are not cleared by Stop), so clingo's
// own honest answer is `true`, but the guard requires `false` regardless.

struct HasWatchStopsReachingClingoAfterStop {
    checked: Arc<Mutex<bool>>,
}
impl Propagator for HasWatchStopsReachingClingoAfterStop {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let slit = changes[0];
        control.add_watch(slit)?;
        assert!(
            control.has_watch(slit),
            "sanity: the watch is really there before Stop"
        );

        let stop = control.add_clause(&[-slit], ClauseType::Learnt)?;
        assert!(
            stop.is_stop(),
            "sanity: a fails its own watched literal, forcing Stop"
        );

        assert!(
            !control.has_watch(slit),
            "has_watch must return false without reaching clingo once Stop \
             was reported; clingo itself still reports the \
             literal watched here (a watch is not cleared by Stop), so this \
             can only pass once has_watch gets its own check_not_stopped \
             call, which it currently lacks entirely"
        );
        *self.checked.lock().unwrap() = true;
        Ok(())
    }
}

#[test]
fn has_watch_on_propagate_control_returns_false_without_reaching_clingo_after_stop() {
    let checked = Arc::new(Mutex::new(false));
    let mut ctl = grounded("a.");
    ctl.register_propagator(HasWatchStopsReachingClingoAfterStop {
        checked: Arc::clone(&checked),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert!(
        *checked.lock().unwrap(),
        "sanity: propagate fired and Stop was reached"
    );
}

// ---------------------------------------------------------------------------
// Assignment/Trail queries through a stopped PropagateControl: the mirror of
// the PropagateInit case above.

struct QueriesAssignmentAfterControlStop {
    checked: Arc<Mutex<bool>>,
}
impl Propagator for QueriesAssignmentAfterControlStop {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let slit = changes[0];
        let stop = control.add_clause(&[-slit], ClauseType::Learnt)?;
        assert!(stop.is_stop(), "sanity: forces Stop");

        let a = control.assignment();
        assert!(a.size() >= 1, "sanity");
        assert!(a.has_literal(slit), "sanity: has_literal is exempt");

        assert_eq!(
            a.level(slit).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "level"
        );
        assert_eq!(
            a.is_fixed(slit).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "is_fixed"
        );
        assert_eq!(
            a.is_true(slit).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "is_true"
        );
        assert_eq!(
            a.is_false(slit).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "is_false"
        );
        assert_eq!(
            a.truth_value(slit).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "truth_value"
        );
        assert_eq!(a.at(0).unwrap_err().kind(), ErrorKind::InvalidInput, "at");
        assert_eq!(
            a.decision(0).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "decision"
        );

        let t = a.trail();
        assert_eq!(
            t.size().unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "trail size"
        );
        assert_eq!(
            t.begin(0).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "trail begin"
        );
        assert_eq!(
            t.end(0).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "trail end"
        );
        assert_eq!(
            t.at(0).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "trail at"
        );

        let _ = a.decision_level();
        let _ = a.root_level();
        let _ = a.has_conflict();
        let _ = a.size();
        let _ = a.is_total();

        *self.checked.lock().unwrap() = true;
        Ok(())
    }
}

#[test]
fn assignment_and_trail_queries_are_refused_through_a_stopped_propagate_control() {
    let checked = Arc::new(Mutex::new(false));
    let mut ctl = grounded("a.");
    ctl.register_propagator(QueriesAssignmentAfterControlStop {
        checked: Arc::clone(&checked),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert!(
        *checked.lock().unwrap(),
        "sanity: propagate fired and Stop was reached"
    );
}

// ============================================================================
// Assignment::size counts the variable space,
// not the trail. Clingox's implementation matches clasp -- pinned here so
// the rustdoc has a fixture tied to the oracle.
// ============================================================================

struct RecordsSizeAndTrailAtFirstFixpointCheck {
    recorded: Arc<Mutex<Option<(usize, u32, u32)>>>,
}
impl Propagator for RecordsSizeAndTrailAtFirstFixpointCheck {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Fixpoint);
        Ok(())
    }

    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let mut recorded = self.recorded.lock().unwrap();
        if recorded.is_none() {
            let a = control.assignment();
            *recorded = Some((a.size(), a.trail().size()?, a.decision_level()));
        }
        Ok(())
    }
}

#[test]
fn assignment_size_counts_the_variable_space_not_the_trail() {
    // Oracle (pyclingo 5.8.2, 2026-09-28, `{a; b}.`, CheckMode::Fixpoint, the
    // first check call, reproduced directly for this file):
    // len(a) == 4, len(a.trail) == 1, a.decision_level == 0.
    let recorded = Arc::new(Mutex::new(None));
    let mut ctl = grounded("{a; b}.");
    ctl.register_propagator(RecordsSizeAndTrailAtFirstFixpointCheck {
        recorded: Arc::clone(&recorded),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();

    let (size, trail_size, decision_level) = recorded.lock().unwrap().expect("check fired");
    assert_eq!(
        decision_level, 0,
        "sanity: the first check fires before any decision"
    );
    assert_eq!(
        size, 4,
        "Assignment::size at the oracle's first Fixpoint check"
    );
    assert_eq!(
        trail_size, 1,
        "Trail::size (the actually-assigned count) differs from size() here: \
         this pins the real semantics: size() is the variable space, not the \
         number of assigned literals"
    );
}

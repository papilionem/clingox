//! `Propagator::check` firing at exactly the points `CheckMode` configures (the
//! getter/setter round trip is tested elsewhere; this file covers the
//! behavioural half, which needs `check` dispatched).
//!
//! Ports `libpyclingo/clingo/tests/test_propagator.py::test_propagator_mode`
//! and the check-mode-firing half of `check-py.lp`
//! (`clingox-sys/clingo/app/clingo/tests/python/check-py.lp`).
//!
//! Every expected count below was checked directly against clingo 5.8.2 (the
//! Python module `clingo`, 2026-09-28).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use clingox::propagate::{
    CheckMode, ClauseType, PropagateControl, PropagateInit, Propagator, SolverLiteral, UndoMode,
};
use clingox::{Control, Part, Result};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a fresh control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// As [`grounded`], but with `--models=0`: `ctl.solve(&[])`'s own single
/// call otherwise stops at clingo's own default of one model, which would
/// undercount every test below that needs every model actually enumerated
/// (a `check`/`undo` firing count, or a search that must explore beyond the
/// first model found, e.g. `check_call_count`'s own oracle, which was
/// counted against pyclingo's `["0"]`).
fn grounded_all_models(program: &str) -> Control {
    let mut ctl = Control::with_args(["--models=0"]).expect("a fresh control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

// ---------------------------------------------------------------------------
// check() fires exactly at the configured CheckMode's own points, for each
// of the four variants, on the same fixture (`{a; b; c}.`, `["0"]`, every
// atom watched so `propagate` cannot starve `check`'s own preconditions).
//
// Oracle (pyclingo 5.8.2, `{a; b; c}.`, `["0"]`, checked directly):
//   Off:      0 check() calls
//   Total:    8 check() calls (one per total assignment: 2^3 models)
//   Fixpoint: 16 check() calls
//   Both:     24 check() calls (== Total's 8 + Fixpoint's 16)

struct CountsCheck {
    mode: CheckMode,
    count: Arc<AtomicU32>,
}

impl Propagator for CountsCheck {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(self.mode);
        for atom in &init.symbolic_atoms()? {
            let slit = init.solver_literal(atom?.literal())?;
            init.add_watch(slit)?;
        }
        Ok(())
    }

    fn check(&self, _control: &mut PropagateControl<'_>) -> Result<()> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn check_call_count(mode: CheckMode) -> u32 {
    let count = Arc::new(AtomicU32::new(0));
    let mut ctl = grounded_all_models("{a; b; c}.");
    ctl.register_propagator(CountsCheck {
        mode,
        count: Arc::clone(&count),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    count.load(Ordering::SeqCst)
}

#[test]
fn check_mode_off_never_fires_check() {
    assert_eq!(check_call_count(CheckMode::Off), 0);
}

#[test]
fn check_mode_total_fires_once_per_total_assignment() {
    assert_eq!(check_call_count(CheckMode::Total), 8);
}

#[test]
fn check_mode_fixpoint_fires_more_often_than_total() {
    assert_eq!(check_call_count(CheckMode::Fixpoint), 16);
}

#[test]
fn check_mode_both_fires_the_sum_of_total_and_fixpoint() {
    assert_eq!(check_call_count(CheckMode::Both), 24);
}

// ---------------------------------------------------------------------------
// libpyclingo::test_propagator_mode, ported: under CheckMode::Fixpoint and
// UndoMode::Always, every check() at a level is paired with exactly one
// undo() when that level is later undone, plus one extra check() for the
// final level, which no undo() ever retires (num_check == num_undo + 1).
//
// Oracle (pyclingo 5.8.2, `{a; b}.`, default options, checked directly):
// num_check=4, num_undo=3 (this document's own run; the original Python
// test only asserts num_check >= 3 and the +1 relationship, since clasp's
// own search is not required to visit the same number of nodes on every
// build; this port keeps the weaker, portable assertion for num_check and
// the exact relationship, which is the part actually characterising
// UndoMode::Always).

struct CountsCheckAndUndo {
    num_check: Arc<Mutex<u32>>,
    num_undo: Arc<Mutex<u32>>,
}

impl Propagator for CountsCheckAndUndo {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Fixpoint);
        init.set_undo_mode(UndoMode::Always);
        assert_eq!(init.check_mode(), CheckMode::Fixpoint);
        assert_eq!(init.undo_mode(), UndoMode::Always);
        Ok(())
    }

    fn check(&self, _control: &mut PropagateControl<'_>) -> Result<()> {
        *self.num_check.lock().unwrap() += 1;
        Ok(())
    }

    fn undo(&self, _control: &PropagateControl<'_>, _changes: &[SolverLiteral]) {
        *self.num_undo.lock().unwrap() += 1;
    }
}

#[test]
fn check_and_undo_fire_together_under_fixpoint_and_always() {
    let num_check = Arc::new(Mutex::new(0));
    let num_undo = Arc::new(Mutex::new(0));
    let mut ctl = grounded("{a; b}.");
    ctl.register_propagator(CountsCheckAndUndo {
        num_check: Arc::clone(&num_check),
        num_undo: Arc::clone(&num_undo),
    })
    .unwrap();
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());

    let num_check = *num_check.lock().unwrap();
    let num_undo = *num_undo.lock().unwrap();
    assert!(
        num_check >= 3,
        "at least 3 check() calls, as the oracle's own"
    );
    assert_eq!(
        num_check,
        num_undo + 1,
        "every check() but the final, unretired one is paired with exactly one undo()"
    );
}

// ---------------------------------------------------------------------------
// check-py.lp, ported (its check-mode-firing half; add_clause needs no new
// coverage beyond the existing add_clause-from-check tests):
// init.check_mode defaults to Total (confirmed directly, and matches
// clingox's own documented default), and a check() that forces every
// unassigned literal true, one at a time via add_clause, converges to the
// single all-true model.
//
// Oracle (pyclingo 5.8.2, `{ p(1..10) }.`, `["0"]`, checked directly):
// exactly one model, every p(1)..p(10) true.

struct ForcesEveryUnassignedLiteralTrue {
    lits: Mutex<Vec<SolverLiteral>>,
}

impl Propagator for ForcesEveryUnassignedLiteralTrue {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        assert_eq!(
            init.check_mode(),
            CheckMode::Total,
            "the documented default"
        );
        let mut lits = Vec::new();
        for atom in &init.symbolic_atoms()? {
            lits.push(init.solver_literal(atom?.literal())?);
        }
        *self.lits.lock().unwrap() = lits;
        init.set_check_mode(CheckMode::Fixpoint);
        assert_eq!(init.check_mode(), CheckMode::Fixpoint);
        Ok(())
    }

    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let lits = self.lits.lock().unwrap().clone();
        for lit in lits {
            let assignment = control.assignment();
            let unassigned = assignment.truth_value(lit)?.is_none();
            if unassigned {
                let _ = control.add_clause(&[lit], ClauseType::Learnt)?;
                break;
            }
        }
        Ok(())
    }
}

#[test]
fn check_forces_every_unassigned_literal_true_in_turn() {
    let mut ctl = grounded("{ p(1..10) }.");
    ctl.register_propagator(ForcesEveryUnassignedLiteralTrue {
        lits: Mutex::new(Vec::new()),
    })
    .unwrap();

    let mut models = Vec::new();
    let _ = ctl
        .for_each_model(&[], |m| {
            let mut syms: Vec<String> = m
                .symbols(clingox::ShowType::SHOWN)?
                .iter()
                .map(ToString::to_string)
                .collect();
            syms.sort();
            models.push(syms);
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .unwrap();

    assert_eq!(
        models.len(),
        1,
        "the propagator forces a unique, all-true model"
    );
    // Sorted the same way the model's own symbols are (syms.sort() above,
    // lexicographic on the string, not numeric on the argument): "p(10)"
    // sorts before "p(2)".
    let mut expected: Vec<String> = (1..=10).map(|i| format!("p({i})")).collect();
    expected.sort();
    assert_eq!(models[0], expected);
}

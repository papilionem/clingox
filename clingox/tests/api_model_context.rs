//! `Model::context`/`SolveControl`: reachable from every model
//! delivery path with no propagator registered anywhere, `add_clause`
//! narrowing a solving step's own enumeration without persisting past it,
//! `symbolic_atoms` matching the control's own view, reading the model
//! after `add_clause` in the same callback (checked under `ASan`),
//! and a foreign program literal being safely, silently accepted rather
//! than rejected (a deliberate difference from this crate's `SolverLiteral`
//! rule).
//!
//! `Model::context` is infallible (`SolveControl<'_>`, not
//! `Result<SolveControl<'_>>`): see the same "Exact signatures" section for
//! why (a pure pointer cast in `clingo_model_context`'s own C body, which
//! cannot throw). Every expected value below was checked directly against
//! clingo 5.8.2 (the Python module `clingo`, 2026-09-28).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each handler is defined next to the test that uses it"
)]

use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};

use clingox::{Control, ExtendableModel, Part, ProgramLiteral, Result, SolveEventHandler};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a fresh control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// A `ProgramLiteral` legitimately obtained from a much larger, separate
/// control's grounding: valid there, but unknown to a small control's own
/// grounding. Identical in spirit to `api_propagator_decide.rs`'s
/// `foreign_literal` helper, for `ProgramLiteral` instead of
/// `SolverLiteral`.
fn foreign_program_literal() -> ProgramLiteral {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("{p(1..200)}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.symbolic_atoms()
        .unwrap()
        .iter()
        .map(|a| a.unwrap().literal())
        .max_by_key(|lit| lit.get())
        .expect("at least one atom exists")
}

// ---------------------------------------------------------------------------
// context()/add_clause is reachable, and narrows the rest of the solving
// step's enumeration, from each of the three model-delivery paths, with no
// propagator registered anywhere: a plain blocking next_model loop
// (solve_yield), for_each_model, and ExtendableModel inside a
// SolveEventHandler. Oracle (pyclingo 5.8.2, `1 {a; b; c} 1.`, `--models=0`):
// each of the 3 models (a, b, c) is found exactly once regardless of which
// path negates the true atom's own literal after seeing it, since each
// model is already distinct; the meaningful check is that `context()`
// itself works (returns `Ok`, no panic, no error) with zero propagators
// registered, with a passing test on all three (not only the one path a
// propagator-based test would already cover).

fn negate_whichever_is_true(m: &clingox::Model) -> Result<()> {
    for name in ["a", "b", "c"] {
        let atom = clingox::Symbol::function(name, &[])?;
        if m.contains(atom)? {
            let plit = clingox_program_literal_of(m, name)?;
            m.context().add_clause(&[-plit])?;
            return Ok(());
        }
    }
    Ok(())
}

/// The program literal of a named nullary atom, read through this model's
/// own `context().symbolic_atoms()` rather than `Control::symbolic_atoms`,
/// to also exercise `SolveControl::symbolic_atoms` in every path below.
fn clingox_program_literal_of(m: &clingox::Model, name: &str) -> Result<ProgramLiteral> {
    let atom = clingox::Symbol::function(name, &[])?;
    Ok(m.context()
        .symbolic_atoms()?
        .find(atom)?
        .unwrap_or_else(|| panic!("{name} is an atom"))
        .literal())
}

#[test]
fn context_and_add_clause_work_from_a_plain_yield_loop_with_no_propagator() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("1 {a; b; c} 1.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut seen = 0;
    let mut handle = ctl.solve_yield(&[]).unwrap();
    while let Some(model) = handle.next_model().unwrap() {
        negate_whichever_is_true(model).unwrap();
        seen += 1;
    }
    let result = handle.close().unwrap();
    assert!(result.is_sat());
    assert_eq!(seen, 3, "each of a/b/c is its own, distinct model");
}

#[test]
fn context_and_add_clause_work_from_for_each_model_with_no_propagator() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("1 {a; b; c} 1.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut seen = 0;
    let result = ctl
        .for_each_model(&[], |m| {
            negate_whichever_is_true(m)?;
            seen += 1;
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(result.is_exhausted());
    assert_eq!(seen, 3);
}

struct NegatesTrueAtom {
    seen: Arc<Mutex<usize>>,
}
impl SolveEventHandler for NegatesTrueAtom {
    fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
        negate_whichever_is_true(model)?;
        *self.seen.lock().unwrap() += 1;
        Ok(ControlFlow::Continue(()))
    }
}

#[test]
fn context_and_add_clause_work_from_extendable_model_with_no_propagator() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("1 {a; b; c} 1.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let seen = Arc::new(Mutex::new(0));
    let handler = NegatesTrueAtom {
        seen: Arc::clone(&seen),
    };
    let result = ctl
        .solve_with_events(clingox::SolveOptions::new(), handler)
        .unwrap();
    assert!(result.is_exhausted());
    assert_eq!(*seen.lock().unwrap(), 3);
}

// ---------------------------------------------------------------------------
// add_clause's effect is scoped to the current solving step: negating a
// model's own true atom's literal the moment it is seen narrows the rest
// of *that* solve (the model that would repeat the same atom stays
// unreachable for the remainder of the step), but a later, separate
// `solve()` call on the same control is unaffected. Oracle (pyclingo 5.8.2,
// `{a;b}.`): `--models=0` finds `{}`, `{b}`, `{a}` (3 of the 4 possible
// models) when `{a}` is negated the moment it is seen, since `{a,b}` also
// requires `a` true; a second, separate `solve()` afterward finds all 4
// again.

#[test]
fn add_clause_narrows_the_current_step_but_does_not_persist_to_a_later_solve() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let mut first_step: Vec<Vec<String>> = Vec::new();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    while let Some(model) = handle.next_model().unwrap() {
        let syms: Vec<String> = model
            .symbols(clingox::ShowType::SHOWN)
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        if syms == ["a"] {
            let plit = clingox_program_literal_of(model, "a").unwrap();
            model.context().add_clause(&[-plit]).unwrap();
        }
        first_step.push(syms);
    }
    let _ = handle.close().unwrap();
    assert_eq!(
        first_step.len(),
        3,
        "the fourth model, {{a,b}}, is excluded"
    );

    let mut second_step: Vec<Vec<String>> = Vec::new();
    let _ = ctl
        .for_each_model(&[], |m| {
            second_step.push(
                m.symbols(clingox::ShowType::SHOWN)?
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            );
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(
        second_step.len(),
        4,
        "the clause from the earlier step must not persist into a later, separate solve() call"
    );
}

// ---------------------------------------------------------------------------
// SolveControl::symbolic_atoms returns the same domain Control::
// symbolic_atoms does, for the same control at the same point.

#[test]
fn solve_controls_symbolic_atoms_matches_the_controls_own() {
    let mut ctl = grounded("a. b(1). b(2).");
    let control_atoms: Vec<clingox::Symbol> = {
        let mut syms: Vec<_> = ctl
            .symbolic_atoms()
            .unwrap()
            .iter()
            .map(|a| a.unwrap().symbol())
            .collect();
        syms.sort_unstable();
        syms
    };
    let mut checked = false;
    let mut handle = ctl.solve_yield(&[]).unwrap();
    if let Some(model) = handle.next_model().unwrap() {
        let mut context_atoms: Vec<clingox::Symbol> = model
            .context()
            .symbolic_atoms()
            .unwrap()
            .iter()
            .map(|a| a.unwrap().symbol())
            .collect();
        context_atoms.sort_unstable();
        assert_eq!(context_atoms, control_atoms);
        checked = true;
    }
    let _ = handle.close().unwrap();
    assert!(checked, "sanity: a model was found");
}

// ---------------------------------------------------------------------------
// Reading the model after add_clause in the same callback: the
// ASan evidence that the aliased `&Model`/`SolveControl` pair is benign
// (add_clause appends to a side list, never touching anything a concurrent
// `Model` read observes). An ordinary test, added to the ASan run
// (`cargo xtask sanitize`) by virtue of being a normal test; no special
// harness needed here.

#[test]
fn reading_the_model_after_add_clause_in_the_same_callback_works() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("1 {a; b; c} 1.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut checked = false;
    let mut handle = ctl.solve_yield(&[]).unwrap();
    if let Some(model) = handle.next_model().unwrap() {
        let a = clingox::Symbol::function("a", &[]).unwrap();
        let a_true_before = model.contains(a).unwrap();
        let number_before = model.number();
        let plit = clingox_program_literal_of(model, "a").unwrap();
        model.context().add_clause(&[-plit]).unwrap();
        // Reading the same model through its own `&Model` methods, after
        // `add_clause` ran through the aliased `SolveControl`, must still
        // work and report the same values: `add_clause` does not touch
        // anything a `Model` read observes.
        assert_eq!(model.contains(a).unwrap(), a_true_before);
        assert_eq!(model.number(), number_before);
        checked = true;
    }
    let _ = handle.close().unwrap();
    assert!(checked, "sanity: a model was found");
}

// ---------------------------------------------------------------------------
// A foreign program literal (legitimately obtained from a different, larger
// control) is safely, silently accepted by add_clause, never crashing and never
// erroring: `ProgramLiteral`'s own established contract already covers this
// ("clingox cannot detect that; the result is wrong but never unsafe",
// `clingox/src/atoms.rs`), unlike every `SolverLiteral`-taking method in this
// crate. Reproduced directly against clingo 5.8.2 via a raw C-level probe: no
// magnitude a caller can construct through `ProgramLiteral::from_raw` ever
// crashed the process.

#[test]
fn add_clause_with_a_foreign_program_literal_is_accepted_not_rejected() {
    let foreign = foreign_program_literal();
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle.next_model().unwrap().expect("a. has a model");
    // No `ErrorKind::InvalidInput`, unlike every `SolverLiteral`-taking
    // method in `PropagateInit`/`PropagateControl` given a foreign literal:
    // this is `ProgramLiteral`'s own, documented, deliberately weaker
    // contract, not an oversight (see the module doc comment above).
    model.context().add_clause(&[foreign]).unwrap();
}

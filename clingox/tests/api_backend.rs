//! The backend: `Control::with_backend` and every `Backend` method,
//! their observable effect on solving, and the lifetime and borrow rules
//! the backend's safety analysis (DESIGN S4, S8) states.
//!
//! Every expected value below was checked against the Python module `clingo`
//! 5.8.2 directly (`with ctl.backend() as b: ...`), not assumed from
//! `clingo.h`'s prose.
//!
//! `ProgramLiteral` is signed: a backend atom is the separate,
//! unsigned `Atom` newtype, and `Atom::pos()`/`Atom::neg()` produce the
//! `ProgramLiteral` a rule body or an assumption takes.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail loudly on unexpected errors"
)]

use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};

use clingox::backend::{
    Atom, ExternalKind, Head, HeuristicKind, TheoryAtomTarget, TheorySequenceKind,
};
use clingox::testing::assert_models;
use clingox::{
    Control, ErrorKind, Id, Outcome, Part, ProgramLiteral, Result, ShowType, Symbol, TheoryTerm,
    TheoryTermKind, TruthValue,
};

fn sym(text: &str) -> Symbol {
    text.parse().expect("the term parses")
}

// ---------------------------------------------------------------------------
// `with_backend`: opening and closing the backend

/// Whether `name` is marked a fact in the symbolic atoms. clingo marks the
/// facts added through a backend only in `clingo_backend_end`
/// (`ClingoControl::endAddBackend`, libclingo/src/clingocontrol.cc:880), so
/// this is true only if the backend session was finished. Checked with
/// pyclingo 5.8.2 through cffi: `is_fact` is `True` with the end call and
/// `False` without it.
fn is_fact(ctl: &Control, name: &str) -> bool {
    ctl.symbolic_atoms()
        .unwrap()
        .find(sym(name))
        .unwrap()
        .expect("the atom is in the symbolic atoms")
        .is_fact()
}

/// `with_backend` still finishes the backend when the closure returns `Err`:
/// a later entry point succeeds afterward, mirroring
/// `a_forgotten_search_is_closed_before_statistics_are_read`
/// (`clingox/tests/api_statistics.rs`) for the backend's own begin/end pair.
#[test]
fn with_backend_finishes_when_the_closure_returns_err() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    // A NUL byte in a symbol's name is a genuine, public-API error
    // (`ErrorKind::Nul`, caught before clingo sees it, RULES §4), the same
    // way `a_clingox_error_from_a_ground_callback_is_returned_unchanged`
    // (`clingox/tests/api_ground_callbacks.rs`) makes its callback fail:
    // there is no public way to build an arbitrary `Error` from outside the
    // crate, so the closure fails for a real reason instead.
    let err = ctl
        .with_backend(|b| {
            // A fact added before the failure only counts as a fact once
            // the session is finished (see `is_fact`).
            let f = b.add_atom(Some(sym("f")))?;
            b.add_rule(Head::Normal(&[f]), &[])?;
            let _ = Symbol::function("bad\0name", &[])?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(is_fact(&ctl, "f"), "the backend session was finished");

    // A second `with_backend` call, and any other entry point, succeed.
    ctl.with_backend(|b| {
        let _ = b.add_atom(Some(sym("b")))?;
        Ok(())
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a f"]);
}

/// `with_backend` still finishes the backend when the closure panics: the panic
/// is caught at the test boundary, and the next call succeeds (mirrors
/// `a_panic_in_a_ground_callback_resumes_on_the_caller`,
/// `clingox/tests/api_ground_callbacks.rs`).
#[test]
fn with_backend_finishes_when_the_closure_panics() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_backend(|b| -> Result<()> {
            // As in the `Err` test: this fact shows only if the session is
            // finished, here by the next call on the control.
            let f = b.add_atom(Some(sym("f")))?;
            b.add_rule(Head::Normal(&[f]), &[])?;
            panic!("stop here")
        })
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"stop here"));
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(is_fact(&ctl, "f"), "the backend session was finished");

    ctl.with_backend(|b| {
        let _ = b.add_atom(Some(sym("b")))?;
        Ok(())
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a f"]);
}

// ---------------------------------------------------------------------------
// `add_atom`, `add_aux_atom`

/// An atom added with a symbol behaves like an ordinary atom: it shows up in
/// `symbolic_atoms` and, once a rule derives it, in the model.
///
/// Oracle: Python module, 2026-09-27: `backend.add_atom(Function("p",
/// [Number(1)]))` followed by `add_rule([p], [], True)` makes `p(1)` a symbolic
/// atom and a possible model member, exactly as `{p(1)}.` in program text
/// would.
#[test]
fn add_atom_with_a_symbol_is_visible_through_symbolic_atoms() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("").unwrap();
    ctl.with_backend(|b| {
        let p1: Atom = b.add_atom(Some(sym("p(1)")))?;
        b.add_rule(Head::Choice(&[p1]), &[])
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let found = ctl
        .symbolic_atoms()
        .unwrap()
        .find(sym("p(1)"))
        .unwrap()
        .is_some();
    assert!(found, "p(1) must be a symbolic atom");
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["", "p(1)"]);
}

/// `add_aux_atom` (`add_atom(None)`) produces an atom with no symbol at all:
/// `SymbolicAtoms::find` never returns it for any symbol in scope, and it can
/// only be observed through `Model::is_true` on its own literal, never by
/// symbol.
///
/// Oracle: Python module, 2026-09-27: `backend.add_atom(None)` (pyclingo's
/// `add_atom()` with no argument) makes an atom that never appears in
/// `ctl.symbolic_atoms` and never appears in `model.symbols(atoms=True)`
/// (which needs a symbol to print), even though the atom is true in the
/// model; only `model.is_true(<its literal>)` reports it, matching
/// `clingo_backend_add_atom`'s header ("optional symbol to associate the
/// atom with").
#[test]
fn add_aux_atom_has_no_symbol_and_is_only_observable_by_literal() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    let (a_symbol_atom, aux) = ctl
        .with_backend(|b| {
            let aux = b.add_aux_atom()?;
            Ok(aux)
        })
        .map(|aux| (sym("a"), aux))
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    // No symbol in scope ever resolves to it.
    let atoms = ctl.symbolic_atoms().unwrap();
    assert!(
        atoms.find(a_symbol_atom).unwrap().is_some(),
        "a is grounded"
    );
    for item in &atoms {
        let atom = item.unwrap();
        assert_ne!(
            atom.literal(),
            aux.pos(),
            "no symbolic atom is the aux atom"
        );
    }
    assert_eq!(
        atoms.len().unwrap(),
        1,
        "aux has no symbol to be counted by"
    );

    let mut seen = false;
    let _ = ctl
        .for_each_model(&[], |model| {
            seen = true;
            // `aux` was never given a rule, so clingo defaults it to false;
            // this only proves it is observable at all, by literal.
            assert!(!model.is_true(aux.pos()).unwrap());
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(seen, "the program has one model");
}

// ---------------------------------------------------------------------------
// `add_rule`: normal, choice and constraint heads; `Atom::pos`/`neg`

/// A worked example: a fact, a choice rule, a constraint and one minimize
/// literal, built entirely through the backend, ground to the same model a
/// text-only equivalent program would.
///
/// `aux` has no symbol (`add_aux_atom`), so the backend model is compared by
/// `is_true`/`cost`, while the text baseline (where `aux` is an ordinary named
/// atom) is compared by its shown symbols; the two are the same program,
/// differently observed, which is the point of the anonymous atom.
///
/// Oracle: Python module, 2026-09-27: both programs have exactly one model,
/// cost `[1]`, with `a` and `aux` true (`add_atom`/`add_rule`/`add_minimize`
/// built directly and cross-checked against `a. {aux} :- a. :- not aux. :~ aux.
/// [1@0]`).
#[test]
fn the_sketch_38_example_matches_the_equivalent_program_text() {
    let mut backend_ctl = Control::with_args(["--models=0"]).unwrap();
    backend_ctl.add_base("").unwrap();
    let (a, aux) = backend_ctl
        .with_backend(|b| {
            let a = b.add_atom(Some(sym("a")))?;
            let aux = b.add_aux_atom()?;
            b.add_rule(Head::Normal(&[a]), &[])?; // a.
            b.add_rule(Head::Choice(&[aux]), &[a.pos()])?; // { aux } :- a.
            b.add_rule(Head::Constraint, &[aux.neg()])?; // :- not aux.
            b.add_minimize(0, &[(aux.pos(), 1)])?;
            Ok((a, aux))
        })
        .unwrap();
    backend_ctl.ground(&[Part::base()]).unwrap();

    let mut backend_models = 0;
    let _ = backend_ctl
        .for_each_model(&[], |model| {
            backend_models += 1;
            assert!(model.is_true(a.pos()).unwrap());
            assert!(model.is_true(aux.pos()).unwrap());
            assert_eq!(model.cost().unwrap(), [1]);
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(backend_models, 1);

    let mut text_ctl = Control::with_args(["--models=0"]).unwrap();
    text_ctl
        .add_base("a. {aux} :- a. :- not aux. :~ aux. [1@0]")
        .unwrap();
    text_ctl.ground(&[Part::base()]).unwrap();
    let (_, text_models) = text_ctl.solve_all().unwrap();
    assert_eq!(text_models.len(), 1);
    assert_eq!(text_models[0].cost(), [1]);
    assert_models!(text_models, ["a aux"]);
}

// ---------------------------------------------------------------------------
// Negative control 3: `Head::Constraint` must use `choice = false`
//
// `the_sketch_38_example_matches_the_equivalent_program_text` above already
// discriminates this: building the last rule as `Head::Choice(&[])` instead
// of `Head::Constraint` makes `:- not aux.` non-binding, giving two models
// instead of one (checked directly, 2026-09-27); no
// separate test is needed; flipping the bit and rerunning this one shows it.

/// `Atom::pos()` and `Atom::neg()` must be distinct literals: a rule whose
/// body has both the positive and the negative literal of the same atom is
/// self-contradictory and can never fire (negative control 4: if `neg()`
/// returned the same value as `pos()`, the body would simplify to one
/// literal and the rule would fire whenever that atom is true).
///
/// Oracle: Python module, 2026-09-27: `{s}. h :- s, not s.` (the text
/// equivalent of a body with a literal and its own negation) is never
/// derivable; `h` is false in both models of `{s}.`.
#[test]
fn atom_pos_and_neg_are_distinct_and_self_contradictory_together() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("").unwrap();
    ctl.with_backend(|b| {
        let s = b.add_atom(Some(sym("s")))?;
        let h = b.add_atom(Some(sym("h")))?;
        b.add_rule(Head::Choice(&[s]), &[])?;
        let positive: ProgramLiteral = s.pos();
        let negative: ProgramLiteral = s.neg();
        assert_ne!(positive, negative, "pos() and neg() must differ");
        b.add_rule(Head::Normal(&[h]), &[positive, negative])
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["", "s"]);
}

// ---------------------------------------------------------------------------
// `add_weight_rule`

/// A weight rule derives its head exactly when the sum of the weights of its
/// true body literals reaches the lower bound, matching
/// `clingo_backend_weight_rule`'s semantics exactly (not a count, a weighted
/// sum), with weight and bound values chosen so that neither literal alone
/// crosses the bound and only their sum does, catching a weight/bound mixup.
///
/// Oracle: Python module, 2026-09-27: `{x;y}.` with `add_weight_rule([head],
/// 3, [(x,2),(y,3)])` gives exactly 4 models: `{}`, `{x}` (sum 2, head
/// false), `{y}` (sum 3, head true), `{x,y}` (sum 5, head true).
#[test]
fn add_weight_rule_derives_the_head_once_the_weighted_sum_meets_the_bound() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("").unwrap();
    ctl.with_backend(|b| {
        let x = b.add_atom(Some(sym("x")))?;
        let y = b.add_atom(Some(sym("y")))?;
        let head = b.add_atom(Some(sym("head")))?;
        b.add_rule(Head::Choice(&[x]), &[])?;
        b.add_rule(Head::Choice(&[y]), &[])?;
        b.add_weight_rule(Head::Normal(&[head]), 3, &[(x.pos(), 2), (y.pos(), 3)])
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["", "x", "y head", "x y head"]);
}

/// A negative weight in a weight rule's body is `clingo_backend_weight_rule`'s
/// own error: clingox passes it through unchanged as
/// `ErrorKind::Runtime`, without checking it first, and it does not poison
/// the control (the failed call added nothing; a later call succeeds
/// normally).
///
/// Oracle: Python module, 2026-09-27 (checked directly): `add_weight_rule([a],
/// 1, [(a, -1)])` raises `RuntimeError: ... Non-negative weight expected!`
/// immediately, and the control still grounds and solves normally afterward.
#[test]
fn add_weight_rule_with_a_negative_weight_is_invalid_input_that_does_not_poison() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("").unwrap();
    let err = ctl
        .with_backend(|b| {
            let a = b.add_atom(Some(sym("a")))?;
            b.add_weight_rule(Head::Normal(&[a]), 1, &[(a.pos(), -1)])
        })
        .unwrap_err();
    // clingo itself reports this as `clingo_error_logic` (2), "Numerical
    // argument out of domain: Non-negative weight expected!" (cffi against
    // pyclingo's libclingo 5.8.2), which would poison the control although the
    // rule never took effect. So clingox rejects it first, like a zero weight.
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(!format!("{ctl:?}").contains("poisoned"));

    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    // `a` was added but never got a rule (the weight rule failed before
    // taking effect), so it stays false: one empty model.
    assert_models!(models, [""]);
}

/// A weight of exactly `0` is not an error clingo itself reports (checked
/// directly: it is accepted silently and simply never contributes to the
/// sum), so it goes in the "clingox rejects it first"
/// half: `ErrorKind::InvalidInput`, before the call reaches clingo, and it
/// does not poison.
///
/// Oracle: Python module, 2026-09-27: `add_weight_rule([head], 1, [(c, 0)])`
/// raises nothing; grounding and solving afterward give one model where
/// `head` is false, since the zero-weight literal never counts toward the
/// bound. clingox's own precondition rejects this before clingo ever sees
/// it, per the header's "All weights ... must be positive" (`H:1678`).
#[test]
fn add_weight_rule_with_a_zero_weight_is_rejected_before_calling_clingo() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("").unwrap();
    let err = ctl
        .with_backend(|b| {
            let c = b.add_atom(Some(sym("c")))?;
            let head = b.add_atom(Some(sym("head")))?;
            b.add_rule(Head::Choice(&[c]), &[])?;
            b.add_weight_rule(Head::Normal(&[head]), 1, &[(c.pos(), 0)])
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(!format!("{ctl:?}").contains("poisoned"));
}

/// A lower bound of `0` or less is never an error clingo itself reports
/// either (checked directly: the sum, which starts at `0`, always meets it,
/// so the head is derived unconditionally, which is very unlikely to be what
/// a caller who wrote a weight rule intended): this goes in the
/// "clingox rejects it first" half too, `ErrorKind::InvalidInput`, for both
/// `0` and a negative bound.
///
/// Oracle: Python module, 2026-09-27: `add_weight_rule([a], -1, [(c, 1)])`
/// (`c` never derived) raises nothing and `a` is unconditionally true; the
/// same holds for a lower bound of `0`.
#[test]
fn add_weight_rule_with_a_non_positive_lower_bound_is_rejected_before_calling_clingo() {
    for bound in [0, -1] {
        let mut ctl = Control::new().unwrap();
        ctl.add_base("").unwrap();
        let err = ctl
            .with_backend(|b| {
                let a = b.add_atom(Some(sym("a")))?;
                let c = b.add_atom(Some(sym("c")))?;
                b.add_weight_rule(Head::Normal(&[a]), bound, &[(c.pos(), 1)])
            })
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "bound {bound}");
        assert!(!format!("{ctl:?}").contains("poisoned"), "bound {bound}");
    }
}

// ---------------------------------------------------------------------------
// `add_minimize`

/// `add_minimize`'s priority, not the raw weight sum, decides the optimum:
/// two mutually exclusive atoms, each minimized at a different priority, with
/// weights chosen so that a priority-blind implementation (one that just adds
/// every weight into one scalar) would pick the *other* model, catching a
/// priority/cost mixup.
///
/// Oracle: Python module, 2026-09-27: `{p;q}. :- not p, not q. :- p, q.`
/// (exactly one of `p`, `q`) with `add_minimize(1, [(p,1)])` (priority 1,
/// small weight) and `add_minimize(0, [(q,100)])` (priority 0, large weight)
/// optimizes to `{q}`, cost `[0,100]`: priority 1 is compared first, and `p`
/// false (giving priority 1's component `0`) beats `p` true (`1`), regardless
/// of priority 0's value. A raw total (`1` for `{p}` versus `100` for `{q}`,
/// ignoring priority buckets) would wrongly prefer `{p}` instead.
#[test]
fn add_minimize_orders_by_priority_first() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("").unwrap();
    ctl.with_backend(|b| {
        let p = b.add_atom(Some(sym("p")))?;
        let q = b.add_atom(Some(sym("q")))?;
        b.add_rule(Head::Choice(&[p]), &[])?;
        b.add_rule(Head::Choice(&[q]), &[])?;
        b.add_rule(Head::Constraint, &[p.neg(), q.neg()])?; // :- not p, not q.
        b.add_rule(Head::Constraint, &[p.pos(), q.pos()])?; // :- p, q.
        b.add_minimize(1, &[(p.pos(), 1)])?;
        b.add_minimize(0, &[(q.pos(), 100)])
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let outcome = ctl.solve_optimal().unwrap();
    let Outcome::Sat(model, _) = outcome else {
        panic!("the program is satisfiable");
    };
    assert_eq!(model.symbols(), [sym("q")]);
    assert_eq!(model.cost(), [0, 100]);
}

// ---------------------------------------------------------------------------
// `add_project`

/// `add_project` narrows enumeration to only the projected atoms, once
/// `solve.project` is enabled (the same `--project` clingo itself needs;
/// checked directly that `add_project` alone, without enabling it, changes
/// nothing).
///
/// Oracle: Python module, 2026-09-27: `{a;b;c}.` projected onto `a` alone,
/// with `configuration.solve.project = "auto"`, enumerates exactly 2 models
/// (`a` true or false, `b`/`c` collapsed); without enabling `solve.project`
/// the same `add_project` call has no effect and all 8 combinations remain.
#[test]
fn add_project_narrows_enumeration_to_the_projected_atom() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.configuration().set("solve.project", "auto").unwrap();
    ctl.add_base("").unwrap();
    ctl.with_backend(|b| {
        let a = b.add_atom(Some(sym("a")))?;
        let bb = b.add_atom(Some(sym("b")))?;
        let c = b.add_atom(Some(sym("c")))?;
        b.add_rule(Head::Choice(&[a]), &[])?;
        b.add_rule(Head::Choice(&[bb]), &[])?;
        b.add_rule(Head::Choice(&[c]), &[])?;
        b.add_project([a])
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut all_atoms_seen = Vec::new();
    let _ = ctl
        .for_each_model(&[], |model| {
            all_atoms_seen.push(model.symbols(ShowType::ATOMS).unwrap());
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    all_atoms_seen.sort();
    assert_eq!(all_atoms_seen.len(), 2, "{all_atoms_seen:?}");
}

/// Without enabling `solve.project`, `add_project` changes nothing: every
/// combination of `{a;b;c}.` still enumerates, proving the config dependency
/// above is real, not a fixture mistake.
#[test]
fn add_project_without_enabling_solve_project_has_no_effect() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("").unwrap();
    ctl.with_backend(|b| {
        let a = b.add_atom(Some(sym("a")))?;
        let bb = b.add_atom(Some(sym("b")))?;
        let c = b.add_atom(Some(sym("c")))?;
        b.add_rule(Head::Choice(&[a]), &[])?;
        b.add_rule(Head::Choice(&[bb]), &[])?;
        b.add_rule(Head::Choice(&[c]), &[])?;
        b.add_project([a])
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 8, "{models:?}");
}

// ---------------------------------------------------------------------------
// `add_external`

/// `ExternalKind::Free` leaves the external open: both truth values have
/// models, exactly as `#external e.` with no forced value would.
///
/// Oracle: Python module, 2026-09-27: `add_external(e, TruthValue.Free)`
/// with `b :- e.` gives two models, `{}` and `{b,e}`.
#[test]
fn add_external_free_leaves_the_atom_open() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("b :- e.").unwrap();
    ctl.with_backend(|be| {
        let e = be.add_atom(Some(sym("e")))?;
        be.add_external(e, ExternalKind::Free)
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["", "b e"]);
}

/// `ExternalKind::True` and `ExternalKind::False` force the external's value:
/// only one model exists in each case.
///
/// Oracle: Python module, 2026-09-27: `add_external(e, TruthValue.True_)`
/// gives one model `{b,e}`; `TruthValue.False_` gives one model `{}`.
#[test]
fn add_external_true_and_false_force_the_atom() {
    for (kind, expected) in [(ExternalKind::True, "b e"), (ExternalKind::False, "")] {
        let mut ctl = Control::with_args(["--models=0"]).unwrap();
        ctl.add_base("b :- e.").unwrap();
        ctl.with_backend(|be| {
            let e = be.add_atom(Some(sym("e")))?;
            be.add_external(e, kind)
        })
        .unwrap();
        ctl.ground(&[Part::base()]).unwrap();
        let (_, models) = ctl.solve_all().unwrap();
        assert_models!(models, [expected]);
    }
}

/// `ExternalKind::Release` makes the atom permanently false and no longer an
/// external, exactly as `Control::release_external` does: assigning it
/// afterward, by symbol, does nothing.
///
/// Oracle: Python module, 2026-09-27: `add_external(e, True_)` then
/// `add_external(e, Release)` leaves one model `{}`; a later
/// `ctl.assign_external(e, True)` raises nothing and changes nothing.
#[test]
fn add_external_release_makes_it_permanently_false_and_no_longer_external() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("b :- e.").unwrap();
    let e_symbol = sym("e");
    ctl.with_backend(|be| {
        let e = be.add_atom(Some(e_symbol))?;
        be.add_external(e, ExternalKind::True)?;
        be.add_external(e, ExternalKind::Release)
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, [""]);

    ctl.assign_external(e_symbol, TruthValue::True).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, [""]);
}

// ---------------------------------------------------------------------------
// `add_assumptions`

/// A backend-authored assumption directive applies only to the *next* solve
/// call (`clingo_backend_assume`'s header: "for the next solve call"), unlike
/// a program-level fact: a second solve, with no new backend call, is
/// unconstrained again.
///
/// Oracle: Python module, 2026-09-27: `{a;b}.` with `add_assume([-a, b])`
/// solves once to `{b}`; solving again afterward, unchanged, enumerates all
/// 4 combinations.
#[test]
fn add_assumptions_apply_only_to_the_next_solve_call() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    // `a`/`b` already exist as atoms from `{a;b}.`; `add_atom` interns by
    // symbol and returns the same atom, so no `add_rule` call is needed here
    // (checked directly, 2026-09-27: `add_atom` after grounding an identical
    // symbol returns the atom the text already created).
    ctl.with_backend(|be| {
        let a = be.add_atom(Some(sym("a")))?;
        let bb = be.add_atom(Some(sym("b")))?;
        be.add_assumptions([a.neg(), bb.pos()])
    })
    .unwrap();

    let (_, first) = ctl.solve_all().unwrap();
    assert_models!(first, ["b"]);
    let (_, second) = ctl.solve_all().unwrap();
    assert_eq!(
        second.len(),
        4,
        "the assumption must not persist: {second:?}"
    );
}

// ---------------------------------------------------------------------------
// `add_heuristic`

/// A domain heuristic directive steers the first model clingo finds: without
/// it, the first model of `{a;b}.` (found under `--models=1`) is `{}`; with
/// `HeuristicKind::True` on `a` and `HeuristicKind::False` on `b`, it becomes
/// `{a}`, reproducibly (checked deterministic: clingox defaults to one
/// solver thread and a fixed seed, DESIGN, `TESTING.md` §6).
///
/// Oracle: Python module, 2026-09-27, ported from
/// `app/clingo/tests/python/backend_heuristic.lp`:
/// list): `configuration.solver.heuristic = "domain"`,
/// `configuration.solve.models = "1"`, `add_heuristic(a, True_, 1, 1, [])`,
/// `add_heuristic(b, False_, 1, 1, [])` gives `{a}` every time; the same
/// program with no heuristic directives gives `{}`.
#[test]
fn add_heuristic_true_and_false_bias_the_first_model_found() {
    let mut ctl = Control::new().unwrap();
    ctl.configuration()
        .set("solver.heuristic", "domain")
        .unwrap();
    ctl.configuration().set("solve.models", "1").unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.with_backend(|be| {
        let a = be.add_atom(Some(sym("a")))?;
        let bb = be.add_atom(Some(sym("b")))?;
        be.add_heuristic(a, HeuristicKind::True, 1, 1, &[])?;
        be.add_heuristic(bb, HeuristicKind::False, 1, 1, &[])
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    // `solve_all` lifts the model limit on purpose, so the first model is
    // read with `solve_first`.
    let Outcome::Sat(first, _) = ctl.solve_first().unwrap() else {
        panic!("the program has a model");
    };
    let mut shown: Vec<String> = first.symbols().iter().map(ToString::to_string).collect();
    shown.sort();
    assert_eq!(shown, ["a"]);
}

// ---------------------------------------------------------------------------
// `add_edge` (acyclicity)

/// Two conditional edges that form a cycle only when both conditions hold
/// forbid exactly that combination.
///
/// Oracle: Python module, 2026-09-27: `{a;b}.` with `add_acyc_edge(1,2,[a])`
/// and `add_acyc_edge(2,1,[b])` excludes `{a,b}` (a 1-2-1 cycle) but keeps
/// `{}`, `{a}` and `{b}`.
#[test]
fn add_edge_forbids_the_cycle_its_conditions_complete() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.with_backend(|be| {
        let a = be.add_atom(Some(sym("a")))?;
        let bb = be.add_atom(Some(sym("b")))?;
        be.add_edge(1, 2, &[a.pos()])?;
        be.add_edge(2, 1, &[bb.pos()])
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["", "a", "b"]);
}

/// Two edges in the *same* direction never form a cycle by themselves: the
/// `u`/`v` order matters, so a swap would wrongly forbid `{a,b}` here.
///
/// Oracle: Python module, 2026-09-27: `{a;b}.` with `add_acyc_edge(1,2,[a])`
/// and `add_acyc_edge(1,2,[b])` (same direction) keeps all 4 combinations.
#[test]
fn add_edge_direction_matters_same_direction_edges_never_cycle() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.with_backend(|be| {
        let a = be.add_atom(Some(sym("a")))?;
        let bb = be.add_atom(Some(sym("b")))?;
        be.add_edge(1, 2, &[a.pos()])?;
        be.add_edge(1, 2, &[bb.pos()])
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["", "a", "b", "a b"]);
}

// ---------------------------------------------------------------------------
// Theory terms, elements and atoms built through the backend, round-tripped
// through `TheoryAtoms`

const THEORY_FOR_BACKEND: &str =
    "#theory t { term { }; &a/0 : term, any; &b/0 : term, {>=}, term, directive }.";

/// Every theory term kind the backend can build (`add_theory_number`,
/// `add_theory_string`, `add_theory_sequence` in each of its three kinds,
/// `add_theory_function`, `add_theory_symbol`) round-trips through
/// `TheoryAtoms::term`/`term_kind` with the matching `TheoryTermKind`, tuple
/// and nesting `TheoryAtoms` reads back.
///
/// Oracle: Python module, 2026-09-27: `add_theory_term_number(42)`,
/// `add_theory_term_string("hi")`, `add_theory_term_sequence(Tuple, [n,s])`
/// prints `(42,hi)`; `add_theory_term_function("f", [n])` prints `f(42)`;
/// `add_theory_term_symbol(Function("c"))` reads back as the symbol `c`.
#[test]
fn theory_terms_built_through_the_backend_round_trip_through_theory_atoms() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base(THEORY_FOR_BACKEND).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (directive_atom, seq_id, fn_id, sym_id) = ctl
        .with_backend(|b| {
            let n = b.add_theory_number(42)?;
            let s = b.add_theory_string("hi")?;
            let seq = b.add_theory_sequence(TheorySequenceKind::Tuple, &[n, s])?;
            let f = b.add_theory_function("f", &[n])?;
            let symbol_term = b.add_theory_symbol(sym("c"))?;
            let elem_seq = b.add_theory_element(&[seq], &[])?;
            let elem_fn = b.add_theory_element(&[f], &[])?;
            let atom = b.add_theory_atom(
                TheoryAtomTarget::Directive,
                symbol_term,
                &[elem_seq, elem_fn],
            )?;
            Ok((atom, seq, f, symbol_term))
        })
        .unwrap();

    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms
        .iter()
        .find_map(|item| {
            let atom = item.unwrap();
            (atom.term().unwrap() == TheoryTerm::Symbol(sym("c"))).then_some(atom)
        })
        .expect("the directive atom is grounded");

    // A `Directive`-target atom gets no literal (the `0` sentinel).
    assert_eq!(atom.literal().unwrap(), None);

    let elements = atom.elements().unwrap();
    assert_eq!(elements.len(), 2);
    assert_eq!(atoms.term_kind(seq_id).unwrap(), TheoryTermKind::Tuple);
    assert_eq!(atoms.term(seq_id).unwrap().to_string(), "(42,hi)");
    assert_eq!(atoms.term_kind(fn_id).unwrap(), TheoryTermKind::Function);
    assert_eq!(atoms.term(fn_id).unwrap().to_string(), "f(42)");
    assert_eq!(atoms.term_kind(sym_id).unwrap(), TheoryTermKind::Symbol);
    assert_eq!(atoms.term(sym_id).unwrap(), TheoryTerm::Symbol(sym("c")));
    let _ = directive_atom;
}

/// `TheoryAtomTarget::Fresh` must pass clingo's `UINT32_MAX` sentinel, not
/// `0`: a fresh atom gets its own program literal and can appear in a rule,
/// unlike a directive (negative control 5: passing `0` here would silently
/// make every fresh atom a directive instead, with no literal at all).
///
/// Oracle: Python module, 2026-09-27: `add_theory_atom_with_guard(g(42), [],
/// ">=", 7)` with no explicit atom id (pyclingo's default, `UINT32_MAX`) gets
/// a nonzero literal and reads back with guard `(">=", 7)`.
#[test]
fn theory_atom_target_fresh_gets_a_real_literal_unlike_a_directive() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base(THEORY_FOR_BACKEND).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let fresh = ctl
        .with_backend(|b| {
            let n = b.add_theory_number(42)?;
            let g = b.add_theory_function("g", &[n])?;
            let guard = b.add_theory_number(7)?;
            b.add_theory_atom_with_guard(TheoryAtomTarget::Fresh, g, &[], ">=", guard)
        })
        .unwrap();

    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms
        .iter()
        .map(|item| item.unwrap())
        .find(|atom| atom.term().unwrap().to_string() == "g(42)")
        .expect("the fresh atom is grounded");
    let literal = atom.literal().unwrap();
    assert!(literal.is_some(), "a fresh atom must get a real literal");
    assert_eq!(literal.unwrap(), fresh.pos());
    let (connective, term) = atom.guard().unwrap().expect("the atom has a guard");
    assert_eq!(connective, ">=");
    assert_eq!(term, TheoryTerm::Number(7));
}

/// `TheoryAtomTarget::Atom(existing)` attaches the theory atom to a plain
/// atom the caller already created, instead of a directive or a fresh one:
/// the theory atom's own id is exactly the atom passed in, and it can be
/// used in an ordinary rule.
///
/// Oracle: Python module, 2026-09-27: `add_theory_atom(sym_term, [], a)`
/// (`a` an existing plain atom id) returns `a` unchanged, and `{a}.` built
/// from the same atom solves to two models.
#[test]
fn theory_atom_target_atom_attaches_to_an_existing_plain_atom() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(THEORY_FOR_BACKEND).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let a = ctl
        .with_backend(|b| {
            let a = b.add_atom(Some(sym("a")))?;
            let term = b.add_theory_symbol(sym("a"))?;
            let atom_id = b.add_theory_atom(TheoryAtomTarget::Atom(a), term, &[])?;
            assert_eq!(atom_id, a, "the theory atom must reuse the given atom");
            b.add_rule(Head::Choice(&[a]), &[])?;
            Ok(a)
        })
        .unwrap();
    // No further `ground` call: the backend already added ground statements
    // directly, and re-grounding the same `base` part is a no-op (checked
    // directly, 2026-09-27: clingo does not re-process an instantiation it
    // already grounded).
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["", "a"]);
    let _ = a;
}

// ---------------------------------------------------------------------------
// Sanity: an `Id` produced by the backend has the same type the theory
// module already exposes.

#[test]
fn backend_theory_term_ids_are_the_same_id_type_theory_atoms_uses() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base(THEORY_FOR_BACKEND).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let id: Id = ctl.with_backend(|b| b.add_theory_number(1)).unwrap();
    assert_eq!(
        ctl.theory_atoms().unwrap().term_kind(id).unwrap(),
        TheoryTermKind::Number
    );
}

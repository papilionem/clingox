//! `Model` extras: the model kind, three-valued consequence
//! checking, and the priority levels behind a model's cost.
//!
//! Expected values were checked against the Python module `clingo` 5.8.2,
//! since neither `clingo.h` nor the design pins the exact
//! partial-information sequence clingo reports while enumerating brave or
//! cautious consequences.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail loudly on unexpected errors"
)]

use clingox::{Consequence, Control, ModelKind, Part, ProgramLiteral, Symbol};

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// Parses an atom from its text, such as `q` or `p(1)`.
fn atom(text: &str) -> Symbol {
    text.parse().expect("the text is a valid term")
}

/// The program literal of an atom already in the grounding.
fn literal_of(ctl: &Control, name: &str) -> ProgramLiteral {
    ctl.symbolic_atoms()
        .expect("the grounding can be read")
        .find(atom(name))
        .expect("the lookup succeeds")
        .expect("the atom is in the grounding")
        .literal()
}

fn set_enum_mode(ctl: &mut Control, mode: &str) {
    ctl.configuration()
        .set("solve.enum_mode", mode)
        .expect("enum_mode is a valid path and value");
}

// ---------------------------------------------------------------------------
// ModelKind

#[test]
fn kind_is_stable_model_for_an_ordinary_solve() {
    let mut ctl = grounded(&[], "a.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle.next_model().unwrap().expect("`a.` has a model");
    match model.kind().unwrap() {
        ModelKind::StableModel => {}
        other => panic!("expected StableModel, got {other:?}"),
    }
}

/// `1{a;b}1. c.` has two answer sets, `{a,c}` and `{b,c}`; brave enumeration
/// under `--enum-mode=brave` reports two running unions, both typed
/// `BraveConsequences` (checked against clingo 5.8.2: `ctl.solve(yield_=True)`
/// under `configuration.solve.enum_mode = "brave"` yields exactly two models).
const ONE_OF_TWO_PLUS_FACT: &str = "1{a;b}1. c.";

#[test]
fn kind_is_brave_consequences_under_enum_mode_brave() {
    let mut ctl = grounded(&["--models=0"], ONE_OF_TWO_PLUS_FACT);
    set_enum_mode(&mut ctl, "brave");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut count = 0;
    while let Some(model) = handle.next_model().unwrap() {
        match model.kind().unwrap() {
            ModelKind::BraveConsequences => {}
            other => panic!("expected BraveConsequences, got {other:?}"),
        }
        count += 1;
    }
    assert_eq!(
        count, 2,
        "clingo 5.8.2 reports two brave models for this fixture"
    );
}

#[test]
fn kind_is_cautious_consequences_under_enum_mode_cautious() {
    let mut ctl = grounded(&["--models=0"], ONE_OF_TWO_PLUS_FACT);
    set_enum_mode(&mut ctl, "cautious");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut count = 0;
    while let Some(model) = handle.next_model().unwrap() {
        match model.kind().unwrap() {
            ModelKind::CautiousConsequences => {}
            other => panic!("expected CautiousConsequences, got {other:?}"),
        }
        count += 1;
    }
    assert_eq!(
        count, 2,
        "clingo 5.8.2 reports two cautious models for this fixture"
    );
}

// ---------------------------------------------------------------------------
// is_consequence

/// Outside brave and cautious enumeration, `is_consequence` on every atom of
/// every model agrees with `is_true` on the same literal, and is never
/// `Unknown` (the header's own fallback: "the function just returns whether a
/// literal is true or false in the current model"). Checked against clingo
/// 5.8.2 for this fixture's two models (`{p(1),p(2)}` and `{p(1),p(2),q}`).
#[test]
fn is_consequence_matches_is_true_outside_brave_and_cautious() {
    let mut ctl = grounded(&["--models=0"], "p(1). p(2). {q}.");
    let literals: Vec<ProgramLiteral> = ["p(1)", "p(2)", "q"]
        .iter()
        .map(|name| literal_of(&ctl, name))
        .collect();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut models_seen = 0;
    while let Some(model) = handle.next_model().unwrap() {
        for &literal in &literals {
            let is_true = model.is_true(literal).unwrap();
            match model.is_consequence(literal).unwrap() {
                Consequence::True => assert!(is_true, "is_consequence(True) but is_true is false"),
                Consequence::False => {
                    assert!(!is_true, "is_consequence(False) but is_true is true");
                }
                Consequence::Unknown => {
                    panic!("is_consequence must not be Unknown outside brave/cautious enumeration")
                }
            }
        }
        models_seen += 1;
    }
    assert_eq!(models_seen, 2, "`{{q}}` gives two models");
}

/// The atoms of `1{a;b}1. c. {d}. :- d.`: `a`/`b` alternate, `c` is a fact in
/// every model, `d` is never true in any answer set. Shared by the brave and
/// cautious partial-information tests below.
const PARTIAL_INFO_PROGRAM: &str = "1{a;b}1. c. {d}. :- d.";

/// Brave consequences of [`PARTIAL_INFO_PROGRAM`], transcribed from clingo
/// 5.8.2 under `--enum-mode=brave` (`ctl.solve(yield_=True)`, one row per
/// model in enumeration order, `(a, b, c, d)`):
///
/// | model | shown  | a    | b    | c    | d     |
/// |---|---|---|---|---|---|
/// | 1 | `a c`   | True | **Unknown** | True | False |
/// | 2 | `a b c` | True | True | True | False |
///
/// `b` is unresolved while it has not yet appeared in any enumerated model,
/// but `d`, which appears in none, is `False` immediately: brave enumeration
/// resolves "never true" to `False` as soon as it is known, not only once
/// every model has been seen.
#[test]
fn is_consequence_tracks_partial_information_during_brave_enumeration() {
    let mut ctl = grounded(&["--models=0"], PARTIAL_INFO_PROGRAM);
    set_enum_mode(&mut ctl, "brave");
    let (la, lb, lc, ld) = (
        literal_of(&ctl, "a"),
        literal_of(&ctl, "b"),
        literal_of(&ctl, "c"),
        literal_of(&ctl, "d"),
    );
    let expected = [
        (
            Consequence::True,
            Consequence::Unknown,
            Consequence::True,
            Consequence::False,
        ),
        (
            Consequence::True,
            Consequence::True,
            Consequence::True,
            Consequence::False,
        ),
    ];
    let mut handle = ctl.solve_yield(&[]).unwrap();
    for (index, expected_row) in expected.iter().enumerate() {
        let model = handle
            .next_model()
            .unwrap()
            .unwrap_or_else(|| panic!("model {} is missing", index + 1));
        let row = (
            model.is_consequence(la).unwrap(),
            model.is_consequence(lb).unwrap(),
            model.is_consequence(lc).unwrap(),
            model.is_consequence(ld).unwrap(),
        );
        assert_eq!(row, *expected_row, "model {}", index + 1);
    }
    assert!(
        handle.next_model().unwrap().is_none(),
        "exactly two brave models"
    );
}

/// Cautious consequences of [`PARTIAL_INFO_PROGRAM`], transcribed the same
/// way, under `--enum-mode=cautious`:
///
/// | model | shown | a | b | c | d |
/// |---|---|---|---|---|---|
/// | 1 | `a c` | **Unknown** | False | True | False |
/// | 2 | `c`   | False | False | True | False |
///
/// `b` and `d` are excluded from the very first model, so cautious
/// enumeration (an intersection) resolves them to `False` at once; `a`, which
/// the first model has, stays `Unknown` until the second model settles
/// whether every answer set agrees.
#[test]
fn is_consequence_tracks_partial_information_during_cautious_enumeration() {
    let mut ctl = grounded(&["--models=0"], PARTIAL_INFO_PROGRAM);
    set_enum_mode(&mut ctl, "cautious");
    let (la, lb, lc, ld) = (
        literal_of(&ctl, "a"),
        literal_of(&ctl, "b"),
        literal_of(&ctl, "c"),
        literal_of(&ctl, "d"),
    );
    let expected = [
        (
            Consequence::Unknown,
            Consequence::False,
            Consequence::True,
            Consequence::False,
        ),
        (
            Consequence::False,
            Consequence::False,
            Consequence::True,
            Consequence::False,
        ),
    ];
    let mut handle = ctl.solve_yield(&[]).unwrap();
    for (index, expected_row) in expected.iter().enumerate() {
        let model = handle
            .next_model()
            .unwrap()
            .unwrap_or_else(|| panic!("model {} is missing", index + 1));
        let row = (
            model.is_consequence(la).unwrap(),
            model.is_consequence(lb).unwrap(),
            model.is_consequence(lc).unwrap(),
            model.is_consequence(ld).unwrap(),
        );
        assert_eq!(row, *expected_row, "model {}", index + 1);
    }
    assert!(
        handle.next_model().unwrap().is_none(),
        "exactly two cautious models"
    );
}

// ---------------------------------------------------------------------------
// priorities

fn only_model_priorities(program: &str) -> Vec<i32> {
    let mut ctl = grounded(&[], program);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle
        .next_model()
        .unwrap()
        .expect("the program has a model");
    let priorities = model.priorities().unwrap();
    assert!(
        handle.next_model().unwrap().is_none(),
        "the fixture is not meant to have a second model"
    );
    priorities
}

/// Checked against clingo 5.8.2:
/// `ctl.solve(yield_=True)`'s only model has `cost == [1, 1]` and
/// `priority == [2, 1]`: level 2 (from `b`'s `[1@2]`) outranks level 1 (from
/// `a`'s `[1@1]`), matching `cost()`'s own highest-priority-first order.
#[test]
fn priorities_are_listed_highest_priority_first() {
    let priorities = only_model_priorities("a. b. :~ a. [1@1] :~ b. [1@2]");
    assert_eq!(priorities, [2, 1]);
}

/// A fixture whose cost values and priority levels are pairwise distinct
/// (`cost == [3, 7]`, `priority == [5, 1]`, both checked against clingo
/// 5.8.2), so a bug that reports `cost()`'s values instead of the levels, or
/// that reads them in the wrong order, cannot pass this test by accident.
#[test]
fn priorities_use_the_actual_level_numbers_not_the_cost_values() {
    let mut ctl = grounded(&[], "a. b. :~ a. [3@5] :~ b. [7@1]");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle
        .next_model()
        .unwrap()
        .expect("the program has a model");
    assert_eq!(model.cost().unwrap(), [3, 7]);
    assert_eq!(model.priorities().unwrap(), [5, 1]);
}

/// Mirrors `a_model_without_optimisation_has_no_cost` (`api_model.rs`):
/// without optimisation statements, `priorities()` is empty, exactly as
/// `cost()` already is.
#[test]
fn priorities_is_empty_without_optimisation_statements() {
    let priorities = only_model_priorities("a.");
    assert_eq!(priorities, []);
}

//! Signed program literals: `ProgramLiteral::is_positive`/`negate`/`Neg`, the
//! literal form of `Assumption`, and `Control::assign_external_literal`/
//! `release_external_literal`.
//!
//! Expected values were checked against clingo 5.8.2, including the
//! negative-literal cases and the `From<i32> for ProgramLiteral` conversion.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use clingox::{Control, ErrorKind, Part, ProgramLiteral, Symbol, TruthValue};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a control can be created");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn constant(name: &str) -> Symbol {
    Symbol::function(name, &[]).expect("the name has no NUL byte")
}

fn literal_of(ctl: &Control, symbol: Symbol) -> ProgramLiteral {
    ctl.symbolic_atoms()
        .unwrap()
        .find(symbol)
        .unwrap()
        .expect("the symbol is a current atom")
        .literal()
}

/// `e` is external, `a` follows it and `b` its negation; `fact` is an
/// ordinary atom. Matches `api_externals.rs`'s own fixture.
const PROGRAM: &str = "#external e. a :- e. b :- not e. fact.";

fn answer_sets(ctl: &mut Control) -> Vec<Vec<String>> {
    let (result, models) = ctl.solve_all().expect("the search succeeds");
    assert!(result.is_exhausted(), "{result:?}");
    models
        .iter()
        .map(|m| m.symbols().iter().map(ToString::to_string).collect())
        .collect()
}

fn sets(sets: &[&[&str]]) -> Vec<Vec<String>> {
    sets.iter()
        .map(|set| set.iter().map(|s| (*s).to_owned()).collect())
        .collect()
}

// ---------------------------------------------------------------------------
// `ProgramLiteral::is_positive`, `negate`, `Neg`
// ---------------------------------------------------------------------------

#[test]
fn a_symbolic_atoms_literal_is_positive() {
    let ctl = grounded("a.");
    let literal = literal_of(&ctl, constant("a"));
    assert!(literal.is_positive());
}

#[test]
fn negate_flips_the_sign_and_is_its_own_inverse() {
    let ctl = grounded("a.");
    let literal = literal_of(&ctl, constant("a"));
    let negated = literal.negate();
    assert!(!negated.is_positive());
    assert_eq!(negated.get(), -literal.get());
    assert_eq!(negated.negate(), literal);
}

#[test]
fn neg_operator_matches_negate() {
    let ctl = grounded("a.");
    let literal = literal_of(&ctl, constant("a"));
    assert_eq!(-literal, literal.negate());
    assert_eq!(-(-literal), literal);
}

// ---------------------------------------------------------------------------
// Literal `Assumption`
// ---------------------------------------------------------------------------

#[test]
fn a_positive_literal_assumption_matches_the_true_symbol_assumption() {
    let mut ctl = grounded("a :- not b. b :- not a.");
    let a = constant("a");
    let literal = literal_of(&ctl, a);

    let by_literal = ctl.solve(&[literal.into()]).unwrap();
    let by_symbol = ctl.solve(&[(a, true).into()]).unwrap();
    assert_eq!(by_literal.is_sat(), by_symbol.is_sat());
    assert!(by_literal.is_sat());
}

#[test]
fn a_negated_literal_assumption_matches_the_false_symbol_assumption() {
    let mut ctl = grounded("a :- not b. b :- not a.");
    let a = constant("a");
    let literal = literal_of(&ctl, a);

    // `.negate()` and the `Neg` operator both produce the same sign flip:
    // exercise both here.
    let by_negate = ctl.solve(&[literal.negate().into()]).unwrap();
    let by_neg_operator = ctl.solve(&[(-literal).into()]).unwrap();
    let by_symbol = ctl.solve(&[(a, false).into()]).unwrap();
    assert_eq!(by_negate.is_sat(), by_symbol.is_sat());
    assert_eq!(by_neg_operator.is_sat(), by_symbol.is_sat());
}

/// Negative control #4: on a plain fact, the two signs of the same literal
/// give opposite, unambiguous results (assuming a fact false is
/// unsatisfiable, unlike the `a :- not b. b :- not a.` fixture above, where
/// both signs happen to be individually satisfiable and so cannot catch a
/// mutant that drops the sign).
#[test]
fn the_sign_of_a_literal_assumption_changes_the_result_on_a_fact() {
    let mut ctl = grounded("a.");
    let literal = literal_of(&ctl, constant("a"));

    assert!(ctl.solve(&[literal.into()]).unwrap().is_sat());
    assert!(ctl.solve(&[(-literal).into()]).unwrap().is_unsat());
}

// ---------------------------------------------------------------------------
// `assign_external_literal`/`release_external_literal`
// ---------------------------------------------------------------------------

#[test]
fn assign_external_literal_matches_the_symbol_form() {
    let mut by_literal = grounded(PROGRAM);
    let e_literal = literal_of(&by_literal, constant("e"));
    by_literal
        .assign_external_literal(e_literal, TruthValue::True)
        .unwrap();

    let mut by_symbol = grounded(PROGRAM);
    by_symbol
        .assign_external(constant("e"), TruthValue::True)
        .unwrap();

    assert_eq!(answer_sets(&mut by_literal), answer_sets(&mut by_symbol));
    assert_eq!(answer_sets(&mut by_literal), sets(&[&["a", "e", "fact"]]));
}

#[test]
fn release_external_literal_matches_the_symbol_form() {
    let mut by_literal = grounded(PROGRAM);
    let e_literal = literal_of(&by_literal, constant("e"));
    by_literal
        .assign_external_literal(e_literal, TruthValue::True)
        .unwrap();
    by_literal.release_external_literal(e_literal).unwrap();

    let mut by_symbol = grounded(PROGRAM);
    by_symbol
        .assign_external(constant("e"), TruthValue::True)
        .unwrap();
    by_symbol.release_external(constant("e")).unwrap();

    assert_eq!(answer_sets(&mut by_literal), answer_sets(&mut by_symbol));
    assert_eq!(answer_sets(&mut by_literal), sets(&[&["b", "fact"]]));
}

#[test]
fn assign_external_literal_on_a_literal_with_no_symbol_behind_it_is_a_no_op() {
    // No backend-made aux atom is used here, so this uses a literal one past
    // the highest one any current atom has. Verified against clingo 5.8.2: the
    // model is unaffected and no error is raised.
    let mut ctl = grounded("a. b :- a.");
    let max = ctl
        .symbolic_atoms()
        .unwrap()
        .iter()
        .map(|atom| atom.unwrap().literal().get())
        .max()
        .expect("the program has at least one atom");
    let unused = ProgramLiteral::from_raw(max + 1).unwrap();

    ctl.assign_external_literal(unused, TruthValue::True)
        .unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["a", "b"]]));
}

#[test]
fn a_negative_literal_passed_to_assign_external_literal_is_invalid_input() {
    let mut ctl = grounded(PROGRAM);
    let e_literal = literal_of(&ctl, constant("e"));

    let err = ctl
        .assign_external_literal(-e_literal, TruthValue::True)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");

    // Does not poison: the control is still usable, and the external is
    // still unassigned (the rejected call never reached clingo).
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert_eq!(answer_sets(&mut ctl), sets(&[&["b", "fact"]]));
}

#[test]
fn release_external_literal_accepts_a_negative_literal() {
    // Unlike `assign_external_literal`, `clingo_control_release_external`'s
    // own doc says a negative literal just releases the same atom (no truth
    // value to double-negate), so clingox does not reject it.
    let mut ctl = grounded(PROGRAM);
    let e_literal = literal_of(&ctl, constant("e"));
    ctl.assign_external_literal(e_literal, TruthValue::True)
        .unwrap();
    ctl.release_external_literal(-e_literal).unwrap();

    assert_eq!(answer_sets(&mut ctl), sets(&[&["b", "fact"]]));
}

// 0 is not a literal in clingo's encoding: the sign carries the truth value,
// so there is no zero atom (clingo.h, `clingo_literal_t`).
#[test]
fn from_raw_rejects_zero_and_keeps_the_sign() {
    assert!(ProgramLiteral::from_raw(0).is_none());
    let positive = ProgramLiteral::from_raw(7).unwrap();
    assert!(positive.is_positive());
    assert_eq!(positive.get(), 7);
    let negative = ProgramLiteral::from_raw(-7).unwrap();
    assert!(!negative.is_positive());
    assert_eq!(negative, -positive);
}

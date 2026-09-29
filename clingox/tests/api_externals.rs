//! Externals and multi-shot solving: assigning, freeing and releasing external
//! atoms, and grounding more parts between solves.
//!
//! Expected values were checked against the Python module `clingo` 5.8.2.

#![forbid(unsafe_code)]

use std::fmt::Debug;
use std::hash::Hash;

use clingox::{Control, ErrorKind, Outcome, Part, Symbol, TruthValue};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a control can be created");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn constant(name: &str) -> Symbol {
    Symbol::function(name, &[]).expect("the name has no NUL byte")
}

fn function(name: &str, argument: i32) -> Symbol {
    Symbol::function(name, &[Symbol::number(argument)]).expect("the name has no NUL byte")
}

/// Every answer set, each as the sorted texts of its shown symbols. `solve_all`
/// sorts both levels, so the result does not depend on the search order.
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

/// `e` is external, `a` follows it and `b` its negation; `fact` is an ordinary
/// atom.
const PROGRAM: &str = "#external e. a :- e. b :- not e. fact.";

#[test]
fn an_external_is_false_until_assigned() {
    let mut ctl = grounded(PROGRAM);
    assert_eq!(answer_sets(&mut ctl), sets(&[&["b", "fact"]]));
}

#[test]
fn assigning_true_makes_an_external_true() {
    let mut ctl = grounded(PROGRAM);
    ctl.assign_external(constant("e"), TruthValue::True)
        .unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["a", "e", "fact"]]));
}

#[test]
fn a_free_external_can_be_true_or_false() {
    let mut ctl = grounded(PROGRAM);
    ctl.assign_external(constant("e"), TruthValue::Free)
        .unwrap();
    assert_eq!(
        answer_sets(&mut ctl),
        sets(&[&["a", "e", "fact"], &["b", "fact"]])
    );
}

#[test]
fn assignments_can_be_changed_between_solves() {
    let mut ctl = grounded(PROGRAM);
    ctl.assign_external(constant("e"), TruthValue::True)
        .unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["a", "e", "fact"]]));
    ctl.assign_external(constant("e"), TruthValue::False)
        .unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["b", "fact"]]));
}

#[test]
fn an_assignment_persists_across_solves() {
    let mut ctl = grounded(PROGRAM);
    ctl.assign_external(constant("e"), TruthValue::True)
        .unwrap();
    for _ in 0..3 {
        assert_eq!(answer_sets(&mut ctl), sets(&[&["a", "e", "fact"]]));
    }
}

#[test]
fn a_declared_value_sets_the_initial_truth() {
    let mut ctl = grounded("#external e. [true] a :- e.");
    assert_eq!(answer_sets(&mut ctl), sets(&[&["a", "e"]]));
}

#[test]
fn assign_external_ignores_symbols_that_are_not_externals() {
    let mut ctl = grounded(PROGRAM);
    ctl.assign_external(constant("fact"), TruthValue::False)
        .unwrap();
    ctl.assign_external(constant("nosuch"), TruthValue::True)
        .unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["b", "fact"]]));
}

#[test]
fn try_assign_external_assigns_an_external() {
    let mut ctl = grounded(PROGRAM);
    ctl.try_assign_external(constant("e"), TruthValue::True)
        .unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["a", "e", "fact"]]));
    ctl.try_assign_external(constant("e"), TruthValue::Free)
        .unwrap();
    assert_eq!(
        answer_sets(&mut ctl),
        sets(&[&["a", "e", "fact"], &["b", "fact"]])
    );
}

#[test]
fn try_assign_external_rejects_a_symbol_that_is_not_an_external() {
    let mut ctl = grounded(PROGRAM);
    for name in ["fact", "nosuch"] {
        let err = ctl
            .try_assign_external(constant(name), TruthValue::True)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime, "{err}");
        assert!(
            err.to_string().contains(name),
            "the message names it: {err}"
        );
    }
    // The error does not poison the control.
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert_eq!(answer_sets(&mut ctl), sets(&[&["b", "fact"]]));
}

#[test]
fn releasing_an_external_makes_it_permanently_false() {
    let mut ctl = grounded(PROGRAM);
    ctl.assign_external(constant("e"), TruthValue::True)
        .unwrap();
    ctl.release_external(constant("e")).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["b", "fact"]]));

    // It is no longer an external: assigning it is ignored or rejected.
    ctl.assign_external(constant("e"), TruthValue::True)
        .unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["b", "fact"]]));
    let err = ctl
        .try_assign_external(constant("e"), TruthValue::True)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
}

#[test]
fn release_external_ignores_symbols_that_are_not_externals() {
    let mut ctl = grounded(PROGRAM);
    ctl.release_external(constant("fact")).unwrap();
    ctl.release_external(constant("nosuch")).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["b", "fact"]]));
}

#[test]
fn externals_drive_multi_shot_solving() {
    // Each step adds `query(t)`, and only the newest query holds.
    let mut ctl = Control::new().unwrap();
    ctl.add("step", &["t"], "#external query(t). x(t) :- query(t).")
        .unwrap();
    for t in 1..=3 {
        ctl.ground(&[Part::new("step", &[Symbol::number(t)]).unwrap()])
            .unwrap();
        if t > 1 {
            ctl.assign_external(function("query", t - 1), TruthValue::False)
                .unwrap();
        }
        ctl.assign_external(function("query", t), TruthValue::True)
            .unwrap();
        let Outcome::Sat(model, _) = ctl.solve_first().unwrap() else {
            panic!("step {t} is satisfiable");
        };
        assert_eq!(
            model.symbols(),
            [function("query", t), function("x", t)],
            "step {t}"
        );
        assert_eq!(answer_sets(&mut ctl).len(), 1, "step {t} has one model");
    }
}

#[test]
fn grounding_more_parts_between_solves_extends_the_program() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(1).").unwrap();
    ctl.add("more", &[], "p(2). {q}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["p(1)"]]));
    ctl.ground(&[Part::new("more", &[]).unwrap()]).unwrap();
    assert_eq!(
        answer_sets(&mut ctl),
        // Sorted in clingo's order, where the constant `q` precedes `p(1)`.
        sets(&[&["q", "p(1)", "p(2)"], &["p(1)", "p(2)"]])
    );
}

#[test]
fn an_external_keeps_its_value_when_more_parts_are_grounded() {
    let mut ctl = grounded("#external e.");
    ctl.assign_external(constant("e"), TruthValue::True)
        .unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["e"]]));
    ctl.add("more", &[], "b :- e.").unwrap();
    ctl.ground(&[Part::new("more", &[]).unwrap()]).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["b", "e"]]));
}

#[test]
fn external_calls_refuse_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let e = constant("e");
    let kinds = [
        ctl.assign_external(e, TruthValue::True).unwrap_err().kind(),
        ctl.try_assign_external(e, TruthValue::True)
            .unwrap_err()
            .kind(),
        ctl.release_external(e).unwrap_err().kind(),
    ];
    assert_eq!(kinds, [ErrorKind::Poisoned; 3]);
}

#[test]
fn truth_values_have_the_expected_traits() {
    fn value_traits<T: Copy + Eq + Hash + Debug + Send + Sync + 'static>() {}
    value_traits::<TruthValue>();
    assert_ne!(TruthValue::True, TruthValue::False);
    assert_ne!(TruthValue::Free, TruthValue::False);
}

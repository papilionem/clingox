//! Multi-shot control: `cleanup`, `enable_cleanup`, `remove_minimize`,
//! `update_project`, `is_conflicting`, `enable_enumeration_assumption` and
//! `get_const`.
//!
//! Expected values were checked against clingo 5.8.2 (the Python module).
//! The harder cases (`cleanup`, `update_project`, `get_const`) were built
//! from oracle sessions.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::float_cmp,
    reason = "clingo's statistics are whole numbers stored as doubles, so exact comparison is meant"
)]

use clingox::{Control, Outcome, Part, Symbol, TruthValue};

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

fn atoms_stat(ctl: &Control) -> f64 {
    ctl.statistics()
        .expect("statistics can be read")
        .value("problem.lp.atoms")
        .expect("the path names a value")
}

// ---------------------------------------------------------------------------
// `cleanup`, `set_enable_cleanup`/`enable_cleanup`
// ---------------------------------------------------------------------------

#[test]
fn enable_cleanup_defaults_to_true() {
    let ctl = Control::new().unwrap();
    assert!(ctl.enable_cleanup());
}

#[test]
fn set_enable_cleanup_round_trips() {
    let mut ctl = Control::new().unwrap();
    ctl.set_enable_cleanup(false).unwrap();
    assert!(!ctl.enable_cleanup());
    ctl.set_enable_cleanup(true).unwrap();
    assert!(ctl.enable_cleanup());
}

#[test]
fn cleanup_is_callable_manually_with_automatic_cleanup_disabled() {
    let mut ctl = grounded("{a}.");
    ctl.set_enable_cleanup(false).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
    ctl.cleanup().unwrap();
    // The control is still usable afterward.
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_manual_cleanup_after_a_solve_is_a_no_op_when_cleanup_stays_enabled() {
    // Cleanup is enabled by default (`clingo.h`,
    // `clingo_control_set_enable_cleanup`), so clingo already cleans up
    // automatically after `solve`; a further manual call changes nothing
    // observable on an already-clean program.
    let mut ctl = grounded("{a}.");
    assert!(ctl.solve(&[]).unwrap().is_sat());
    let before = atoms_stat(&ctl);
    ctl.cleanup().unwrap();
    assert_eq!(atoms_stat(&ctl), before);
}

/// Builds the multi-shot scenario checked
/// against the Python module:
/// records against the Python module: five externals, three permanently
/// falsified after one solve, then a further part whose rule depends on
/// their domain is grounded. With automatic cleanup disabled throughout,
/// `clean` calls `cleanup()` once before the further grounding; both start
/// from the identical state otherwise.
fn atoms_after_regrounding_with_falsified_atoms(clean: bool) -> f64 {
    let mut ctl = Control::new().unwrap();
    ctl.set_enable_cleanup(false).unwrap();
    ctl.add_base("#external p(1..5).").unwrap();
    ctl.add("step", &[], "q(X) :- p(X).").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    for i in 1..=5 {
        ctl.assign_external(function("p", i), TruthValue::True)
            .unwrap();
    }
    assert!(ctl.solve(&[]).unwrap().is_sat());
    for i in 1..=3 {
        ctl.release_external(function("p", i)).unwrap();
    }
    assert!(ctl.solve(&[]).unwrap().is_sat());
    if clean {
        ctl.cleanup().unwrap();
    }
    ctl.ground(&[Part::new("step", &[]).unwrap()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
    atoms_stat(&ctl)
}

#[test]
fn cleanup_before_regrounding_reduces_new_atoms() {
    // Verified against the Python module: 10 atoms without the manual
    // `cleanup()` call, 7 with it (5 externals plus 5 or 2 `q/1` atoms).
    let without = atoms_after_regrounding_with_falsified_atoms(false);
    let with = atoms_after_regrounding_with_falsified_atoms(true);
    assert_eq!(without, 10.0);
    assert_eq!(with, 7.0);
    assert!(
        with < without,
        "cleanup should shrink the further grounding"
    );
}

// ---------------------------------------------------------------------------
// `remove_minimize`
// ---------------------------------------------------------------------------

#[test]
fn remove_minimize_stops_solve_optimal_from_optimising() {
    let mut ctl = grounded("{ a; b; c }. :~ a. [1] :~ b. [2]");
    let Outcome::Sat(best, _) = ctl.solve_optimal().unwrap() else {
        panic!("the program has a model");
    };
    // Verified against the Python module: the optimal model has cost `[0]`
    // (neither `a` nor `b` holds).
    assert_eq!(best.cost(), [0]);

    ctl.remove_minimize().unwrap();
    let Outcome::Sat(after, _) = ctl.solve_optimal().unwrap() else {
        panic!("the program still has a model");
    };
    // No minimize statements are left, so there is nothing to optimise.
    assert!(after.cost().is_empty(), "{:?}", after.cost());
}

// ---------------------------------------------------------------------------
// `update_project`
// ---------------------------------------------------------------------------

/// Every answer set under `--project`, each as its sorted, shown symbols.
fn projected_answer_sets(ctl: &mut Control) -> Vec<Vec<String>> {
    let (result, models) = ctl.solve_all().expect("the search succeeds");
    assert!(result.is_exhausted(), "{result:?}");
    let mut sets: Vec<Vec<String>> = models
        .iter()
        .map(|m| {
            let mut v: Vec<String> = m.symbols().iter().map(ToString::to_string).collect();
            v.sort();
            v
        })
        .collect();
    sets.sort();
    sets
}

#[test]
fn update_project_replace_restricts_enumeration_to_the_given_atoms() {
    // Verified against the Python module: `{a}. {b}. {c}.` under `--project`
    // enumerates 8 models with no projection set, 2 once projected onto `a`
    // alone, 4 once `b` is appended.
    let mut ctl = Control::with_args(["--project", "--models=0"]).unwrap();
    ctl.add_base("{a}. {b}. {c}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(projected_answer_sets(&mut ctl).len(), 8);

    ctl.update_project([constant("a")], false).unwrap();
    assert_eq!(projected_answer_sets(&mut ctl).len(), 2);

    ctl.update_project([constant("b")], true).unwrap();
    assert_eq!(projected_answer_sets(&mut ctl).len(), 4);
}

#[test]
fn update_project_ignores_a_symbol_that_is_not_a_current_atom() {
    let mut ctl = Control::with_args(["--project", "--models=0"]).unwrap();
    ctl.add_base("{a}. {b}. {c}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.update_project([constant("a"), Symbol::number(999)], false)
        .unwrap();
    assert_eq!(projected_answer_sets(&mut ctl).len(), 2);
}

// ---------------------------------------------------------------------------
// `is_conflicting`
// ---------------------------------------------------------------------------

#[test]
fn is_conflicting_is_false_on_a_fresh_satisfiable_program() {
    assert!(!grounded("a.").is_conflicting());
}

#[test]
fn is_conflicting_is_true_after_grounding_an_unsatisfiable_constraint() {
    assert!(grounded("a. :- a.").is_conflicting());
}

// ---------------------------------------------------------------------------
// `set_enable_enumeration_assumption`/`enable_enumeration_assumption`
// ---------------------------------------------------------------------------

#[test]
fn enable_enumeration_assumption_defaults_to_true() {
    let ctl = Control::new().unwrap();
    assert!(ctl.enable_enumeration_assumption());
}

#[test]
fn set_enable_enumeration_assumption_round_trips() {
    let mut ctl = Control::new().unwrap();
    ctl.set_enable_enumeration_assumption(false).unwrap();
    assert!(!ctl.enable_enumeration_assumption());
    ctl.set_enable_enumeration_assumption(true).unwrap();
    assert!(ctl.enable_enumeration_assumption());
}

// ---------------------------------------------------------------------------
// `get_const`
// ---------------------------------------------------------------------------

#[test]
fn get_const_returns_the_symbol_of_a_numeric_constant() {
    let ctl = grounded_with_consts();
    assert_eq!(ctl.get_const("x").unwrap(), Some(Symbol::number(1)));
}

#[test]
fn get_const_returns_the_symbol_of_a_non_numeric_constant() {
    let ctl = grounded_with_consts();
    assert_eq!(ctl.get_const("y").unwrap(), Some(constant("foo")));
    assert_eq!(
        ctl.get_const("z").unwrap(),
        Some(Symbol::tuple(&[Symbol::number(1), Symbol::number(2)]).unwrap())
    );
}

#[test]
fn get_const_returns_none_for_an_undefined_constant() {
    // Checked directly against clingo 5.8.2 with `ctypes`: neither
    // `clingo_control_has_const` nor `clingo_control_get_const` raises an
    // error for an undefined name in this build, unlike `clingo.h`'s own doc
    // comment for `has_const` claims. `get_const` alone (skipping
    // `has_const`) would silently return `Some(Symbol::function("nosuch",
    // &[]))` instead, which this assertion also catches (negative control
    // #3).
    let ctl = grounded_with_consts();
    assert_eq!(ctl.get_const("nosuch").unwrap(), None);
    assert_ne!(
        ctl.get_const("nosuch").unwrap(),
        Some(constant("nosuch")),
        "an undefined constant must not silently resolve to its own name"
    );
}

fn grounded_with_consts() -> Control {
    grounded("#const x = 1. #const y = foo. #const z = (1,2).")
}

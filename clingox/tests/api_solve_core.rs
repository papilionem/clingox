//! `SolveHandle::core`: the subset of assumptions clingo reports when a search
//! is unsatisfiable (`clingo_solve_handle_core`).
//!
//! `clingo.h` only documents the satisfiable case ("If the program is not
//! unsatisfiable, core is set to NULL and size to zero") and the error it can
//! raise (`clingo_error_bad_alloc`). It says nothing about the sign of the
//! returned literals, their order, whether the core is minimal, or what an
//! unfinished handle reports, so every one of those was checked against the
//! Python module `clingo` 5.8.2 rather than assumed.
//!
//! Only `SolveHandle` (the yield form) is tested here. The blocking
//! form (`Control::solve`) never exposes a handle at all, so `core` does not
//! apply to it.

#![forbid(unsafe_code)]

use clingox::{Assumption, Control, Part, ProgramLiteral, Symbol};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("the default arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn atom(name: &str) -> Symbol {
    Symbol::function(name, &[]).expect("the name has no NUL byte")
}

/// The program literal of a 0-arity atom already in the grounding.
fn literal_of(ctl: &Control, name: &str) -> ProgramLiteral {
    ctl.symbolic_atoms()
        .expect("the grounding can be read")
        .find(atom(name))
        .expect("the lookup succeeds")
        .expect("the atom is in the grounding")
        .literal()
}

/// `{a;b}. :- a, b.`: satisfiable alone, but assuming both `a` and `b` true
/// makes it unsatisfiable, and both assumptions are needed together (assuming
/// only one leaves the other free, which is satisfiable).
const BOTH_NEEDED: &str = "{a;b}. :- a, b.";

#[test]
fn core_is_empty_before_the_handle_has_a_result() {
    let mut ctl = grounded(BOTH_NEEDED);
    let (la, lb) = (literal_of(&ctl, "a"), literal_of(&ctl, "b"));
    let handle = ctl
        .solve_yield(&[Assumption::from(la), Assumption::from(lb)])
        .unwrap();
    // Checked against clingo 5.8.2: calling `core()` before `model()`/`get()`
    // on a fresh yield handle returns an empty list, not an error, exactly as
    // it does for a satisfiable search (the C header's "not unsatisfiable"
    // case covers "not yet known to be unsatisfiable" the same way).
    assert!(handle.core().unwrap().is_empty());
}

#[test]
fn core_contains_both_assumptions_in_the_order_given_when_both_are_needed() {
    let mut ctl = grounded(BOTH_NEEDED);
    let (la, lb) = (literal_of(&ctl, "a"), literal_of(&ctl, "b"));
    let mut handle = ctl
        .solve_yield(&[Assumption::from(la), Assumption::from(lb)])
        .unwrap();
    let result = handle.get().unwrap();
    assert!(result.is_unsat());
    // Checked against clingo 5.8.2: the core lists exactly the two assumption
    // literals, in the order they were given to `solve`, not in literal-id
    // order (confirmed by also trying the reversed order below).
    assert_eq!(handle.core().unwrap(), [la, lb]);
}

#[test]
fn core_follows_assumption_order_not_literal_id_order() {
    let mut ctl = grounded(BOTH_NEEDED);
    let (la, lb) = (literal_of(&ctl, "a"), literal_of(&ctl, "b"));
    let mut handle = ctl
        .solve_yield(&[Assumption::from(lb), Assumption::from(la)])
        .unwrap();
    let _ = handle.get().unwrap();
    // `a`'s literal is numerically smaller than `b`'s (`a` is declared and
    // therefore numbered first), yet with the assumptions given as `[b, a]`
    // clingo 5.8.2 reports the core as `[b, a]`, not `[a, b]`.
    assert_eq!(handle.core().unwrap(), [lb, la]);
}

#[test]
fn core_contains_only_the_assumption_actually_needed_among_redundant_ones() {
    // `a` alone already makes the program unsatisfiable (`:- a.`); `b` has its
    // own independent, unused reason (`:- b.`), and `c` is unconstrained.
    // Checked against clingo 5.8.2: assuming all three true reports a core
    // containing only `a`. This is not a general guarantee of minimality
    // (clingo.h does not document one); it is this fixture's observed,
    // oracle-checked value.
    let mut ctl = grounded("{a;b;c}. :- a. :- b.");
    let (la, lb, lc) = (
        literal_of(&ctl, "a"),
        literal_of(&ctl, "b"),
        literal_of(&ctl, "c"),
    );
    let mut handle = ctl
        .solve_yield(&[
            Assumption::from(la),
            Assumption::from(lb),
            Assumption::from(lc),
        ])
        .unwrap();
    let _ = handle.get().unwrap();
    assert_eq!(handle.core().unwrap(), [la]);
}

#[test]
fn core_preserves_a_negative_assumptions_sign() {
    // `{a}. :- not a.`: `a` must be true for the program to be satisfiable;
    // assuming it false (`-lit`) is unsatisfiable, and clingo 5.8.2 reports
    // the core as the negative literal itself, not its positive form.
    let mut ctl = grounded("{a}. :- not a.");
    let la = literal_of(&ctl, "a");
    let mut handle = ctl.solve_yield(&[Assumption::from(la.negate())]).unwrap();
    let _ = handle.get().unwrap();
    assert_eq!(handle.core().unwrap(), [la.negate()]);
}

#[test]
fn core_is_empty_when_the_search_is_satisfiable() {
    let mut ctl = grounded("a.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    assert!(handle.get().unwrap().is_sat());
    assert!(handle.core().unwrap().is_empty());
}

#[test]
fn core_is_empty_when_unsat_does_not_come_from_assumptions() {
    // `a :- not a.` is unsatisfiable on its own, with no assumptions at all.
    // Checked against clingo 5.8.2: the core is empty here too, since `core`
    // only ever reports on assumption literals, never on the program's own
    // rules.
    let mut ctl = grounded("a :- not a.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.get().unwrap().is_unsat());
    assert!(handle.core().unwrap().is_empty());
}

#[test]
fn core_is_populated_by_get_alone_without_reading_a_model_first() {
    // `SolveHandle::get` "waits for the result of the search"; for an
    // unsatisfiable search that never lends a model at all, calling it
    // directly must be enough to finish the search and populate the core, as
    // clingo 5.8.2 confirms (`hnd.get()` alone, no `hnd.model()` first).
    let mut ctl = grounded(BOTH_NEEDED);
    let (la, lb) = (literal_of(&ctl, "a"), literal_of(&ctl, "b"));
    let mut handle = ctl
        .solve_yield(&[Assumption::from(la), Assumption::from(lb)])
        .unwrap();
    assert!(handle.get().unwrap().is_unsat());
    assert_eq!(handle.core().unwrap(), [la, lb]);
}

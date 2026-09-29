//! `AsyncSolveHandle::core`: the same unsat core [`SolveHandle::core`] reads
//! (`clingo_solve_handle_core`), on a search run on clasp's own thread
//! (`Control::solve_async`).
//!
//! clingo does not distinguish the yield and async forms here: pyclingo uses
//! one `SolveHandle` class for both, and the oracle check for the fixtures
//! below (`clingo` 5.8.2, `ctl.solve(async_=True)`, `hnd.wait()`, `hnd.get()`,
//! `hnd.core()`) gave the same values as the yield form already checked for
//! `api_solve_core.rs`. Asynchronous solving needs a
//! build of clingo with threads, so every test is gated on `HAS_THREADS` and
//! returns at once otherwise, as `threads_optimisation.rs` and
//! `conformance_scripts.rs` do.
//!
//! [`SolveHandle::core`]: clingox::SolveHandle::core

#![forbid(unsafe_code)]

use clingox::{Assumption, Control, Part, ProgramLiteral, Symbol};

/// Whether this build of clingo has threads, which `Control::solve_async`
/// needs.
fn has_threads() -> bool {
    clingox_sys::HAS_THREADS
}

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

// `api_solve_core.rs` has a fixture for `core()` called before the handle has
// a result at all. It has no async counterpart here: in yield mode nothing
// runs until the handle is driven, so "before a result" is a state the test
// controls; in async mode the search starts on clasp's own thread as soon as
// `solve_async` returns, and `BOTH_NEEDED` solves fast enough that it can
// already be finished by the time `core()` is called, making the fixture's
// "still empty" assertion flaky rather than a real test of anything. Every
// other fixture below calls `core()` only after `get()` has waited for the
// result, which is deterministic.

#[test]
fn core_contains_both_assumptions_in_the_order_given_when_both_are_needed() {
    if !has_threads() {
        return;
    }
    let mut ctl = grounded(BOTH_NEEDED);
    let (la, lb) = (literal_of(&ctl, "a"), literal_of(&ctl, "b"));
    let mut handle = ctl
        .solve_async(&[Assumption::from(la), Assumption::from(lb)])
        .unwrap();
    // `get()` alone waits for the background search to finish; an async
    // handle lends no model to wait for first.
    assert!(handle.get().unwrap().is_unsat());
    // Checked against clingo 5.8.2 (`async_=True`): the core lists exactly
    // the two assumption literals, in the order they were given to `solve`,
    // not in literal-id order (confirmed by also trying the reversed order
    // below).
    assert_eq!(handle.core().unwrap(), [la, lb]);
}

#[test]
fn core_follows_assumption_order_not_literal_id_order() {
    if !has_threads() {
        return;
    }
    let mut ctl = grounded(BOTH_NEEDED);
    let (la, lb) = (literal_of(&ctl, "a"), literal_of(&ctl, "b"));
    let mut handle = ctl
        .solve_async(&[Assumption::from(lb), Assumption::from(la)])
        .unwrap();
    assert!(handle.get().unwrap().is_unsat());
    // `a`'s literal is numerically smaller than `b`'s (`a` is declared and
    // therefore numbered first), yet with the assumptions given as `[b, a]`
    // clingo 5.8.2 reports the core as `[b, a]`, not `[a, b]`.
    assert_eq!(handle.core().unwrap(), [lb, la]);
}

#[test]
fn core_contains_only_the_assumption_actually_needed_among_redundant_ones() {
    if !has_threads() {
        return;
    }
    // `a` alone already makes the program unsatisfiable (`:- a.`); `b` has
    // its own independent, unused reason (`:- b.`), and `c` is unconstrained.
    // Checked against clingo 5.8.2: assuming all three true reports a core
    // containing only `a`. This is this fixture's observed, oracle-checked
    // value, not a general guarantee of minimality (`clingo.h` does not
    // document one).
    let mut ctl = grounded("{a;b;c}. :- a. :- b.");
    let (la, lb, lc) = (
        literal_of(&ctl, "a"),
        literal_of(&ctl, "b"),
        literal_of(&ctl, "c"),
    );
    let mut handle = ctl
        .solve_async(&[
            Assumption::from(la),
            Assumption::from(lb),
            Assumption::from(lc),
        ])
        .unwrap();
    assert!(handle.get().unwrap().is_unsat());
    assert_eq!(handle.core().unwrap(), [la]);
}

#[test]
fn core_preserves_a_negative_assumptions_sign() {
    if !has_threads() {
        return;
    }
    // `{a}. :- not a.`: `a` must be true for the program to be satisfiable;
    // assuming it false (`-lit`) is unsatisfiable, and clingo 5.8.2 reports
    // the core as the negative literal itself, not its positive form.
    let mut ctl = grounded("{a}. :- not a.");
    let la = literal_of(&ctl, "a");
    let mut handle = ctl.solve_async(&[Assumption::from(la.negate())]).unwrap();
    assert!(handle.get().unwrap().is_unsat());
    assert_eq!(handle.core().unwrap(), [la.negate()]);
}

#[test]
fn core_is_empty_when_the_search_is_satisfiable() {
    if !has_threads() {
        return;
    }
    let mut ctl = grounded("a.");
    let mut handle = ctl.solve_async(&[]).unwrap();
    assert!(handle.get().unwrap().is_sat());
    assert!(handle.core().unwrap().is_empty());
}

#[test]
fn core_is_empty_when_unsat_does_not_come_from_assumptions() {
    if !has_threads() {
        return;
    }
    // `a :- not a.` is unsatisfiable on its own, with no assumptions at all.
    // Checked against clingo 5.8.2: the core is empty here too, since `core`
    // only ever reports on assumption literals, never on the program's own
    // rules.
    let mut ctl = grounded("a :- not a.");
    let mut handle = ctl.solve_async(&[]).unwrap();
    assert!(handle.get().unwrap().is_unsat());
    assert!(handle.core().unwrap().is_empty());
}

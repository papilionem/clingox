//! The theory atoms of a grounding: `Control::theory_atoms`, term
//! kinds, elements, guards, `to_string` forms, and the lifetime and id-reuse
//! rules `clingo.h`'s `TheoryAtoms` group states.
//!
//! Every expected value below was checked against the Python module `clingo`
//! 5.8.2 (`Control.theory_atoms`, whose terms expose `type`, `name`,
//! `number`, `arguments`), not assumed from `clingo.h`'s prose. The tests cover
//! the API surface this file depends on (`TheoryAtom::id`,
//! `TheoryAtom::elements` returning an owned `Vec`, `TheoryAtom::literal`
//! returning `Option<ProgramLiteral>`, `TheoryTerm::Compound`'s fields).

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail loudly on unexpected errors"
)]

use clingox::{Control, Part, Symbol, TheoryTerm, TheoryTermKind};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("creating a control");
    ctl.add("base", &[], program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn sym(text: &str) -> Symbol {
    text.parse().expect("the term parses")
}

/// A minimal theory with one binary left-associative operator, so a single
/// element can contain an arithmetic-looking compound term. Not a real
/// theory (nothing consumes the operator); clingo only needs the grammar to
/// parse `&a { ... }`, matching `theory-atoms.c`'s and `test_atoms.py`'s own
/// fixtures.
const THEORY: &str = "#theory t { term { + : 1, binary, left }; \
                       &a/0 : term, any; &b/0 : term, {=}, term, directive }.";

// ---------------------------------------------------------------------------
// TheoryAtoms: the view itself

#[test]
fn a_grounding_without_theory_atoms_is_empty() {
    let ctl = grounded("a. b :- a.");
    let atoms = ctl.theory_atoms().unwrap();
    assert_eq!(atoms.len().unwrap(), 0);
    assert!(atoms.is_empty().unwrap());
    assert_eq!(atoms.iter().count(), 0);
}

// ---------------------------------------------------------------------------
// Every term kind, with nesting
//
// Oracle: pyclingo 5.8.2, `test_atoms.py::test_theory_term`. `&a { 1,a,f(a),
// {1},(1,),[1] }.` has one element whose six terms are, in order, a number, a
// symbol, a function, a set, a tuple and a list; the function's one argument
// is the symbol term (`f(a)`), while the set, tuple and list each carry the
// number term as their one argument (checked 2026-09-27).

#[test]
fn every_theory_term_kind_is_reported_and_displayed() {
    let program = format!("{THEORY}\n&a {{ 1,a,f(a),{{1}},(1,),[1] }}.");
    let ctl = grounded(&program);
    let atoms = ctl.theory_atoms().unwrap();
    assert_eq!(atoms.len().unwrap(), 1);
    let atom = atoms.iter().next().unwrap().unwrap();
    let elements = atom.elements().unwrap();
    assert_eq!(elements.len(), 1);
    let terms = elements[0].tuple().unwrap();
    assert_eq!(terms.len(), 6);

    let texts: Vec<String> = terms
        .iter()
        .map(|&id| atoms.term(id).unwrap().to_string())
        .collect();
    assert_eq!(texts, vec!["1", "a", "f(a)", "{1}", "(1,)", "[1]"]);

    let num = atoms.term(terms[0]).unwrap();
    assert_eq!(atoms.term_kind(terms[0]).unwrap(), TheoryTermKind::Number);
    assert_eq!(num, TheoryTerm::Number(1));

    let sym = atoms.term(terms[1]).unwrap();
    assert_eq!(atoms.term_kind(terms[1]).unwrap(), TheoryTermKind::Symbol);
    assert_eq!(sym, TheoryTerm::Symbol(self::sym("a")));

    // `f(a)`'s one argument is the symbol term; `{1}`, `(1,)` and `[1]`'s one
    // argument is the number term (pyclingo: `fun.arguments == [sym]`,
    // `set_.arguments == tup.arguments == lst.arguments == [num]`).
    for (index, kind, name, expected_argument) in [
        (2, TheoryTermKind::Function, Some("f"), &sym),
        (3, TheoryTermKind::Set, None, &num),
        (4, TheoryTermKind::Tuple, None, &num),
        (5, TheoryTermKind::List, None, &num),
    ] {
        assert_eq!(atoms.term_kind(terms[index]).unwrap(), kind, "term {index}");
        match &atoms.term(terms[index]).unwrap() {
            TheoryTerm::Compound {
                kind: got_kind,
                name: got_name,
                arguments,
            } => {
                assert_eq!(*got_kind, kind, "term {index}");
                assert_eq!(*got_name, name, "term {index}");
                assert_eq!(*arguments, vec![expected_argument.clone()], "term {index}");
            }
            other => panic!("term {index}: expected a compound, got {other:?}"),
        }
    }
}

/// Nesting a compound inside a compound: `f(g(1,2),[3,[4,5]])` has a function
/// inside a function and a list inside a list.
///
/// Oracle: Python module, 2026-09-27: `f`'s
/// arguments are `[g(1,2), [3,[4,5]]]`; `g`'s arguments are the numbers 1
/// and 2; the outer list's second element is the list `[4,5]`.
#[test]
fn compound_terms_nest() {
    let program = format!("{THEORY}\n&a {{ f(g(1,2),[3,[4,5]]) }}.");
    let ctl = grounded(&program);
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let elements = atom.elements().unwrap();
    let top_id = elements[0].tuple().unwrap()[0];
    let top = atoms.term(top_id).unwrap();
    assert_eq!(top.to_string(), "f(g(1,2),[3,[4,5]])");

    let TheoryTerm::Compound {
        kind: TheoryTermKind::Function,
        name: Some("f"),
        arguments: f_args,
    } = &top
    else {
        panic!("expected the function f, got {top:?}");
    };
    assert_eq!(f_args.len(), 2);

    let TheoryTerm::Compound {
        kind: TheoryTermKind::Function,
        name: Some("g"),
        arguments: g_args,
    } = &f_args[0]
    else {
        panic!("expected the nested function g, got {:?}", f_args[0]);
    };
    assert_eq!(*g_args, vec![TheoryTerm::Number(1), TheoryTerm::Number(2)]);

    let TheoryTerm::Compound {
        kind: TheoryTermKind::List,
        name: None,
        arguments: outer_list,
    } = &f_args[1]
    else {
        panic!("expected the outer list, got {:?}", f_args[1]);
    };
    assert_eq!(outer_list[0], TheoryTerm::Number(3));
    let TheoryTerm::Compound {
        kind: TheoryTermKind::List,
        name: None,
        arguments: inner_list,
    } = &outer_list[1]
    else {
        panic!("expected the nested list, got {:?}", outer_list[1]);
    };
    assert_eq!(
        *inner_list,
        vec![TheoryTerm::Number(4), TheoryTerm::Number(5)]
    );
}

// ---------------------------------------------------------------------------
// Elements: conditions and condition literals

/// Oracle: pyclingo 5.8.2, `test_atoms.py::test_theory_element`. `&a { 1; 2,3:
/// a,b }.` (with `{a; b}.` so `a`/`b` are atoms) has two elements: `1`, with
/// no condition, and `2,3: a,b`, whose condition has two literals, both
/// positive program literals, and a `condition_id` that is also positive
/// (checked 2026-09-27).
#[test]
fn elements_carry_their_condition_and_condition_id() {
    let program = format!("{THEORY}\n{{a; b}}.\n&a {{ 1; 2,3: a,b }}.");
    let ctl = grounded(&program);
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let mut elements = atom.elements().unwrap();
    elements.sort_by_key(|e| e.tuple().unwrap().len());
    assert_eq!(elements.len(), 2);

    let bare = &elements[0];
    assert_eq!(bare.tuple().unwrap().len(), 1);
    assert_eq!(
        atoms.term(bare.tuple().unwrap()[0]).unwrap().to_string(),
        "1"
    );
    assert_eq!(bare.condition().unwrap().len(), 0);
    assert_eq!(bare.to_string(), "1");

    let conditioned = &elements[1];
    assert_eq!(conditioned.tuple().unwrap().len(), 2);
    let condition = conditioned.condition().unwrap();
    assert_eq!(condition.len(), 2);
    assert!(condition.iter().all(|l| l.get() > 0), "{condition:?}");
    assert!(conditioned.condition_id().unwrap().unwrap().get() > 0);
    assert_eq!(conditioned.to_string(), "2,3: a,b");
}

// ---------------------------------------------------------------------------
// Atoms: with and without a guard, `to_string`, and the literal

/// Oracle: pyclingo 5.8.2, `test_atoms.py::test_theory_atom`. `&a {}.` has no
/// guard and a positive literal; `&b {} = 1.` has a guard `("=", 1)`.
/// Matches `theory-atoms.c` and `libclingo.cc`'s `SECTION("theory-atoms")`
/// for the same shape (checked 2026-09-27).
#[test]
fn atoms_report_their_guard_or_its_absence() {
    let program = format!("{THEORY}\n&a {{}}.\n&b {{}} = 1.");
    let ctl = grounded(&program);
    let atoms = ctl.theory_atoms().unwrap();
    assert_eq!(atoms.len().unwrap(), 2);

    let mut a = None;
    let mut b = None;
    for item in atoms.iter() {
        let theory_atom = item.unwrap();
        match theory_atom.term().unwrap() {
            TheoryTerm::Symbol(s) if s.name() == Some("a") => a = Some(theory_atom),
            TheoryTerm::Symbol(s) if s.name() == Some("b") => b = Some(theory_atom),
            other => panic!("unexpected top-level term {other:?}"),
        }
    }
    let a = a.expect("&a is grounded");
    let b = b.expect("&b is grounded");

    assert_eq!(a.to_string(), "&a{}");
    assert!(a.guard().unwrap().is_none());
    assert!(a.elements().unwrap().is_empty());
    assert!(a.literal().unwrap().unwrap().get() > 0);

    assert_eq!(b.to_string(), "&b{}=1");
    let (connective, term) = b.guard().unwrap().expect("&b has a guard");
    assert_eq!(connective, "=");
    assert_eq!(term, TheoryTerm::Number(1));
}

/// A `directive`-role theory atom is never part of a rule body or head, so
/// clingo gives it no program literal at all: `clingo_theory_atoms_
/// atom_literal` reports `0`, which is not a valid `ProgramLiteral`
/// (`ProgramLiteral::from_raw(0)` is `None`, since the sign carries the
/// truth value). Checked directly (ctypes against `clingo_theory_atoms_
/// atom_literal`, 2026-09-27): the same `#theory` block's `&a/0 : term, any`
/// atom (usable in a rule) gets a positive literal, while `&b/0 : term,
/// {=}, term, directive` used as a fact-like statement (`&b {} = 42.`) gets
/// literal `0`. This is why `literal()` returns
/// `Result<Option<ProgramLiteral>>` rather than
/// `Result<ProgramLiteral>`.
#[test]
fn a_directive_atoms_literal_is_none() {
    let program = format!("{THEORY}\n&b {{}} = 42.");
    let ctl = grounded(&program);
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    assert_eq!(atom.literal().unwrap(), None);
}

// ---------------------------------------------------------------------------
// Ids are reused after a solve and a fresh grounding

/// `clingo.h`'s `TheoryAtoms` group: "All structural information about
/// theory atoms, elements, and terms is reset after solving. If afterward
/// fresh theory atoms are grounded, previously used ids are reused."
/// Checked directly (Python module, 2026-09-27): grounding the same program
/// twice, with a `solve()` between, gives the first theory atom id `0` both
/// times.
#[test]
fn atom_ids_are_reused_after_a_solve_and_reground() {
    // The `#theory` block is declared once, in `base`: redeclaring it in a
    // later part is a parse error ("redefinition of theory"), checked
    // directly. `next` only uses the atom the block already defined.
    let mut ctl = Control::new().unwrap();
    ctl.add("base", &[], &format!("{THEORY}\n&a {{ 1 }}."))
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let first_id = {
        let atoms = ctl.theory_atoms().unwrap();
        let atom = atoms.iter().next().unwrap().unwrap();
        assert_eq!(format!("{:?}", atom.id()), "Id(0)");
        atom.id()
    };
    let _ = ctl.solve(&[]).unwrap();
    ctl.add("next", &[], "&a { 1 }.").unwrap();
    ctl.ground(&[Part::new("next", &[]).unwrap()]).unwrap();
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    assert_eq!(atom.id(), first_id, "the id is reused, not a fresh Id(1)");
    assert_eq!(format!("{:?}", atom.id()), "Id(0)");
}

// ---------------------------------------------------------------------------
// Lookups with an id from a different grounding

/// A term id that is out of range for the *current* grounding is what
/// `clingo_theory_atoms_term_type` (and every other term/element accessor,
/// checked directly with `ctypes`) reports as `clingo_error_logic` ("Unknown
/// term '<id>'"), a clean, non-crashing error, unlike an out-of-range *atom*
/// id (`clingo_theory_atoms_atom_term` segfaults on one, checked directly).
/// `ErrorKind::Logic` already
/// poisons the control (`Control::note`, matching every other `Logic` error
/// in the crate), so clingox does not need to add its own bounds check here:
/// clingo already validates what its own contract requires, and clingox
/// only has to classify the result.
///
/// Fixture: grounding `f(g(1,2),[3,[4,5]])` uses term ids up to `11`;
/// grounding a small, unrelated program after a `solve()` leaves only ids
/// `0` and `1`. Reusing the old id `11` there is `Unknown term '11'`
/// (checked directly with `ctypes`, 2026-09-27).
#[test]
fn a_term_id_out_of_range_for_the_current_grounding_is_a_logic_error_and_poisons() {
    let mut ctl = Control::new().unwrap();
    ctl.add(
        "base",
        &[],
        &format!("{THEORY}\n&a {{ f(g(1,2),[3,[4,5]]) }}."),
    )
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let old_id = {
        let atoms = ctl.theory_atoms().unwrap();
        let atom = atoms.iter().next().unwrap().unwrap();
        let elements = atom.elements().unwrap();
        // The element has one term, the top-level function `f`; clingo
        // interns a compound's arguments before the compound itself, so
        // `f`'s id (checked directly: `11`) is the highest id this
        // grounding uses.
        elements[0].tuple().unwrap()[0]
    };
    let _ = ctl.solve(&[]).unwrap();
    // No `#theory` redeclaration here (see `atom_ids_are_reused_after_a_solve_
    // and_reground`): `next` only uses the atom `base` already declared.
    ctl.add("next", &[], "&a { 9 }.").unwrap();
    ctl.ground(&[Part::new("next", &[]).unwrap()]).unwrap();

    let err = {
        let atoms = ctl.theory_atoms().unwrap();
        atoms.term_kind(old_id).unwrap_err()
    };
    assert_eq!(err.kind(), clingox::ErrorKind::Logic);

    let poisoned = ctl.theory_atoms().unwrap_err();
    assert_eq!(poisoned.kind(), clingox::ErrorKind::Poisoned);
}

/// A term id that happens to still be *in range* for a later, unrelated
/// grounding is not an error at all: it silently names whatever term now
/// has that id, exactly the "wrong but never unsafe" shape `ProgramLiteral`
/// and `Symbol` already document for a value from another grounding
/// (`clingox/src/atoms.rs`). Checked directly (Python module, 2026-09-27):
/// term id `1` is `Number(9)` in `&a { 9 }.`; after a `solve()` and
/// regrounding `f(g(1,2),[3,[4,5]])` fresh, the same id `1` is a `Symbol`
/// term named `f` there instead, and reading it succeeds normally.
#[test]
fn a_term_id_still_in_range_for_a_later_grounding_reads_a_different_term() {
    let mut ctl = Control::new().unwrap();
    ctl.add("base", &[], &format!("{THEORY}\n&a {{ 9 }}."))
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (old_id, old_kind) = {
        let atoms = ctl.theory_atoms().unwrap();
        let atom = atoms.iter().next().unwrap().unwrap();
        let id = atom.elements().unwrap()[0].tuple().unwrap()[0];
        (id, atoms.term_kind(id).unwrap())
    };
    assert_eq!(old_kind, TheoryTermKind::Number);

    let _ = ctl.solve(&[]).unwrap();
    ctl.add("rich", &[], "&a { f(g(1,2),[3,[4,5]]) }.").unwrap();
    ctl.ground(&[Part::new("rich", &[]).unwrap()]).unwrap();

    let atoms = ctl.theory_atoms().unwrap();
    // The same numeric id, reused for the new grounding, now names a
    // different, unrelated term: no error, just a wrong answer for anyone
    // who still thought it meant the old `9`.
    let new_kind = atoms.term_kind(old_id).unwrap();
    assert_ne!(
        new_kind, old_kind,
        "the id was silently repurposed, not rejected"
    );
}

//! Regression tests for `TheoryTerm`'s `Display` and `TheoryAtom::literal`
//! on values `clingox/tests/api_theory_atoms.rs`'s own fixtures do not reach:
//! a raw negative theory number (only reachable through aspif input), a
//! declared unary prefix operator, and a theory atom whose aspif literal
//! differs from its clingox-side `Id`.
//!
//! Every expected value below was checked against the Python module `clingo`
//! 5.8.2 directly, 2026-09-27, with the oracle command in a comment above
//! each test.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail loudly on unexpected errors"
)]

use clingox::{Control, Part, TheoryTerm, TheoryTermKind};

/// A raw negative theory number, only reachable through aspif input: the
/// ordinary grammar has no negative number literal for a theory term, only a
/// unary-minus function (see `a_unary_operator_prints_prefix_in_parentheses`
/// below). Mirrors `conformance_pyclingo.rs::pyclingo_aspif_theory`'s own
/// "No guard" fixture, with `9 0 1 1` (term 1 = the number 1) changed to
/// `9 0 1 -5` (term 1 = the number -5).
///
/// Oracle, checked directly:
/// ```python
/// import clingo
/// ctl = clingo.Control()
/// ctl.add("base", [], "asp 1 0 0\n1 1 1 1 0 0\n1 0 1 2 0 0\n9 1 0 1 b\n"
///                      "9 0 1 -5\n9 4 0 1 1 1 1\n9 5 2 0 1 0\n4 1 x 1 1\n0\n")
/// ctl.ground([("base", [])])
/// for atom in ctl.theory_atoms:
///     for e in atom.elements:
///         for t in e.terms:
///             print(t.type, repr(str(t)))
/// # TheoryTermType.Number '(-5)'
/// ```
#[test]
fn a_negative_theory_number_prints_in_parentheses() {
    let mut ctl = Control::new().unwrap();
    ctl.add(
        "base",
        &[],
        "asp 1 0 0\n\
         1 1 1 1 0 0\n\
         1 0 1 2 0 0\n\
         9 1 0 1 b\n\
         9 0 1 -5\n\
         9 4 0 1 1 1 1\n\
         9 5 2 0 1 0\n\
         4 1 x 1 1\n\
         0\n",
    )
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let term_id = atom.elements().unwrap()[0].tuple().unwrap()[0];
    let term = atoms.term(term_id).unwrap();
    assert_eq!(term, TheoryTerm::Number(-5));
    assert_eq!(term.to_string(), "(-5)");
}

/// A declared unary prefix operator applied to a term. Mirrors
/// `theory::tests::displayed_compounds_match_clingos_own_term_to_string`'s
/// own `-1` case, standing on its own here as an acceptance-level check.
///
/// Oracle, checked directly:
/// ```python
/// import clingo
/// ctl = clingo.Control()
/// ctl.add("base", [],
///          "#theory t { term { - : 4, unary }; &a/0 : term, head }. &a { -5 }.")
/// ctl.ground([("base", [])])
/// for atom in ctl.theory_atoms:
///     for e in atom.elements:
///         for t in e.terms:
///             print(t.type, repr(str(t)))
/// # TheoryTermType.Function '(-5)'
/// ```
#[test]
fn a_unary_operator_prints_prefix_in_parentheses() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("#theory t { term { - : 4, unary }; &a/0 : term, head }. &a { -5 }.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let term_id = atom.elements().unwrap()[0].tuple().unwrap()[0];
    let term = atoms.term(term_id).unwrap();
    assert_eq!(
        term,
        TheoryTerm::Compound {
            kind: TheoryTermKind::Function,
            name: Some("-"),
            arguments: vec![TheoryTerm::Number(5)],
        }
    );
    assert_eq!(term.to_string(), "(-5)");
}

/// A theory atom's aspif literal is a program-wide number; its clingox `Id`
/// is a position among theory atoms only, starting at zero. Three ordinary
/// facts precede the two theory atoms here, so the second theory atom
/// (`Id(1)`) gets aspif atom 3 as its literal, not 1.
///
/// Oracle, checked directly:
/// ```python
/// import clingo
/// ctl = clingo.Control()
/// ctl.add("base", [],
///          "asp 1 0 0\n1 0 1 1 0 0\n1 0 1 2 0 0\n1 0 1 3 0 0\n"
///          "9 1 0 1 c\n9 1 1 1 d\n9 5 2 0 0\n9 5 3 1 0\n4 1 p 1 1\n0\n")
/// ctl.ground([("base", [])])
/// for i, atom in enumerate(ctl.theory_atoms):
///     print(i, atom, atom.literal)
/// # 0 &c{} 2
/// # 1 &d{} 3
/// ```
#[test]
fn an_atoms_literal_can_differ_from_its_id() {
    let mut ctl = Control::new().unwrap();
    ctl.add(
        "base",
        &[],
        "asp 1 0 0\n\
         1 0 1 1 0 0\n\
         1 0 1 2 0 0\n\
         1 0 1 3 0 0\n\
         9 1 0 1 c\n\
         9 1 1 1 d\n\
         9 5 2 0 0\n\
         9 5 3 1 0\n\
         4 1 p 1 1\n\
         0\n",
    )
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atoms = ctl.theory_atoms().unwrap();
    let second = atoms.iter().nth(1).unwrap().unwrap();
    assert_eq!(format!("{:?}", second.id()), "Id(1)");
    assert_eq!(second.literal().unwrap().unwrap().get(), 3);
}

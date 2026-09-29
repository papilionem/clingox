//! `SymbolicAtoms::of::<T>()` (smoke finding B5): the typed read of a
//! predicate's atoms in the grounder's domain, without a manual
//! `by_signature` and `from_symbol`, as `Model::atoms::<T>()` reads a model
//! (the clingo crate's `ctl.atoms::<T>()`).
//!
//! Oracle: pyclingo 5.8.2, `symbolic_atoms.by_signature(name, arity)`: every
//! atom of the domain with that name, arity and sign, facts and choices
//! alike, whatever its truth value. The order is not asserted; the tests
//! compare sorted values, as `Model::atoms` returns them sorted.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use clingox::{Control, ErrorKind, FromSymbol, Part, Signature};

#[derive(FromSymbol, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct P(i32);

#[derive(FromSymbol, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Edge(i32, i32);

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().unwrap();
    ctl.add_base(program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl
}

#[test]
fn it_reads_every_atom_of_the_predicate_in_the_domain() {
    // pyclingo: by_signature("p", 1, True) -> p(1), p(2), p(3)
    let ctl = grounded("p(2). {p(1)}. p(3) :- p(2). q(1). p(1,1). -p(4).");
    let atoms = ctl.symbolic_atoms().unwrap();
    let mut got = atoms.of::<P>().unwrap();
    got.sort();
    assert_eq!(got, [P(1), P(2), P(3)]);
}

#[test]
fn it_reads_by_name_and_arity() {
    let ctl = grounded("edge(1,2). {edge(2,3)}. edge(1). p(1).");
    let atoms = ctl.symbolic_atoms().unwrap();
    let mut got = atoms.of::<Edge>().unwrap();
    got.sort();
    assert_eq!(got, [Edge(1, 2), Edge(2, 3)]);
}

#[test]
fn it_matches_the_manual_read() {
    let ctl = grounded("{p(1..6)}. p(9) :- p(1), p(2).");
    let atoms = ctl.symbolic_atoms().unwrap();
    let mut manual = Vec::new();
    for atom in atoms.by_signature(Signature::new("p", 1).unwrap()) {
        manual.push(<P as FromSymbol>::from_symbol(atom.unwrap().symbol()).unwrap());
    }
    manual.sort();
    let mut typed = atoms.of::<P>().unwrap();
    typed.sort();
    assert_eq!(typed, manual);
    assert_eq!(typed.len(), 7);
}

#[test]
fn a_predicate_without_atoms_reads_as_empty() {
    let ctl = grounded("q(1).");
    assert!(ctl.symbolic_atoms().unwrap().of::<P>().unwrap().is_empty());
}

#[test]
fn an_atom_that_does_not_convert_is_a_conversion_error() {
    let ctl = grounded("p(a). p(1).");
    let error = ctl.symbolic_atoms().unwrap().of::<P>().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Conversion);
}

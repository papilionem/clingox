//! The symbolic atoms of a grounding, signatures, program literals, and
//! `Model::is_true`.
//!
//! Expected atoms, flags and signatures were checked against the Python module
//! `clingo` 5.8.2 (`Control.symbolic_atoms`), and the order of signatures
//! against `clingo_signature_is_less_than` through its C functions.

#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashSet};
use std::ops::ControlFlow;

use clingox::{Control, ErrorKind, Part, ProgramLiteral, Sign, Signature, Symbol, SymbolicAtom};

/// Facts, choices, externals, a derived atom, a classically negated fact, and a
/// predicate (`k/1`) whose only rule never fires.
const PROGRAM: &str = "f(1). f(2). {g(1..2)}. #external e. #external e2(3). h :- g(1). -n(1). \
                       k(X) :- f(X), X > 5.";

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn term(text: &str) -> Symbol {
    text.parse().expect("the test term parses")
}

fn signature(name: &str, arity: u32) -> Signature {
    Signature::new(name, arity).expect("the name has no NUL byte")
}

fn texts<I: IntoIterator<Item = clingox::Result<SymbolicAtom>>>(atoms: I) -> BTreeSet<String> {
    atoms
        .into_iter()
        .map(|atom| atom.expect("clingo reports the atom").symbol().to_string())
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

// ---------------------------------------------------------------------------
// SymbolicAtoms

#[test]
fn the_view_lists_every_atom_of_the_grounding() {
    let ctl = grounded(&[], PROGRAM);
    let atoms = ctl.symbolic_atoms().unwrap();
    assert_eq!(atoms.len().unwrap(), 8);
    let expected = set(&["-n(1)", "e", "e2(3)", "f(1)", "f(2)", "g(1)", "g(2)", "h"]);
    assert_eq!(texts(atoms.iter()), expected);
    assert_eq!(texts(&atoms), expected, "`&SymbolicAtoms` is iterable");
}

#[test]
fn atoms_report_facts_and_externals() {
    let ctl = grounded(&[], PROGRAM);
    let atoms = ctl.symbolic_atoms().unwrap();
    let mut facts = BTreeSet::new();
    let mut externals = BTreeSet::new();
    for atom in &atoms {
        let atom = atom.unwrap();
        if atom.is_fact() {
            facts.insert(atom.symbol().to_string());
        }
        if atom.is_external() {
            externals.insert(atom.symbol().to_string());
        }
    }
    assert_eq!(facts, set(&["-n(1)", "f(1)", "f(2)"]));
    assert_eq!(externals, set(&["e", "e2(3)"]));
}

/// Specific to literals from `SymbolicAtoms`: that is a property of this
/// source, not of `ProgramLiteral` in general. A `ProgramLiteral` built with
/// `ProgramLiteral::from_raw` or `ProgramLiteral::negate` can be
/// negative.
#[test]
fn literals_are_positive_and_distinct() {
    let ctl = grounded(&[], PROGRAM);
    let atoms = ctl.symbolic_atoms().unwrap();
    let literals: Vec<ProgramLiteral> = atoms.iter().map(|a| a.unwrap().literal()).collect();
    assert!(literals.iter().all(|l| l.get() > 0), "{literals:?}");
    let distinct: HashSet<ProgramLiteral> = literals.iter().copied().collect();
    assert_eq!(distinct.len(), literals.len());
}

#[test]
fn find_returns_the_atom_of_a_symbol() {
    let ctl = grounded(&[], PROGRAM);
    let atoms = ctl.symbolic_atoms().unwrap();
    for atom in &atoms {
        let atom = atom.unwrap();
        let found = atoms
            .find(atom.symbol())
            .unwrap()
            .expect("every listed atom can be found");
        assert_eq!(found, atom);
    }
    let g2 = atoms.find(term("g(2)")).unwrap().unwrap();
    assert!(!g2.is_fact() && !g2.is_external());
}

#[test]
fn find_returns_none_for_a_symbol_that_is_not_an_atom() {
    let ctl = grounded(&[], PROGRAM);
    let atoms = ctl.symbolic_atoms().unwrap();
    assert_eq!(atoms.find(term("zz")).unwrap(), None);
    assert_eq!(
        atoms.find(term("k(1)")).unwrap(),
        None,
        "k(1) is never derived"
    );
    assert_eq!(
        atoms.find(term("n(1)")).unwrap(),
        None,
        "only -n(1) is an atom"
    );
    assert_eq!(atoms.find(Symbol::number(1)).unwrap(), None);
}

#[test]
fn by_signature_lists_the_atoms_of_one_predicate() {
    let ctl = grounded(&[], PROGRAM);
    let atoms = ctl.symbolic_atoms().unwrap();
    assert_eq!(
        texts(atoms.by_signature(signature("f", 1))),
        set(&["f(1)", "f(2)"])
    );
    assert_eq!(texts(atoms.by_signature(signature("e", 0))), set(&["e"]));
    assert!(texts(atoms.by_signature(signature("k", 1))).is_empty());
    assert!(texts(atoms.by_signature(signature("nosuch", 3))).is_empty());
    assert!(
        texts(atoms.by_signature(signature("f", 2))).is_empty(),
        "arity counts"
    );
}

#[test]
fn by_signature_distinguishes_classical_negation() {
    let ctl = grounded(&[], PROGRAM);
    let atoms = ctl.symbolic_atoms().unwrap();
    assert!(texts(atoms.by_signature(signature("n", 1))).is_empty());
    let negative = Signature::with_sign("n", 1, Sign::Negative).unwrap();
    assert_eq!(texts(atoms.by_signature(negative)), set(&["-n(1)"]));
}

#[test]
fn signatures_include_predicates_without_atoms() {
    let ctl = grounded(&[], PROGRAM);
    let atoms = ctl.symbolic_atoms().unwrap();
    let signatures: BTreeSet<String> = atoms
        .signatures()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        signatures,
        set(&["-n/1", "e/0", "e2/1", "f/1", "g/1", "h/0", "k/1"])
    );
}

#[test]
fn an_empty_grounding_has_no_atoms() {
    let ctl = grounded(&[], "");
    let atoms = ctl.symbolic_atoms().unwrap();
    assert_eq!(atoms.len().unwrap(), 0);
    assert_eq!(atoms.iter().count(), 0);
    assert!(atoms.signatures().unwrap().is_empty());
}

#[test]
fn a_later_part_extends_the_atoms_and_can_make_facts() {
    // clingo 5.8.2: after a later part adds `p.`, the atom `p` from the choice
    // rule reads as a fact.
    let mut ctl = Control::new().unwrap();
    ctl.add_base("{p}.").unwrap();
    ctl.add("step", &[], "p. q.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    {
        let atoms = ctl.symbolic_atoms().unwrap();
        assert!(!atoms.find(term("p")).unwrap().unwrap().is_fact());
        assert_eq!(atoms.find(term("q")).unwrap(), None);
    }
    ctl.ground(&[Part::new("step", &[]).unwrap()]).unwrap();
    let atoms = ctl.symbolic_atoms().unwrap();
    assert!(atoms.find(term("p")).unwrap().unwrap().is_fact());
    assert!(atoms.find(term("q")).unwrap().unwrap().is_fact());
    assert_eq!(texts(&atoms), set(&["p", "q"]));
}

#[test]
fn a_forgotten_search_is_closed_before_atoms_are_read() {
    // DESIGN S4: `&self` entry points finish a leftover search first.
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);
    assert_eq!(ctl.symbolic_atoms().unwrap().len().unwrap(), 2);
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
}

#[test]
fn symbolic_atoms_refuse_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let err = ctl.symbolic_atoms().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

#[test]
fn atoms_are_plain_values() {
    fn atom_traits<T: Copy + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static>() {}
    atom_traits::<SymbolicAtom>();
    atom_traits::<ProgramLiteral>();

    let ctl = grounded(&[], PROGRAM);
    let atoms = ctl.symbolic_atoms().unwrap();
    let g1 = atoms.find(term("g(1)")).unwrap().unwrap();
    let text = format!("{g1:?}");
    assert!(text.contains("g(1)"), "{text}");
    let text = format!("{atoms:?}");
    assert!(text.contains("SymbolicAtoms"), "{text}");
    let text = format!("{:?}", atoms.iter());
    assert!(text.contains("SymbolicAtomIter"), "{text}");
}

// ---------------------------------------------------------------------------
// Model::is_true

#[test]
fn is_true_agrees_with_contains_for_every_atom() {
    // `{a}. b :- a. c.` has the models {c} and {a, b, c}.
    let mut ctl = grounded(&["--models=0"], "{a}. b :- a. c.");
    let literals: Vec<(Symbol, ProgramLiteral)> = ctl
        .symbolic_atoms()
        .unwrap()
        .iter()
        .map(|atom| {
            let atom = atom.unwrap();
            (atom.symbol(), atom.literal())
        })
        .collect();
    assert_eq!(literals.len(), 3);

    let mut seen = BTreeSet::new();
    let result = ctl
        .for_each_model(&[], |model| {
            let mut true_atoms = BTreeSet::new();
            for (symbol, literal) in &literals {
                let is_true = model.is_true(*literal)?;
                assert_eq!(is_true, model.contains(*symbol)?, "{symbol}");
                if is_true {
                    true_atoms.insert(symbol.to_string());
                }
            }
            seen.insert(true_atoms.into_iter().collect::<Vec<_>>());
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(result.is_exhausted());
    let expected: BTreeSet<Vec<String>> = [vec!["c"], vec!["a", "b", "c"]]
        .into_iter()
        .map(|m| m.into_iter().map(str::to_owned).collect())
        .collect();
    assert_eq!(seen, expected);
}

// ---------------------------------------------------------------------------
// Signature

#[test]
fn signatures_have_a_name_an_arity_and_a_sign() {
    let p = signature("p", 2);
    assert_eq!(p.name(), "p");
    assert_eq!(p.arity(), 2);
    assert_eq!(p.sign(), Sign::Positive);
    let q = Signature::with_sign("q", 0, Sign::Negative).unwrap();
    assert_eq!(q.name(), "q");
    assert_eq!(q.arity(), 0);
    assert_eq!(q.sign(), Sign::Negative);
    assert_ne!(
        signature("n", 1),
        Signature::with_sign("n", 1, Sign::Negative).unwrap()
    );
    assert_eq!(
        signature("n", 1),
        Signature::with_sign("n", 1, Sign::Positive).unwrap()
    );
}

#[test]
fn signatures_print_like_clingo() {
    assert_eq!(signature("p", 2).to_string(), "p/2");
    let q = Signature::with_sign("q", 0, Sign::Negative).unwrap();
    assert_eq!(q.to_string(), "-q/0");
    assert_eq!(format!("{:?}", signature("p", 2)), "Signature(p/2)");
}

#[test]
fn signatures_parse_their_text_form() {
    assert_eq!("p/2".parse::<Signature>().unwrap(), signature("p", 2));
    assert_eq!(
        "-q/0".parse::<Signature>().unwrap(),
        Signature::with_sign("q", 0, Sign::Negative).unwrap()
    );
    for text in ["-q/0", "p/2", "e2/1"] {
        assert_eq!(text.parse::<Signature>().unwrap().to_string(), text);
    }
}

#[test]
fn malformed_signatures_do_not_parse() {
    for text in ["", "p", "p/", "p/x", "p/-1", "p/4294967296", "/", "p/1.5"] {
        let err = text.parse::<Signature>().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Parse, "{text:?}");
    }
}

#[test]
fn a_signature_name_with_nul_is_rejected() {
    let err = Signature::new("p\0", 1).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
}

#[test]
fn signatures_order_like_clingo() {
    // clingo 5.8.2: positive before negative, then by arity, then by name.
    let mut signatures = [
        signature("a", 2),
        signature("b", 1),
        signature("a", 1),
        Signature::with_sign("a", 1, Sign::Negative).unwrap(),
        Signature::with_sign("b", 0, Sign::Negative).unwrap(),
        signature("zz", 0),
    ];
    signatures.sort();
    let texts: Vec<String> = signatures.iter().map(ToString::to_string).collect();
    assert_eq!(texts, vec!["zz/0", "a/1", "b/1", "a/2", "-b/0", "-a/1"]);
}

#[test]
fn signatures_are_values_shared_between_threads() {
    fn signature_traits<
        T: Copy
            + Send
            + Sync
            + Eq
            + Ord
            + std::hash::Hash
            + std::fmt::Debug
            + std::fmt::Display
            + std::str::FromStr
            + 'static,
    >() {
    }
    signature_traits::<Signature>();
    let mut set = HashSet::new();
    set.insert(signature("p", 1));
    assert!(set.contains(&"p/1".parse().unwrap()));
}

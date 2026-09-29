//! Adding typed facts: `Control::add_facts`.
//!
//! The multi-shot behaviour was checked against the Python module `clingo`
//! 5.8.2 by adding the same facts in a separate part and grounding only that
//! part, including the error code of the redefinition case
//! (`clingo_error_code()` through `clingo._internal._lib`: `clingo_error_logic`).
//! Which program texts clingo accepts as facts was checked the same way.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use clingox::{Control, ErrorKind, Part, Sign, Symbol, ToSymbol};

#[derive(ToSymbol)]
struct P(i32);

#[derive(ToSymbol)]
struct Edge(i32, i32);

#[derive(ToSymbol)]
struct Count(u32);

#[derive(ToSymbol)]
struct Label(#[clingo(string)] String);

#[derive(ToSymbol)]
struct Tag(#[clingo(constant)] String);

fn term(text: &str) -> Symbol {
    text.parse().expect("the test term parses")
}

fn constant(name: &str) -> Symbol {
    Symbol::function(name, &[]).expect("the name has no NUL byte")
}

/// Every model of the control, each as the set of its shown symbols' texts.
fn answer_sets(ctl: &mut Control) -> BTreeSet<BTreeSet<String>> {
    let (result, models) = ctl.solve_all().expect("the search succeeds");
    assert!(result.is_exhausted(), "{result}");
    models
        .iter()
        .map(|m| m.symbols().iter().map(Symbol::to_string).collect())
        .collect()
}

fn sets(models: &[&[&str]]) -> BTreeSet<BTreeSet<String>> {
    models
        .iter()
        .map(|m| m.iter().map(|s| (*s).to_owned()).collect())
        .collect()
}

fn control(program: &str) -> Control {
    let mut ctl = Control::new().expect("a control can be created");
    ctl.add_base(program).expect("the program parses");
    ctl
}

fn assert_usable(ctl: &mut Control) {
    assert!(!format!("{ctl:?}").contains("poisoned"), "{ctl:?}");
    let result = ctl.solve(&[]).expect("the control is still usable");
    assert!(result.is_sat(), "{result}");
}

#[test]
fn facts_are_visible_to_rules_grounded_afterwards() {
    let mut ctl = control("q(X) :- p(X).");
    ctl.add_facts([P(1), P(2)]).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(
        answer_sets(&mut ctl),
        sets(&[&["p(1)", "p(2)", "q(1)", "q(2)"]])
    );
}

#[test]
fn add_facts_grounds_the_facts_at_once() {
    let mut ctl = Control::new().unwrap();
    ctl.add_facts([Edge(1, 2), Edge(2, 3)]).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["edge(1,2)", "edge(2,3)"]]));
}

#[test]
fn add_facts_accepts_symbols_references_and_derived_values() {
    let mut ctl = Control::new().unwrap();
    let symbols = [term("a"), term("b(1)")];
    ctl.add_facts(symbols.iter()).unwrap();
    let edges = vec![Edge(3, 4)];
    ctl.add_facts(&edges).unwrap();
    ctl.add_facts(edges.iter().map(|edge| P(edge.1))).unwrap();
    ctl.add_facts(vec![term("-c")]).unwrap();
    assert_eq!(
        answer_sets(&mut ctl),
        sets(&[&["-c", "a", "b(1)", "edge(3,4)", "p(4)"]])
    );
}

#[test]
fn add_facts_after_solving_does_not_reground_base() {
    // clingo 5.8.2: grounding `base` again after this solve fails with
    // "redefinition of atom <'a',1>", a logic error.
    let mut ctl = control("{a}. q(X) :- p(X).");
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&[], &["a"]]));

    ctl.add_facts([P(1)]).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["p(1)"], &["a", "p(1)"]]));
}

#[test]
fn add_facts_can_be_called_repeatedly() {
    let mut ctl = Control::new().unwrap();
    ctl.add_facts([P(1)]).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["p(1)"]]));
    // Adding an existing fact again is harmless.
    ctl.add_facts([P(1), P(2)]).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["p(1)", "p(2)"]]));
    ctl.add_facts([P(3)]).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["p(1)", "p(2)", "p(3)"]]));
}

#[test]
fn rules_grounded_before_the_facts_do_not_see_them() {
    let mut ctl = control("q(X) :- p(X).");
    ctl.ground(&[Part::base()]).unwrap();
    ctl.add_facts([P(1)]).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["p(1)"]]));
}

#[test]
fn a_fact_for_an_atom_defined_in_an_earlier_step_is_a_logic_error() {
    let mut ctl = control("{p(1)}.");
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&[], &["p(1)"]]));

    // clingo 5.8.2: "redefinition of atom <'p(1)',1>", clingo_error_logic.
    let err = ctl.add_facts([P(1)]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Logic, "{err}");
    let err = ctl.add_facts([P(2)]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

#[test]
fn symbols_that_are_not_atoms_are_rejected_without_poisoning() {
    let mut ctl = control("a.");
    ctl.ground(&[Part::base()]).unwrap();
    for symbol in [
        Symbol::number(1),
        Symbol::string("x").unwrap(),
        Symbol::supremum(),
        Symbol::infimum(),
        term("(1,2)"),
        term("(1,)"),
        term("()"),
        term("-(1,2)"),
    ] {
        let err = ctl.add_facts([symbol]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Conversion, "{symbol}: {err}");
        assert_usable(&mut ctl);
    }
    assert_eq!(answer_sets(&mut ctl), sets(&[&["a"]]));
}

#[test]
fn names_clingo_cannot_read_back_are_rejected_without_poisoning() {
    // clingo 5.8.2 reads `Foo` as a variable, and rejects `not.`, `p(not).`,
    // `-not.` and `a b.` as syntax errors.
    let mut ctl = control("a.");
    ctl.ground(&[Part::base()]).unwrap();
    let rejected = [
        constant("Foo"),
        constant("not"),
        Symbol::function_with_sign("not", &[], Sign::Negative).unwrap(),
        Symbol::function("p", &[constant("not")]).unwrap(),
        Symbol::function("p", &[constant("Foo")]).unwrap(),
        Symbol::function("p", &[Symbol::number(1), constant("_")]).unwrap(),
        Symbol::function("q", &[Symbol::function("r", &[constant("X")]).unwrap()]).unwrap(),
        constant("a b"),
        constant("_"),
        constant("é"),
        constant("1a"),
    ];
    for symbol in rejected {
        let err = ctl.add_facts([symbol]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Conversion, "{symbol}: {err}");
        assert_usable(&mut ctl);
    }
    assert_eq!(answer_sets(&mut ctl), sets(&[&["a"]]));
}

#[test]
fn every_fact_clingo_reads_back_is_accepted_exactly() {
    // Each of these texts is a fact clingo 5.8.2 accepts, and the model holds
    // exactly the symbol that was added.
    let accepted = [
        constant("_p"),
        constant("__p"),
        constant("x'"),
        constant("default"),
        constant("show"),
        constant("true"),
        constant("pQ_9'"),
        term("-p(1)"),
        term("p(#sup,#inf)"),
        term("p((1,2),(),(1,))"),
        term("p(-q,-1,-r(2))"),
        term("p(-(1,2))"),
        Symbol::function("s", &[Symbol::string("a\"b\\c\nd\té").unwrap()]).unwrap(),
        Symbol::function("s", &[Symbol::string("").unwrap()]).unwrap(),
    ];
    let mut ctl = Control::new().unwrap();
    ctl.add_facts(accepted).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_exhausted());
    assert_eq!(models.len(), 1);
    let expected: BTreeSet<Symbol> = accepted.into_iter().collect();
    let found: BTreeSet<Symbol> = models[0].all_atoms().iter().copied().collect();
    assert_eq!(found, expected);
}

#[test]
fn a_failed_conversion_adds_nothing() {
    let mut ctl = Control::new().unwrap();
    let err = ctl
        .add_facts([Count(1), Count(u32::MAX), Count(2)])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    let err = ctl
        .add_facts([term("p(1)"), Symbol::number(5)])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    let err = ctl.add_facts([term("p(2)"), constant("Foo")]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert_eq!(answer_sets(&mut ctl), sets(&[&[]]));
}

#[test]
fn string_and_constant_facts_differ() {
    // clingo 5.8.2: `label("x")` does not satisfy `label(x)`.
    let mut ctl = control("by_string :- label(x). by_constant :- tag(x).");
    ctl.add_facts([Label("x".into())]).unwrap();
    ctl.add_facts([Tag("x".into())]).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(
        answer_sets(&mut ctl),
        sets(&[&["by_constant", "label(\"x\")", "tag(x)"]])
    );
}

#[test]
fn an_empty_iterator_changes_nothing() {
    let mut ctl = control("a.");
    ctl.ground(&[Part::base()]).unwrap();
    ctl.add_facts(Vec::<Symbol>::new()).unwrap();
    ctl.add_facts(std::iter::empty::<P>()).unwrap();
    assert_eq!(answer_sets(&mut ctl), sets(&[&["a"]]));
}

#[test]
fn add_facts_on_a_poisoned_control_is_refused() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let err = ctl.add_facts([P(1)]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

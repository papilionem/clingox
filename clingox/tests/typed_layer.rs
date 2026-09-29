//! Further checks of the typed layer, beyond the acceptance
//! tests: error messages, `sym!` inside other macros, and the edges of the
//! conversions. Symbol texts were checked against the Python module `clingo`
//! 5.8.2.

#![forbid(unsafe_code)]

use clingox::testing::parse_answer;
use clingox::{Control, ErrorKind, FromSymbol, Part, Symbol, ToSymbol, sym};

fn term(text: &str) -> Symbol {
    text.parse().expect("the test term parses")
}

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Node(i32);

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
enum Shape {
    Point,
    Circle(i32),
}

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Keyword {
    r#type: i32,
}

#[test]
fn an_enum_error_names_the_enum_and_the_symbol() {
    let err = Shape::from_symbol(term("circle(1,2)")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    let message = err.to_string();
    assert!(message.contains("Shape"), "{message}");
    assert!(message.contains("circle(1,2)"), "{message}");
}

#[test]
fn a_struct_error_names_the_expected_predicate() {
    let message = Node::from_symbol(term("vertex(1)"))
        .unwrap_err()
        .to_string();
    assert!(message.contains("vertex(1)"), "{message}");
    assert!(message.contains("node/1"), "{message}");
}

#[test]
fn an_integer_error_names_the_value() {
    let message = u32::MAX.to_symbol().unwrap_err().to_string();
    assert!(message.contains("4294967295"), "{message}");
}

#[test]
fn raw_field_names_are_read_without_their_prefix() {
    let value = Keyword { r#type: 1 };
    assert_eq!(value.to_symbol().unwrap(), term("keyword(1)"));
    let err = Keyword::from_symbol(term("keyword(a)")).unwrap_err();
    assert!(err.to_string().contains("field `type`"), "{err}");
}

macro_rules! edge {
    ($from:expr, $to:expr) => {
        clingox::sym!(edge({ $from }, { $to }))
    };
}

#[test]
fn sym_works_inside_another_macro() {
    let from = 1;
    assert_eq!(edge!(from, 2).unwrap(), term("edge(1,2)"));
}

#[test]
fn a_splice_may_hold_statements() {
    let symbol = sym!(p({
        let n = 20;
        n + 1
    }))
    .unwrap();
    assert_eq!(symbol, term("p(21)"));
}

#[test]
fn sym_accepts_other_integer_notations() {
    assert_eq!(sym!(p(0x10, 1_000)).unwrap(), term("p(16,1000)"));
    assert_eq!(sym!(-0).unwrap(), Symbol::number(0));
}

#[test]
fn sym_splices_nest_in_negated_functions_and_tuples() {
    let node = Node(2);
    assert_eq!(
        sym!(-p((1, { node }), { Shape::Point })).unwrap(),
        term("-p((1,node(2)),point)")
    );
}

#[test]
fn owned_models_read_shown_terms() {
    #[derive(FromSymbol, Debug, PartialEq)]
    struct T(i32);

    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(1). #show t(X) : p(X).").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models[0].shown::<T>().unwrap(), [T(1)]);
    assert_eq!(models[0].atoms::<T>().unwrap(), []);
}

#[test]
fn add_facts_names_the_rejected_symbol() {
    let mut ctl = Control::new().unwrap();
    let foo = Symbol::function("Foo", &[]).unwrap();
    let err = ctl
        .add_facts([Symbol::function("p", &[foo]).unwrap()])
        .unwrap_err();
    assert!(err.to_string().contains("p(Foo)"), "{err}");
    let err = ctl.add_facts([Symbol::number(3)]).unwrap_err();
    assert!(err.to_string().contains("`3`"), "{err}");
}

#[test]
fn add_facts_after_a_part_with_the_same_prefix_stays_separate() {
    // A user part named like the internal ones does not collide: the facts
    // parts are numbered per control, and grounding one grounds only it.
    let mut ctl = Control::new().unwrap();
    ctl.add("step", &[], "q(1).").unwrap();
    ctl.add_facts([Node(1)]).unwrap();
    ctl.add_facts([Node(2)]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models[0].atoms::<Node>().unwrap(), [Node(1), Node(2)]);
    assert!(!models[0].contains(term("q(1)")));
}

#[test]
fn parse_answer_reads_escaped_backslashes_before_a_quote() {
    assert_eq!(
        parse_answer(r#"s("a\\") t"#).unwrap(),
        [
            Symbol::function("s", &[Symbol::string("a\\").unwrap()]).unwrap(),
            term("t")
        ]
    );
}

#[test]
fn add_facts_closes_a_forgotten_search_first() {
    for facts in [vec![], vec![Symbol::number(1)], vec![term("p(1)")]] {
        let mut ctl = Control::with_args(["--models=0"]).unwrap();
        ctl.add_base("{a; b}.").unwrap();
        ctl.ground(&[Part::base()]).unwrap();
        let mut handle = ctl.solve_yield(&[]).unwrap();
        assert!(handle.next_model().unwrap().is_some());
        std::mem::forget(handle);
        assert!(format!("{ctl:?}").contains("solving"), "{ctl:?}");
        let _ = ctl.add_facts(facts);
        assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
    }
}

#[test]
#[allow(
    clippy::used_underscore_items,
    reason = "the leading underscore is what the test is about"
)]
fn derived_names_keep_leading_underscores() {
    #[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
    struct _Hidden(i32);

    assert_eq!(_Hidden(1).to_symbol().unwrap(), term("_hidden(1)"));
    assert_eq!(<_Hidden as clingox::Predicate>::NAME, "_hidden");
    assert_eq!(
        _Hidden::from_symbol(term("_hidden(2)")).unwrap(),
        _Hidden(2)
    );
}

#[test]
fn the_part_names_of_add_facts_are_reserved() {
    let mut ctl = Control::new().unwrap();
    // clingox's own check on caller input is `InvalidInput` .
    let err = ctl.add("__clingox_facts_0", &[], "p(9).").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(err.to_string().contains("__clingox_facts_"), "{err}");
    let err = Part::new("__clingox_facts_1", &[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    // The refusal does not poison, and names that only resemble the prefix
    // are ordinary.
    ctl.add("__clingox_fact", &[], "q.").unwrap();
    ctl.add_facts([Node(1)]).unwrap();
    ctl.ground(&[Part::new("__clingox_fact", &[]).unwrap()])
        .unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models[0].atoms::<Node>().unwrap(), [Node(1)]);
    assert!(models[0].contains(term("q")));
}

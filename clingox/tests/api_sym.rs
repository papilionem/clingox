//! `sym!`: symbols written in clingo's term syntax, with `{expr}` splices.
//!
//! Each expected symbol is the one clingo's own parser builds from the same
//! text (`Symbol::from_str`, which calls `clingo_parse_term`). Which texts
//! clingo 5.8.2 accepts, and how it prints them, was checked with the Python
//! module `clingo` 5.8.2.

#![forbid(unsafe_code)]

use std::cell::Cell;

use clingox::{ErrorKind, Sign, Symbol, ToSymbol, sym};

fn term(text: &str) -> Symbol {
    text.parse().expect("the test term parses")
}

#[derive(ToSymbol)]
struct Node(i32);

#[test]
fn sym_builds_the_same_symbol_as_the_parser() {
    let symbol = sym!(p(1, "x", c, -q, (1, 2), #sup, #inf)).unwrap();
    assert_eq!(symbol, term(r#"p(1,"x",c,-q,(1,2),#sup,#inf)"#));
    assert_eq!(symbol.to_string(), r#"p(1,"x",c,-q,(1,2),#sup,#inf)"#);

    assert_eq!(sym!(p).unwrap(), term("p"));
    assert_eq!(sym!(f(g(h(1)), k)).unwrap(), term("f(g(h(1)),k)"));
}

#[test]
fn sym_reads_numbers_including_negative_and_extremes() {
    assert_eq!(sym!(7).unwrap(), Symbol::number(7));
    assert_eq!(sym!(-5).unwrap(), Symbol::number(-5));
    assert_eq!(sym!(0).unwrap(), Symbol::number(0));
    assert_eq!(
        sym!(p(-2147483648, 2147483647)).unwrap(),
        Symbol::function("p", &[Symbol::number(i32::MIN), Symbol::number(i32::MAX)]).unwrap()
    );
}

#[test]
fn sym_reads_strings_with_escapes() {
    assert_eq!(sym!("x").unwrap(), Symbol::string("x").unwrap());
    let symbol = sym!(p("a\"b\\c\nd")).unwrap();
    assert_eq!(
        symbol.arguments().unwrap()[0].as_string(),
        Some("a\"b\\c\nd")
    );
    assert_eq!(symbol, term(r#"p("a\"b\\c\nd")"#));
    assert_eq!(sym!(r"a\b").unwrap(), Symbol::string("a\\b").unwrap());
    assert_eq!(sym!(p("")).unwrap(), term(r#"p("")"#));
}

#[test]
fn sym_reads_classical_negation() {
    let negated = sym!(-p).unwrap();
    assert_eq!(negated.sign(), Some(Sign::Negative));
    assert_eq!(negated, term("-p"));
    assert_eq!(sym!(-p(1)).unwrap(), term("-p(1)"));
    assert_eq!(sym!(q(-r(2))).unwrap(), term("q(-r(2))"));
    assert_eq!(sym!(p(1)).unwrap().sign(), Some(Sign::Positive));
}

#[test]
fn sym_reads_tuples_of_every_size() {
    assert_eq!(sym!(()).unwrap(), term("()"));
    assert_eq!(sym!(()).unwrap(), Symbol::tuple(&[]).unwrap());
    assert_eq!(sym!((1,)).unwrap(), term("(1,)"));
    assert_eq!(sym!((1, 2)).unwrap(), term("(1,2)"));
    // A trailing comma is allowed in a tuple. `#inf` keeps rustfmt from
    // reformatting the tuple.
    assert_eq!(sym!((1, 2, #inf,)).unwrap(), term("(1,2,#inf)"));
    assert_eq!(sym!(((1, 2), (3,), ())).unwrap(), term("((1,2),(3,),())"));
    // A parenthesised term is the term itself, as in clingo.
    assert_eq!(sym!((1)).unwrap(), Symbol::number(1));
    assert_eq!(sym!((p(1))).unwrap(), term("p(1)"));
}

#[test]
fn sym_reads_supremum_and_infimum() {
    assert_eq!(sym!(#sup).unwrap(), Symbol::supremum());
    assert_eq!(sym!(#inf).unwrap(), Symbol::infimum());
}

#[test]
fn an_empty_argument_list_is_a_constant() {
    assert_eq!(sym!(p()).unwrap(), sym!(p).unwrap());
    assert_eq!(sym!(-p()).unwrap(), term("-p"));
}

#[test]
fn names_follow_clingo_not_rust() {
    // Rust keywords are ordinary clingo names.
    assert_eq!(sym!(type(1)).unwrap(), term("type(1)"));
    assert_eq!(sym!(match).unwrap(), term("match"));
    assert_eq!(sym!(p(fn, struct)).unwrap(), term("p(fn,struct)"));
    // Names may start with underscores and contain digits and capitals.
    assert_eq!(sym!(_p).unwrap(), term("_p"));
    assert_eq!(sym!(p2Q_r(1)).unwrap(), term("p2Q_r(1)"));
}

#[test]
fn splices_accept_any_to_symbol_value() {
    let n = 42;
    let q = sym!(q).unwrap();
    let node = Node(3);
    let pair = (1, 2);
    let list = vec![4, 5];

    assert_eq!(sym!({ n }).unwrap(), Symbol::number(42));
    assert_eq!(sym!(p({ n })).unwrap(), term("p(42)"));
    assert_eq!(sym!(p({ n + 1 })).unwrap(), term("p(43)"));
    assert_eq!(sym!(p({ q })).unwrap(), term("p(q)"));
    assert_eq!(sym!(p({ node })).unwrap(), term("p(node(3))"));
    assert_eq!(sym!(p({ &node })).unwrap(), term("p(node(3))"));
    assert_eq!(sym!(p({ pair })).unwrap(), term("p((1,2))"));
    assert_eq!(sym!(p({ list })).unwrap(), term("p((4,5))"));
    assert_eq!(sym!(({ n }, { n })).unwrap(), term("(42,42)"));
    assert_eq!(sym!(-p({ n })).unwrap(), term("-p(42)"));
    // The splices do not consume their values.
    assert_eq!(list.len(), 2);
    assert_eq!(node.0, 3);
}

#[test]
fn splices_are_evaluated_once_from_left_to_right() {
    let counter = Cell::new(0);
    let next = || {
        counter.set(counter.get() + 1);
        counter.get()
    };
    let symbol = sym!(p({ next() }, q({ next() }), ({ next() }, { next() }))).unwrap();
    assert_eq!(symbol, term("p(1,q(2),(3,4))"));
    assert_eq!(counter.get(), 4);
}

#[test]
fn a_failing_splice_returns_its_error() {
    let err = sym!(p({ u32::MAX })).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    let err = sym!(p(1, { i64::MAX }, 3)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
}

fn spliced(name: &str) -> clingox::Result<Symbol> {
    // `?` inside a splice applies to this function, not to the macro.
    sym!(p({ Symbol::function(name, &[])? }))
}

#[test]
fn splices_may_use_the_question_mark_operator_of_the_caller() {
    assert_eq!(spliced("c").unwrap(), term("p(c)"));
    assert_eq!(spliced("c\0d").unwrap_err().kind(), ErrorKind::Nul);
}

#[test]
fn a_string_with_a_nul_byte_is_a_nul_error() {
    assert_eq!(sym!(p("a\0b")).unwrap_err().kind(), ErrorKind::Nul);
}

mod through_the_prelude {
    #![forbid(unsafe_code)]

    use clingox::prelude::*;

    #[test]
    fn sym_is_in_the_prelude() {
        let symbol: Symbol = sym!(p(1)).unwrap();
        assert_eq!(symbol.to_string(), "p(1)");
    }
}

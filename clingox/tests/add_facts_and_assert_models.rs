//! `add_facts`, `sym!` splices and `assert_models!`.
//!
//! Every expected value about clingo's behaviour was checked against the
//! Python module `clingo` 5.8.2: how it prints strings with quotes,
//! backslashes, newlines, tabs and non-ASCII text, and that each such
//! `p("..").` fact reads back as the same atom, as do `i32::MIN` and
//! `i32::MAX`.

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

use clingox::testing::assert_models;
use clingox::{Control, FromSymbol, OwnedModel, Part, Symbol, ToSymbol, sym};

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Note(#[clingo(string)] String);

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Count(i32);

fn first_model(ctl: &mut Control) -> OwnedModel {
    ctl.ground(&[Part::base()]).expect("the program grounds");
    let (_, models) = ctl.solve_all().expect("the search succeeds");
    models.into_iter().next().expect("the program has a model")
}

// ---------------------------------------------------------------------------
// add_facts writes symbols as program text; clingo must read each back as
// the same atom.

#[test]
fn strings_with_escapes_and_non_ascii_text_survive_add_facts() {
    let texts = [
        "a\"b",
        "a\\b",
        "a\nb",
        "a\tb",
        "a\rb",
        "héllo ✓",
        "\\n",
        "a\\",
        "",
        " ",
        "\\\"",
        "{x}",
        "%x",
        "a.b",
        ":-",
    ];
    let mut ctl = Control::new().expect("a control can be created");
    ctl.add_facts(texts.iter().map(|t| Note((*t).to_owned())))
        .expect("every string is a valid fact");
    let model = first_model(&mut ctl);
    let mut read: Vec<String> = model
        .atoms::<Note>()
        .expect("the atoms convert")
        .into_iter()
        .map(|n| n.0)
        .collect();
    let mut expected: Vec<String> = texts.iter().map(|t| (*t).to_owned()).collect();
    read.sort();
    expected.sort();
    assert_eq!(read, expected);
}

#[test]
fn the_extreme_numbers_survive_add_facts() {
    let mut ctl = Control::new().expect("a control can be created");
    ctl.add_facts([Count(i32::MIN), Count(0), Count(i32::MAX)])
        .expect("every number is a valid fact");
    let model = first_model(&mut ctl);
    assert_eq!(
        model.atoms::<Count>().expect("the atoms convert"),
        [Count(i32::MIN), Count(0), Count(i32::MAX)]
    );
}

// ---------------------------------------------------------------------------
// sym!: the bindings that hold splice values must not capture the caller's
// names (RULES 11.5).

#[test]
fn a_splice_sees_the_callers_variables_not_the_macros() {
    #[allow(
        clippy::no_effect_underscore_binding,
        reason = "the name is the one the macro gives its first splice"
    )]
    let __clingox_splice0 = 5;
    let symbol = sym!(p({ 1 }, { __clingox_splice0 })).expect("the symbol builds");
    assert_eq!(symbol, "p(1,5)".parse::<Symbol>().expect("the term parses"));
}

// ---------------------------------------------------------------------------
// assert_models!: the panic must report the caller's file and
// line, which includes the panic for an expected line that does not parse.

thread_local! {
    static LAST_PANIC: RefCell<Option<(String, u32)>> = const { RefCell::new(None) };
}

fn record_panic_locations() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if let Some(location) = info.location() {
                let place = (location.file().to_owned(), location.line());
                LAST_PANIC.with(|last| *last.borrow_mut() = Some(place));
            }
            previous(info);
        }));
    });
}

#[test]
fn a_line_that_does_not_parse_reports_the_callers_line() {
    record_panic_locations();
    let mut ctl = Control::new().expect("a control can be created");
    ctl.add_base("a.").expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    let (_, models) = ctl.solve_all().expect("the search succeeds");
    let line = line!() + 1;
    let result = panic::catch_unwind(AssertUnwindSafe(|| assert_models!(models, ["a("])));
    assert!(
        result.is_err(),
        "assert_models! must fail on a line that does not parse"
    );
    let (file, reported) = LAST_PANIC
        .with(|last| last.borrow_mut().take())
        .expect("assert_models! panicked");
    assert!(
        file.ends_with("add_facts_and_assert_models.rs"),
        "{file}:{reported}"
    );
    assert_eq!(reported, line);
}

// ---------------------------------------------------------------------------
// The derives in a module without the prelude, beside items that shadow the
// names the generated code uses. It is a compile check: the module must
// build.

#[allow(
    dead_code,
    non_snake_case,
    reason = "the shadowing items exist only to be in scope"
)]
mod without_the_prelude {
    #![no_implicit_prelude]

    pub(crate) type Result<T> = ::core::result::Result<T, ()>;
    pub(crate) struct Symbol;
    pub(crate) struct Error;
    pub(crate) enum Option<T> {
        Some(T),
        None,
    }
    pub(crate) fn Ok() {}
    pub(crate) mod clingox {}
    pub(crate) mod core {}
    pub(crate) mod std {}

    #[derive(::clingox::ToSymbol, ::clingox::FromSymbol)]
    pub(crate) struct Edge {
        pub(crate) symbol: i32,
        pub(crate) __argument0: i32,
    }

    #[derive(::clingox::ToSymbol, ::clingox::FromSymbol)]
    pub(crate) enum Shape {
        Point,
        Named(#[clingo(constant)] ::std::string::String),
        Rect { w: i32, h: i32 },
    }

    #[derive(::clingox::ToSymbol, ::clingox::FromSymbol)]
    pub(crate) struct Pair<Symbol, Result>(pub(crate) Symbol, pub(crate) Result);

    pub(crate) fn build(n: u64) -> ::clingox::Result<::clingox::Symbol> {
        ::clingox::sym!(p({ n }, (1,), (), #sup, #inf, -q, "s"))
    }
}

#[test]
fn derived_code_ignores_shadowed_names() {
    use without_the_prelude::{Edge, Pair, Shape, build};

    let edge = Edge {
        symbol: 1,
        __argument0: 2,
    };
    let symbol = edge.to_symbol().expect("the edge converts");
    assert_eq!(symbol.to_string(), "edge(1,2)");
    let back = Edge::from_symbol(symbol).expect("the edge reads back");
    assert_eq!((back.symbol, back.__argument0), (1, 2));
    let shape = Shape::Named("c".to_owned())
        .to_symbol()
        .expect("the shape converts");
    assert_eq!(shape.to_string(), "named(c)");
    assert!(matches!(Shape::from_symbol(shape), Ok(Shape::Named(name)) if name == "c"));
    let pair = Pair(1, (2, 3)).to_symbol().expect("the pair converts");
    assert_eq!(pair.to_string(), "pair(1,(2,3))");
    assert_eq!(
        build(7).expect("the symbol builds").to_string(),
        r#"p(7,(1,),(),#sup,#inf,-q,"s")"#
    );
}

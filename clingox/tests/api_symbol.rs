//! Building, inspecting, parsing, printing and ordering symbols.

#![forbid(unsafe_code)]

use std::collections::HashSet;

use clingox::{ErrorKind, Sign, Symbol, SymbolKind};

#[test]
fn numbers_round_trip() {
    let n = Symbol::number(42);
    assert_eq!(n.as_number(), Some(42));
    assert_eq!(n.to_string(), "42");
    assert_eq!(n.name(), None);
    assert_eq!(n.as_string(), None);
    assert_eq!(Symbol::number(-3).to_string(), "-3");
    assert_eq!(Symbol::from(7), Symbol::number(7));
}

#[test]
fn strings_round_trip() {
    let s = Symbol::string("hello").unwrap();
    assert_eq!(s.as_string(), Some("hello"));
    assert_eq!(s.to_string(), "\"hello\"");
    assert_eq!(s.as_number(), None);
}

#[test]
fn functions_have_a_name_arguments_and_a_sign() {
    let p = Symbol::function("p", &[Symbol::number(1), Symbol::string("x").unwrap()]).unwrap();
    assert_eq!(p.name(), Some("p"));
    assert_eq!(
        p.arguments().unwrap(),
        &[Symbol::number(1), Symbol::string("x").unwrap()]
    );
    assert_eq!(p.sign(), Some(Sign::Positive));
    assert_eq!(p.to_string(), "p(1,\"x\")");
}

#[test]
fn constants_are_functions_without_arguments() {
    let c = Symbol::function("c", &[]).unwrap();
    assert_eq!(c.name(), Some("c"));
    assert_eq!(c.arguments(), Some(&[][..]));
    assert_eq!(c.to_string(), "c");
}

#[test]
fn negative_functions_print_with_a_minus() {
    let q = Symbol::function_with_sign("q", &[], Sign::Negative).unwrap();
    assert_eq!(q.sign(), Some(Sign::Negative));
    assert_eq!(q.to_string(), "-q");
}

#[test]
fn tuples_are_functions_with_an_empty_name() {
    let t = Symbol::tuple(&[Symbol::number(1), Symbol::number(2)]).unwrap();
    assert_eq!(t.name(), Some(""));
    assert_eq!(t.to_string(), "(1,2)");
}

#[test]
fn supremum_and_infimum_print_like_clingo() {
    assert_eq!(Symbol::supremum().to_string(), "#sup");
    assert_eq!(Symbol::infimum().to_string(), "#inf");
    assert!(matches!(Symbol::supremum().kind(), SymbolKind::Supremum));
    assert!(matches!(Symbol::infimum().kind(), SymbolKind::Infimum));
}

#[test]
fn kind_describes_the_symbol() {
    assert!(matches!(Symbol::number(3).kind(), SymbolKind::Number(3)));
    assert!(matches!(
        Symbol::string("s").unwrap().kind(),
        SymbolKind::String("s")
    ));
    match Symbol::function("f", &[Symbol::number(1)]).unwrap().kind() {
        SymbolKind::Function {
            name,
            arguments,
            sign,
        } => {
            assert_eq!(name, "f");
            assert_eq!(arguments, &[Symbol::number(1)]);
            assert_eq!(sign, Sign::Positive);
        }
        other => panic!("expected a function, got {other:?}"),
    }
}

#[test]
fn parsing_gives_the_same_symbol_as_building() {
    let built = Symbol::function("p", &[Symbol::number(1), Symbol::string("x").unwrap()]).unwrap();
    let parsed: Symbol = "p(1,\"x\")".parse().unwrap();
    assert_eq!(parsed, built);
}

#[test]
fn a_term_with_a_syntax_error_does_not_parse() {
    let err = "p(".parse::<Symbol>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
}

#[test]
fn a_string_with_nul_is_rejected() {
    let err = Symbol::string("a\0b").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    let err = Symbol::function("a\0b", &[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
}

/// The expected order was taken from clingo 5.8.2 itself (Python `sorted`), and
/// it differs from any order on the raw 64-bit representation: numbers are
/// ordered numerically, and constants come before strings, which come before
/// functions with arguments.
#[test]
fn symbols_order_like_clingo() {
    let p = |n| Symbol::function("p", &[Symbol::number(n)]).unwrap();
    let expected = vec![
        Symbol::infimum(),
        Symbol::number(-3),
        Symbol::number(2),
        Symbol::number(10),
        Symbol::function("c", &[]).unwrap(),
        Symbol::function_with_sign("q", &[], Sign::Negative).unwrap(),
        Symbol::string("a").unwrap(),
        Symbol::string("b").unwrap(),
        p(1),
        p(2),
        Symbol::tuple(&[Symbol::number(1), Symbol::number(2)]).unwrap(),
        Symbol::supremum(),
    ];
    let mut shuffled = expected.clone();
    shuffled.reverse();
    shuffled.swap(0, 5);
    shuffled.swap(3, 8);
    shuffled.sort();
    assert_eq!(shuffled, expected);
}

#[test]
fn equal_symbols_hash_equally() {
    let a: Symbol = "f(1)".parse().unwrap();
    let b = Symbol::function("f", &[Symbol::number(1)]).unwrap();
    let set: HashSet<Symbol> = [a, b].into_iter().collect();
    assert_eq!(set.len(), 1);
}

#[test]
fn debug_shows_the_clingo_text() {
    let p = Symbol::function("p", &[Symbol::number(1), Symbol::string("x").unwrap()]).unwrap();
    assert_eq!(format!("{p:?}"), "Symbol(p(1,\"x\"))");
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn symbols_can_be_shared_between_threads() {
    fn assert_traits<
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
    assert_traits::<Symbol>();

    let s = Symbol::function("shared", &[Symbol::number(1)]).unwrap();
    let from_thread = std::thread::spawn(move || {
        let made_there = Symbol::function("shared", &[Symbol::number(1)]).unwrap();
        (s, made_there)
    })
    .join()
    .unwrap();
    assert_eq!(from_thread.0, from_thread.1);
}

// ---------------------------------------------------------------------------
// symbol equality compares values

#[test]
fn symbols_compare_by_value() {
    let parse = |t: &str| t.parse::<Symbol>().expect("the test term parses");
    let pairs = [
        ("p(1)", "p(2)"),
        ("p", "-p"),
        ("\"a\"", "a"),
        ("1", "\"1\""),
        ("()", "(1,)"),
        ("#inf", "#sup"),
        ("f(a)", "f(b)"),
        ("f(g(1))", "f(g(2))"),
        ("p(1,2)", "p(2,1)"),
        ("p", "q"),
        ("0", "#inf"),
    ];
    for (a, b) in pairs {
        let (a, b) = (parse(a), parse(b));
        assert_ne!(a, b, "{a} and {b}");
        assert_ne!(a.cmp(&b), std::cmp::Ordering::Equal, "{a} and {b}");
        assert_eq!(a, parse(&a.to_string()));
    }
    assert_ne!(
        [parse("a"), parse("b")].as_slice(),
        [parse("a"), parse("c")].as_slice()
    );
}

#[test]
fn equal_symbols_built_in_different_ways_are_equal() {
    let built = Symbol::function("p", &[Symbol::number(1), Symbol::string("x").unwrap()]).unwrap();
    assert_eq!(built, "p(1,\"x\")".parse::<Symbol>().unwrap());
    let negated = Symbol::function_with_sign("q", &[], Sign::Negative).unwrap();
    assert_eq!(negated, "-q".parse::<Symbol>().unwrap());
    assert_ne!(negated, Symbol::function("q", &[]).unwrap());
}

// ---------------------------------------------------------------------------
// clingo's internal symbol type

#[test]
fn symbol_kind_has_a_variant_for_clingos_internal_symbols() {
    // clingo's "special" symbol (type 6, `libgringo/src/symbol.cc:78`) cannot
    // be built through the public API, so no test can obtain one; the variant
    // exists so that `kind()` never reports such a symbol as `#inf` and never
    // panics. Every symbol clingox can build has one of the five public kinds.
    let other = SymbolKind::Other;
    assert_ne!(other, SymbolKind::Infimum);
    assert_ne!(other, SymbolKind::Supremum);
    for text in ["#inf", "#sup", "1", "\"s\"", "p(1)", "-p", "()", "(1,2)"] {
        let symbol: Symbol = text.parse().unwrap();
        assert_ne!(symbol.kind(), SymbolKind::Other, "{text}");
    }
}

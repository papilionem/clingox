//! A string in a theory atom is a string symbol (smoke finding A2).
//!
//! clingo hands a theory term for `"str"` to the caller as a symbol term whose
//! *name* is the quoted, escaped text (`"str"` with the quotes). The term must
//! become `Symbol::string("str")`, the value the grounder itself uses for the
//! string, and not a constant whose name happens to contain quote characters.
//! Expected values are pyclingo 5.8.2 (`TheoryTerm.name` is the quoted, escaped
//! text, `str(term)` the same text the terms print with here), checked
//! 2026-09-29.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail loudly on unexpected errors"
)]

use clingox::{Control, Part, Symbol, SymbolKind, TheoryTerm, TheoryTermKind};

const THEORY: &str = "#theory t { term { }; &a/0 : term, head }.";

/// The terms of the first element of the first theory atom of `program`,
/// rendered with their display text, with symbol terms also as symbols.
fn terms(program: &str) -> Vec<(String, Option<Symbol>)> {
    let mut ctl = Control::new().unwrap();
    ctl.add("base", &[], &format!("{THEORY}\n{program}"))
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let elements = atom.elements().unwrap();
    let mut out = Vec::new();
    for &id in elements[0].tuple().unwrap() {
        let term = atoms.term(id).unwrap();
        let symbol = match &term {
            TheoryTerm::Symbol(symbol) => Some(*symbol),
            _ => None,
        };
        out.push((term.to_string(), symbol));
    }
    out
}

fn string(text: &str) -> Symbol {
    Symbol::string(text).unwrap()
}

#[test]
fn a_quoted_string_is_a_string_symbol() {
    let got = terms(r#"&a { "str" }."#);
    assert_eq!(got.len(), 1);
    let symbol = got[0].1.expect("a symbol term");
    assert_eq!(symbol, string("str"));
    assert_eq!(symbol.kind(), SymbolKind::String("str"));
    assert_eq!(symbol.as_string(), Some("str"));
    assert_eq!(got[0].0, r#""str""#);
}

#[test]
fn a_string_is_not_the_constant_with_the_same_text() {
    let got = terms(r#"&a { "c", c }."#);
    assert_eq!(got[0].1, Some(string("c")));
    assert_eq!(got[1].1, Some(Symbol::function("c", &[]).unwrap()));
    assert_ne!(got[0].1, got[1].1);
    assert!(matches!(
        got[1].1.unwrap().kind(),
        SymbolKind::Function { .. }
    ));
}

/// Quotes, backslashes, newlines, the empty string and non-ASCII text, each
/// with the text pyclingo prints for the term.
#[test]
fn strings_round_trip_through_their_escapes() {
    let cases: [(&str, &str, &str); 8] = [
        // source text, the string's value, pyclingo's str(term)
        (
            r#""with \"quote\"""#,
            "with \"quote\"",
            r#""with \"quote\"""#,
        ),
        (r#""back\\slash""#, "back\\slash", r#""back\\slash""#),
        (r#""new\nline""#, "new\nline", r#""new\nline""#),
        (r#""""#, "", r#""""#),
        (r#""uni é""#, "uni é", r#""uni é""#),
        // A backslash followed by the letter n is not a newline.
        (r#""a\\nb""#, "a\\nb", r#""a\\nb""#),
        // A backslash followed by a quote.
        (r#""\\\"""#, "\\\"", r#""\\\"""#),
        (r#""tab	here""#, "tab\there", "\"tab\there\""),
    ];
    for (source, value, shown) in cases {
        let got = terms(&format!("&a {{ {source} }}."));
        assert_eq!(got.len(), 1, "{source}");
        assert_eq!(got[0].1, Some(string(value)), "{source}");
        assert_eq!(got[0].0, shown, "{source}");
        // The symbol prints as the term does.
        assert_eq!(got[0].1.unwrap().to_string(), shown, "{source}");
    }
}

#[test]
fn several_strings_and_constants_keep_their_order_and_kinds() {
    let got = terms(r#"&a { "str", c, "with \"quote\"", "" }."#);
    let strings: Vec<Option<&str>> = got.iter().map(|(_, s)| s.unwrap().as_string()).collect();
    assert_eq!(
        strings,
        [Some("str"), None, Some("with \"quote\""), Some("")]
    );
}

#[test]
fn strings_inside_compound_terms_are_string_symbols() {
    let program =
        format!("{THEORY}\n&a {{ f(\"x\", g, \"y\\\\z\"), (\"t\",), [\"l\"], {{\"s\"}} }}.");
    let mut ctl = Control::new().unwrap();
    ctl.add("base", &[], &program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let elements = atom.elements().unwrap();
    let ids = elements[0].tuple().unwrap();
    let all: Vec<TheoryTerm<'_>> = ids.iter().map(|&id| atoms.term(id).unwrap()).collect();

    let TheoryTerm::Compound {
        kind: TheoryTermKind::Function,
        name: Some("f"),
        arguments,
    } = &all[0]
    else {
        panic!("f(..) expected, got {:?}", all[0]);
    };
    assert_eq!(arguments[0], TheoryTerm::Symbol(string("x")));
    assert_eq!(
        arguments[1],
        TheoryTerm::Symbol(Symbol::function("g", &[]).unwrap())
    );
    assert_eq!(arguments[2], TheoryTerm::Symbol(string("y\\z")));
    for (index, text) in [(1, "t"), (2, "l"), (3, "s")] {
        let TheoryTerm::Compound { arguments, .. } = &all[index] else {
            panic!("a compound term expected, got {:?}", all[index]);
        };
        assert_eq!(arguments, &[TheoryTerm::Symbol(string(text))]);
    }
}

/// The same string in a ground atom and in a theory atom is one symbol, so a
/// caller can look a theory string up among the program's own symbols.
#[test]
fn a_theory_string_equals_the_string_the_program_derives() {
    let mut ctl = Control::new().unwrap();
    ctl.add(
        "base",
        &[],
        &format!("{THEORY}\nname(\"a \\\"b\\\"\\n\"). &a {{ \"a \\\"b\\\"\\n\" }}."),
    )
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let from_program = {
        let atoms = ctl.symbolic_atoms().unwrap();
        let sig = clingox::Signature::new("name", 1).unwrap();
        let found: Vec<Symbol> = atoms
            .by_signature(sig)
            .map(|a| a.unwrap().symbol())
            .collect();
        found[0].arguments().unwrap()[0]
    };
    let theory = ctl.theory_atoms().unwrap();
    let atom = theory.iter().next().unwrap().unwrap();
    let id = atom.elements().unwrap()[0].tuple().unwrap()[0];
    assert_eq!(
        theory.term(id).unwrap(),
        TheoryTerm::Symbol(from_program),
        "the theory string and the program's string are one symbol"
    );
}

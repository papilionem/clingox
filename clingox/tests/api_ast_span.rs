//! `Span::new`, the public constructor of an AST location.
//!
//! Oracle: pyclingo 5.8.2 (`clingo.ast.Location`/`Position`, built into nodes
//! and read back) and `clingo.Control` log messages for the `Display` forms
//! (`<block>:1:1-2:12`, `<string>:1:8-9`). clingo stores line and column as
//! `unsigned` and truncates silently (a line of 2**40 reads back as 0), so
//! `Span::new` rejects anything above `u32::MAX`. The tests need a real
//! clingo (string interning), so none runs under Miri.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::collections::HashSet;

use clingox::ErrorKind;
use clingox::ast::{self, Attribute, Span};

#[test]
fn the_six_components_are_read_back() {
    let span = Span::new("first.lp", 3, 4, "second.lp", 5, 6).unwrap();
    assert_eq!(span.begin_file(), "first.lp");
    assert_eq!(span.begin_line(), 3);
    assert_eq!(span.begin_column(), 4);
    assert_eq!(span.end_file(), "second.lp");
    assert_eq!(span.end_line(), 5);
    assert_eq!(span.end_column(), 6);
    assert!(!span.is_single_file());
    assert!(Span::new("f", 1, 1, "f", 2, 2).unwrap().is_single_file());
}

#[test]
fn file_names_are_interned_for_the_whole_process() {
    // Two separately allocated, equal names come back as the same `'static`
    // string, which is what lets `begin_file` return `&'static str`.
    let one = Span::new(&String::from("shared.lp"), 1, 1, "shared.lp", 1, 1).unwrap();
    let two = Span::new(&["shared", ".lp"].concat(), 9, 9, "other.lp", 9, 9).unwrap();
    assert_eq!(one.begin_file().as_ptr(), two.begin_file().as_ptr());
    let name: &'static str = one.begin_file();
    assert_eq!(name, "shared.lp");
}

#[test]
fn nothing_but_the_range_of_the_numbers_is_checked() {
    // Oracle: clingo takes line 0, an end before the begin, two files and a
    // non-ASCII or empty name without complaint, and so does `Span::new`.
    for span in [
        Span::new("", 0, 0, "", 0, 0).unwrap(),
        Span::new("a", 5, 9, "b", 1, 1).unwrap(),
        Span::new("é ü", 1, 1, "é ü", 1, 2).unwrap(),
        Span::new("x", 9, 9, "x", 1, 1).unwrap(),
    ] {
        let node = ast::id(&span, "x").unwrap();
        assert_eq!(node.span(Attribute::Location).unwrap(), span);
    }
}

#[test]
fn components_up_to_u32_max_are_accepted_and_survive_clingo() {
    let max = usize::try_from(u32::MAX).unwrap();
    let span = Span::new("x", max, max, "x", max, max).unwrap();
    let node = ast::id(&span, "x").unwrap();
    let back = node.span(Attribute::Location).unwrap();
    assert_eq!((back.begin_line(), back.end_column()), (max, max));
    assert_eq!(back, span);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn a_component_above_u32_max_is_invalid_input() {
    // Oracle: clingo would store a line of 2**40 as 0.
    let over = usize::try_from(u64::from(u32::MAX) + 1).unwrap();
    let ok = 1;
    for (bl, bc, el, ec) in [
        (over, ok, ok, ok),
        (ok, over, ok, ok),
        (ok, ok, over, ok),
        (ok, ok, ok, over),
    ] {
        let err = Span::new("x", bl, bc, "x", el, ec).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{bl} {bc} {el} {ec}");
    }
    let err = Span::new("x", usize::MAX, 1, "x", 1, 1).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[test]
fn a_nul_byte_in_either_file_name_is_a_nul_error() {
    assert_eq!(
        Span::new("a\0b", 1, 1, "c", 1, 1).unwrap_err().kind(),
        ErrorKind::Nul
    );
    assert_eq!(
        Span::new("a", 1, 1, "c\0", 1, 1).unwrap_err().kind(),
        ErrorKind::Nul
    );
}

#[test]
fn equal_spans_are_equal_and_hash_alike() {
    let a = Span::new("f.lp", 1, 2, "f.lp", 3, 4).unwrap();
    let b = Span::new(&String::from("f.lp"), 1, 2, "f.lp", 3, 4).unwrap();
    let c = Span::new("f.lp", 1, 2, "f.lp", 3, 5).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, c);
    let set: HashSet<Span> = [a.clone(), b, c].into_iter().collect();
    assert_eq!(set.len(), 2);
    assert!(set.contains(&a));
}

#[test]
fn display_uses_gringos_forms() {
    // A point, one line of one file, several lines, and two files. The
    // one-line form is the oracle's `<string>:1:8-9`, the several-line form
    // its `<block>:1:1-2:12`; the point form is the same format with equal
    // ends; the two-file form is the documented format.
    assert_eq!(
        Span::new("<t>", 1, 2, "<t>", 1, 2).unwrap().to_string(),
        "<t>:1:2"
    );
    assert_eq!(
        Span::new("<string>", 1, 8, "<string>", 1, 9)
            .unwrap()
            .to_string(),
        "<string>:1:8-9"
    );
    assert_eq!(
        Span::new("<block>", 1, 1, "<block>", 2, 12)
            .unwrap()
            .to_string(),
        "<block>:1:1-2:12"
    );
    assert_eq!(
        Span::new("a.lp", 1, 2, "b.lp", 3, 4).unwrap().to_string(),
        "a.lp:1:2-b.lp:3:4"
    );
}

#[test]
fn a_parsed_span_and_a_built_one_agree() {
    // The span read from a parsed node can be passed straight to a
    // constructor and comes back equal.
    let mut nodes = Vec::new();
    ast::parse_string("a.", |node| {
        nodes.push(node);
        Ok(())
    })
    .unwrap();
    let parsed = nodes[1].span(Attribute::Location).unwrap();
    let node = ast::id(&parsed, "x").unwrap();
    assert_eq!(node.span(Attribute::Location).unwrap(), parsed);
    assert_eq!(parsed.begin_file(), "<string>");
}

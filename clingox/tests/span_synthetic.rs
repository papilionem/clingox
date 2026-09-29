//! `Span::synthetic` (smoke finding B9): the location for a node a program
//! builds without source text, the clingo crate's `Location::default()`.
//!
//! The decision (smoke-notes.md, 5): file `<generated>`, line 1, column 1,
//! for the beginning and the end alike. clingo takes any location, so the
//! span goes into a node and comes back unchanged.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use clingox::ast::{self, Attribute, Span};
use clingox::{Control, Part};

#[test]
fn the_synthetic_span_is_generated_at_line_one_column_one() {
    let span = Span::synthetic();
    assert_eq!(span.begin_file(), "<generated>");
    assert_eq!(span.end_file(), "<generated>");
    assert_eq!((span.begin_line(), span.begin_column()), (1, 1));
    assert_eq!((span.end_line(), span.end_column()), (1, 1));
    assert!(span.is_single_file());
}

#[test]
fn it_equals_the_span_built_by_hand() {
    assert_eq!(
        Span::synthetic(),
        Span::new("<generated>", 1, 1, "<generated>", 1, 1).unwrap()
    );
    assert_eq!(Span::synthetic(), Span::synthetic());
}

#[test]
fn a_node_built_with_it_carries_it_and_the_program_takes_it() {
    let span = Span::synthetic();
    let node = ast::id(&span, "x").unwrap();
    assert_eq!(node.span(Attribute::Location).unwrap(), span);

    // A program made of generated nodes still grounds.
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve_all().unwrap().0.is_sat());
}

#[test]
fn it_prints_like_the_span_of_a_generated_file() {
    assert_eq!(Span::synthetic().to_string(), "<generated>:1:1");
}

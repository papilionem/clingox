// Enum-valued number attributes are typed: clingo accepts any integer for
// them and misprints (a sign of 7 prints as no sign), so a bare integer where
// `LiteralSign` is expected must not compile.
use clingox::ast::{self, Span};

fn main() {
    let span = Span::new("f", 1, 1, "f", 1, 1).unwrap();
    let term = ast::id(&span, "x").unwrap();
    let atom = ast::symbolic_atom(&term).unwrap();
    let _ = ast::literal(&span, 1, &atom);
}

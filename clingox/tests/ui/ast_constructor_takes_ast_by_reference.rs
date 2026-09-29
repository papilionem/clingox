// A generated constructor borrows its node arguments: clingo takes its own
// reference (`SAST(AST*)` increments the count, `astv2.cc:253`), so the
// caller's handle stays valid. An owned `Ast` where `&Ast` is expected must
// not compile, or a caller would believe the handle was consumed.
use clingox::ast::{self, Span};

fn main() {
    let span = Span::new("f", 1, 1, "f", 1, 1).unwrap();
    let head = ast::id(&span, "x").unwrap();
    let _ = ast::rule(&span, head, &[]);
}

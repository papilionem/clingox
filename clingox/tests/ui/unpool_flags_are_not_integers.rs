// `Unpool` is a type with private contents, like `ShowType`: the raw integers
// clingo's C API takes (or an `i32` that might carry bits clingo would mask)
// cannot be passed where the flags are expected.
// Expected: E0308, mismatched types (expected `Unpool`, found integer).
use clingox::ast::{self, AstType};

fn main() {
    ast::parse_string("p(1;2).", |node| {
        if node.ast_type() == AstType::Rule {
            node.unpool(3, |_| Ok(()))?;
        }
        Ok(())
    })
    .unwrap();
}

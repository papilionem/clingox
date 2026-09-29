// A `String` field is a clingo string or a constant, never both. Expected: an
// error from the derive at the second key.
#![forbid(unsafe_code)]

use clingox::ToSymbol;

#[derive(ToSymbol)]
struct Label {
    #[clingo(string, constant)]
    text: String,
}

fn main() {}

// A predicate name must be a clingo identifier; `Edge` would read as a
// variable. Expected: an error from the derive at the string `"Edge"`.
#![forbid(unsafe_code)]

use clingox::ToSymbol;

#[derive(ToSymbol)]
#[clingo(name = "Edge")]
struct Link(i32, i32);

fn main() {
    let _ = Link(1, 2).to_symbol();
}

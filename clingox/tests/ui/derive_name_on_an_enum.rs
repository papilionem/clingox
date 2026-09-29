// Each variant of an enum is its own term, so the enum takes no name.
// Expected: an error from the derive at the attribute on the enum.
#![forbid(unsafe_code)]

use clingox::ToSymbol;

#[derive(ToSymbol)]
#[clingo(name = "shape")]
enum Shape {
    Point,
}

fn main() {}

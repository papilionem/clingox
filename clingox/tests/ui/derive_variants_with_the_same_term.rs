// Two variants that are both `circle/1` could not be told apart when reading.
// Expected: an error from the derive at the second variant.
#![forbid(unsafe_code)]

use clingox::FromSymbol;

#[derive(FromSymbol)]
enum Shape {
    Circle(i32),
    #[clingo(name = "circle")]
    Round(i32),
}

fn main() {}

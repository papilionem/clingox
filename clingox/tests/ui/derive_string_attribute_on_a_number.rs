// `#[clingo(string)]` and `#[clingo(constant)]` apply only to `String` fields.
// Expected: an error from the derive at the attribute on the field `value`.
#![forbid(unsafe_code)]

use clingox::FromSymbol;

#[derive(FromSymbol)]
struct Reading {
    #[clingo(string)]
    value: i32,
}

fn main() {
    let _ = Reading::from_symbol(clingox::Symbol::number(1));
}

// clingo has no floating-point values, so `f64` implements neither trait.
// Expected: the unsatisfied bound `f64: ToSymbol` at the field `weight` or its
// type, with the trait's `on_unimplemented` message.
#![forbid(unsafe_code)]

use clingox::ToSymbol;

#[derive(ToSymbol)]
struct Measurement {
    weight: f64,
}

fn main() {
    let _ = Measurement { weight: 1.5 }.to_symbol();
}

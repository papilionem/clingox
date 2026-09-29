// A `String` field must say whether it is a clingo string or a constant: the
// two never match each other in rules. Expected: an
// error from the derive at the field `object`, naming `#[clingo(string)]` and
// `#[clingo(constant)]`.
#![forbid(unsafe_code)]

use clingox::ToSymbol;

#[derive(ToSymbol)]
struct Asserted {
    object: String,
    value: i32,
}

fn main() {
    let _ = Asserted { object: String::new(), value: 0 }.to_symbol();
}

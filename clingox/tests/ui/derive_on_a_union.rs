// A union has no clingo form. Expected: an error from the derive at `union`.
#![forbid(unsafe_code)]

use clingox::ToSymbol;

#[derive(ToSymbol)]
union Bits {
    int: i32,
    small: u8,
}

fn main() {}

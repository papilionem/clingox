// An enum without variants has no values. Expected: an error from the derive
// at the enum's name.
#![forbid(unsafe_code)]

use clingox::FromSymbol;

#[derive(FromSymbol)]
enum Never {}

fn main() {}

// `#[clingo(..)]` takes `name` on a struct. Expected: an error from the
// derive at the unknown key `rename`.
#![forbid(unsafe_code)]

use clingox::ToSymbol;

#[derive(ToSymbol)]
#[clingo(rename = "edge")]
struct Link(i32, i32);

fn main() {}

// `-` applies to a name or an integer only. Expected: an error from `sym!` at
// the `-`.
#![forbid(unsafe_code)]

fn main() {
    let _ = clingox::sym!(p(-(1, 2)));
}

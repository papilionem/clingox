// A clingo number is a plain integer. Expected: an error from `sym!` at the
// literal `1u8`.
#![forbid(unsafe_code)]

fn main() {
    let _ = clingox::sym!(p(1u8));
}

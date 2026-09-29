// Classical negation applies to functions and constants, not to strings.
// Expected: an error from `sym!` at `-"x"` saying so.
#![forbid(unsafe_code)]

fn main() {
    let _ = clingox::sym!(p(-"x"));
}

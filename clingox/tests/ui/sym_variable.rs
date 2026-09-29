// A symbol is a ground term, so it has no variables. Expected: an error from
// `sym!` at `X`, saying that variables are not allowed.
#![forbid(unsafe_code)]

fn main() {
    let _ = clingox::sym!(p(X));
}

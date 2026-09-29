// Arguments are separated by commas. Expected: an error from `sym!` at the
// token `2`, saying that `,` or `)` was expected.
#![forbid(unsafe_code)]

fn main() {
    let _ = clingox::sym!(p(1 2));
}

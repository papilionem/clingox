// clingo numbers are 32-bit, and clingo 5.8.2 itself wraps `2147483648` to
// `-2147483648` silently. Expected: an error from `sym!` at the literal
// `2147483648`, saying it is outside the range of `i32`.
#![forbid(unsafe_code)]

fn main() {
    let _ = clingox::sym!(p(2147483648));
}

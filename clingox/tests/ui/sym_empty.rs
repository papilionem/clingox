// `sym!` takes one term. Expected: an error from `sym!` at the macro call.
#![forbid(unsafe_code)]

fn main() {
    let _ = clingox::sym!();
}

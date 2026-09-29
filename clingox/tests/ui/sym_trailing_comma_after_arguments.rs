// clingo rejects `p(1,)`, so `sym!` does too. Expected: an error from `sym!`
// at the trailing comma.
#![forbid(unsafe_code)]

fn main() {
    let _ = clingox::sym!(p(1,));
}

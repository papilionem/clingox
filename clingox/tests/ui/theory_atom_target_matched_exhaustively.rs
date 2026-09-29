// `TheoryAtomTarget` is `#[non_exhaustive]`, so a match on it from outside
// the crate can no longer be exhaustive without a wildcard arm: a later
// variant can be added to it without breaking every caller who matched it
// exhaustively.
//
#![forbid(unsafe_code)]

use clingox::backend::{Atom, TheoryAtomTarget};

fn describe(target: TheoryAtomTarget) -> &'static str {
    match target {
        TheoryAtomTarget::Directive => "directive",
        TheoryAtomTarget::Fresh => "fresh",
        TheoryAtomTarget::Atom(_atom) => "atom",
    }
}

fn main() {
    let _ = describe(TheoryAtomTarget::Directive);
    let _: fn(Atom) = |_| {};
}

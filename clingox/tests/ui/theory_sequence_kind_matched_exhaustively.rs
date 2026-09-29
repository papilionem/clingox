// `TheorySequenceKind` is `#[non_exhaustive]`, so a match on it from
// outside the crate can no longer be exhaustive without a wildcard arm: a
// later variant can be added to it without breaking every caller who
// matched it exhaustively.
//
#![forbid(unsafe_code)]

use clingox::backend::TheorySequenceKind;

fn describe(kind: TheorySequenceKind) -> &'static str {
    match kind {
        TheorySequenceKind::Tuple => "tuple",
        TheorySequenceKind::Set => "set",
        TheorySequenceKind::List => "list",
    }
}

fn main() {
    let _ = describe(TheorySequenceKind::Tuple);
}

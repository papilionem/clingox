// `GroundingLimit` is `#[non_exhaustive]` (with a `GroundingLimit::new`
// constructor), so it cannot be built with a struct literal from
// outside the crate: a later field can be added to it without breaking
// every caller.
//
#![forbid(unsafe_code)]

use clingox::observer::GroundingLimit;

fn main() {
    let _ = GroundingLimit {
        max_atoms: None,
        max_rules: None,
    };
}

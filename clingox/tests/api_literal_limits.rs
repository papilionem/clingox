//! Limits of `ProgramLiteral::from_raw`.
//!
//! clasp numbers its variables below 2^30 (`varMax`, clasp `literal.h`), so a
//! literal of larger magnitude can never name anything. Measurement
//! showed what clingo 5.8.2 does with one: `assign_external` and
//! `release_external` with `i32::MAX`, `-i32::MAX` or `i32::MIN` each grew the
//! process to about 19 GB over some 14 seconds and then reported "Id out of
//! range" (UPSTREAM-ISSUES U22). `from_raw` rejects every such value, so none
//! can reach clingo through clingox.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use clingox::ProgramLiteral;

#[test]
fn from_raw_rejects_zero_and_every_magnitude_clasp_cannot_represent() {
    for raw in [0, 1 << 30, -(1 << 30), i32::MAX, -i32::MAX, i32::MIN] {
        assert!(
            ProgramLiteral::from_raw(raw).is_none(),
            "{raw} was accepted"
        );
    }
}

#[test]
fn from_raw_accepts_the_largest_representable_magnitude_with_either_sign() {
    let max = ProgramLiteral::from_raw(ProgramLiteral::MAX_MAGNITUDE).unwrap();
    let neg_max = ProgramLiteral::from_raw(-ProgramLiteral::MAX_MAGNITUDE).unwrap();
    assert_eq!(max.get(), (1 << 28) - 1);
    assert_eq!(max.negate(), neg_max);
    assert_eq!(-neg_max, max);
}

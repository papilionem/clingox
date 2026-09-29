//! Regression test:
//! `ProgramLiteral::MAX_MAGNITUDE` was `2^30 - 1`, clasp's own limit on a
//! *variable*, but clasp treats a value of `2^28` and above as a body id
//! instead once it is used as a literal: `Model::is_true` on such a value
//! returned `ErrorKind::Logic`, and `Backend::add_rule` with such a value in
//! the body returned `ErrorKind::Runtime`.
//! `ProgramLiteral::MAX_MAGNITUDE` is `2^28 - 1`, matching clasp's atom
//! range, so a value in the old, wrongly-accepted range can no longer be
//! built at all.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use clingox::ProgramLiteral;

const CLASP_ATOM_RANGE: i32 = 1 << 28;

/// Oracle: clasp's own variable/atom split, `clasp/libclasp/clasp/literal.h`
/// (`varMax` bounds a *variable*, but a *body id* is distinguished from an
/// atom id starting at `2^28`, `clasp/src/logic_program.cpp`); checked
/// directly against clingo 5.8.2 through the Python module's C API,
/// 2026-09-27: `clingo_model_is_true` on a raw literal of `2^28` reports a
/// logic error, and `clingo_backend_rule` with `2^28 + 5` in the body
/// reports a runtime error, although both were `ProgramLiteral::MAX_MAGNITUDE`
/// (`2^30 - 1`) accepted them as ordinary literals.
#[test]
fn max_magnitude_matches_clasps_atom_range_not_its_variable_range() {
    assert_eq!(
        ProgramLiteral::MAX_MAGNITUDE,
        CLASP_ATOM_RANGE - 1,
        "MAX_MAGNITUDE must match clasp's atom range (2^28 - 1), not its wider variable range"
    );
}

#[test]
fn a_literal_at_the_new_boundary_is_accepted() {
    let at_boundary = ProgramLiteral::from_raw(CLASP_ATOM_RANGE - 1);
    assert!(
        at_boundary.is_some(),
        "2^28 - 1 is still a valid literal under the new, narrower bound"
    );
    let negative_at_boundary = ProgramLiteral::from_raw(-(CLASP_ATOM_RANGE - 1));
    assert!(negative_at_boundary.is_some());
}

#[test]
fn a_literal_in_clasps_body_id_range_is_rejected() {
    // These raw values were accepted before the fix (they are within the old
    // 2^30 - 1 bound) and reached clasp as literals that collide with its
    // body-id numbering, producing the errors the oracle above documents.
    for raw in [CLASP_ATOM_RANGE, CLASP_ATOM_RANGE + 5, -(CLASP_ATOM_RANGE)] {
        assert!(
            ProgramLiteral::from_raw(raw).is_none(),
            "{raw} is in clasp's body-id range and must be rejected at construction, \
             not accepted and left to fail later inside clingo: {:?}",
            ProgramLiteral::from_raw(raw)
        );
    }
}

/// A value comfortably inside clasp's *variable* range but outside its
/// *atom* range (the old bound) must also now be rejected, not only the
/// boundary value itself.
#[test]
fn a_literal_well_inside_the_old_bound_but_outside_the_new_one_is_rejected() {
    let old_bound_only = (1 << 29) + 12345;
    assert!(ProgramLiteral::from_raw(old_bound_only).is_none());
}

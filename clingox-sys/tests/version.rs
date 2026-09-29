//! The linked clingo library is the version these bindings were generated from.
//!
//! clingo keeps the same shared-library version across releases whose C API
//! differs, so a mismatch is only visible at run time (DESIGN §4).

use clingox_sys::*;

#[test]
fn linked_library_is_5_8_2() {
    let (mut major, mut minor, mut revision) = (0, 0, 0);
    // SAFETY: three valid out-pointers.
    unsafe { clingo_version(&raw mut major, &raw mut minor, &raw mut revision) };
    assert_eq!((major, minor, revision), (5, 8, 2));
}

#[test]
fn version_constant_matches_linked_library() {
    let (mut major, mut minor, mut revision) = (0, 0, 0);
    // SAFETY: three valid out-pointers.
    unsafe { clingo_version(&raw mut major, &raw mut minor, &raw mut revision) };
    let unsigned = |n: i32| u32::try_from(n).expect("version numbers are non-negative");
    let linked = (unsigned(major), unsigned(minor), unsigned(revision));
    assert_eq!(CLINGO_VERSION, linked);
}

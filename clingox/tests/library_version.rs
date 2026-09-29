//! `clingox::version`: the version of the clingo library that is linked at run
//! time (smoke finding B1, the clingo crate's `version()`).
//!
//! The header constant `clingox_sys::CLINGO_VERSION` is what the bindings
//! were generated from (pyclingo 5.8.2 reports (5, 8, 2)); `version` asks the
//! linked library itself, so the two agree when the library is the one the
//! bindings were generated from.

#![forbid(unsafe_code)]

#[test]
fn version_is_the_linked_library() {
    assert_eq!(clingox::version(), clingox_sys::CLINGO_VERSION);
}

#[test]
fn version_is_the_same_on_every_call() {
    assert_eq!(clingox::version(), clingox::version());
}

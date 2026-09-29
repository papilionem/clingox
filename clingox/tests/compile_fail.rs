//! Programs that must not compile, because they would break a safety rule,
//! and programs that must compile, because a rule allows them.

#![forbid(unsafe_code)]
// trybuild runs cargo to compile each case, which Android devices, iOS
// simulators and browsers do not have. The property it checks belongs to the host build anyway.
#![cfg(not(any(target_os = "android", target_os = "ios", target_family = "wasm")))]
#![allow(
    clippy::print_stderr,
    reason = "a skipped test says so, since a green result would otherwise look like a pass"
)]

/// The snapshots in `tests/ui` are exact rustc output, which changes from one
/// release to the next and shows standard-library excerpts only where the
/// `rust-src` component is installed. They are checked on the release in
/// `xtask/compile-fail-toolchain` by `cargo xtask test compile-fail`; a job on
/// any other setup sets `CLINGOX_SKIP_COMPILE_FAIL` to leave the tests out, so
/// a mismatch of wording is never mistaken for a broken safety rule. The
/// variable is read as set, whatever its value.
fn skipped() -> bool {
    let skip = std::env::var_os("CLINGOX_SKIP_COMPILE_FAIL").is_some();
    if skip {
        eprintln!("SKIPPED: CLINGOX_SKIP_COMPILE_FAIL is set");
    }
    skip
}

#[test]
fn misuse_does_not_compile() {
    if skipped() {
        return;
    }
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}

/// Uses that must compile, where an earlier contract forbade them (a `Control`
/// is `Send`).
#[test]
fn allowed_uses_compile() {
    if skipped() {
        return;
    }
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui_pass/*.rs");
}

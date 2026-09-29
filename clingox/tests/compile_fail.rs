//! Programs that must not compile, because they would break a safety rule,
//! and programs that must compile, because a rule allows them.

#![forbid(unsafe_code)]
// trybuild runs cargo to compile each case, which Android devices, iOS
// simulators and browsers do not have. The property it checks belongs to the host build anyway.
#![cfg(not(any(target_os = "android", target_os = "ios", target_family = "wasm")))]

#[test]
fn misuse_does_not_compile() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}

/// Uses that must compile, where an earlier contract forbade them (a `Control`
/// is `Send`).
#[test]
fn allowed_uses_compile() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui_pass/*.rs");
}

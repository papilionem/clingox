//! What clingo itself does when an options callback fails, read at the C level
//! through `clingox_sys`, independent of `Application`.
//!
//! These are the raw facts that the wrapper's contract rests on (pyclingo
//! 5.8.2, clingo 5.8.2, clasp 3.4.1, section 2):
//!
//! - a failing `validate_options` makes `clingo_main` return **0** (U39: clasp
//!   sets exit code 0 before the callback runs), which is why
//!   `Application::run` returns the callback's `Err` instead of an exit code;
//! - a failing `register_options` returns 1;
//! - a failing `parse` returns 1 and clingo discards the callback's message.
//!
//! A future clingo that fixes U39 or keeps the message turns the matching
//! assertion red, and the wrapper's documentation then needs an update. Every
//! case runs in a child process (`common/child.rs`): `clingo_main` installs
//! signal handlers and writes with C stdio.

#![allow(
    unsafe_code,
    reason = "the raw C API is the object of this file; nothing else here uses unsafe"
)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use std::ffi::{CString, c_char, c_void};

use clingox_sys as ffi;

const ENTRY: &str = "child_entry";

/// Sets clingo's error state to a runtime error and returns `false`.
fn fail(message: &std::ffi::CStr) -> bool {
    // SAFETY: `message` is a valid NUL-terminated string for the call, which
    // copies it.
    unsafe { ffi::clingo_set_error(ffi::clingo_error_runtime.cast_signed(), message.as_ptr()) };
    false
}

unsafe extern "C" fn parse_fails(_value: *const c_char, _data: *mut c_void) -> bool {
    fail(c"raw parse message")
}

unsafe extern "C" fn parse_ok(_value: *const c_char, _data: *mut c_void) -> bool {
    true
}

unsafe extern "C" fn register_adds_option(
    options: *mut ffi::clingo_options_t,
    _data: *mut c_void,
) -> bool {
    // SAFETY: `options` is the pointer clingo passes to the callback; the
    // strings are valid for the call and `parse_fails` needs no data.
    unsafe {
        ffi::clingo_options_add(
            options,
            c"Raw".as_ptr(),
            c"zzraw".as_ptr(),
            c"a raw option".as_ptr(),
            Some(parse_fails),
            std::ptr::null_mut(),
            false,
            std::ptr::null(),
        )
    }
}

unsafe extern "C" fn register_adds_good_option(
    options: *mut ffi::clingo_options_t,
    _data: *mut c_void,
) -> bool {
    // SAFETY: as for `register_adds_option`.
    unsafe {
        ffi::clingo_options_add(
            options,
            c"Raw".as_ptr(),
            c"zzraw".as_ptr(),
            c"a raw option".as_ptr(),
            Some(parse_ok),
            std::ptr::null_mut(),
            false,
            std::ptr::null(),
        )
    }
}

unsafe extern "C" fn register_fails(
    _options: *mut ffi::clingo_options_t,
    _data: *mut c_void,
) -> bool {
    fail(c"raw register message")
}

unsafe extern "C" fn validate_fails(_data: *mut c_void) -> bool {
    fail(c"raw validate message")
}

unsafe extern "C" fn main_ok(
    _control: *mut ffi::clingo_control_t,
    _files: *const *const c_char,
    _size: usize,
    _data: *mut c_void,
) -> bool {
    true
}

/// Calls `clingo_main` with the given callbacks and arguments and prints the
/// exit code it returned.
fn call(
    register: Option<unsafe extern "C" fn(*mut ffi::clingo_options_t, *mut c_void) -> bool>,
    validate: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
    arguments: &[&str],
) {
    let mut application = ffi::clingo_application_t {
        program_name: None,
        version: None,
        message_limit: None,
        main: Some(main_ok),
        logger: None,
        printer: None,
        register_options: register,
        validate_options: validate,
    };
    let owned: Vec<CString> = arguments
        .iter()
        .map(|a| CString::new(*a).unwrap())
        .collect();
    let pointers: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
    // SAFETY: `application` holds valid callbacks that ignore their data
    // pointer; `pointers` holds `pointers.len()` NUL-terminated strings that
    // outlive the call.
    let code = unsafe {
        ffi::clingo_main(
            &raw mut application,
            pointers.as_ptr(),
            pointers.len(),
            std::ptr::null_mut(),
        )
    };
    println!("CHILD-RC {code}");
}

/// The child side: runs the case named by the environment, or does nothing.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    match case.as_str() {
        "baseline" => call(Some(register_adds_good_option), None, &["--zzraw=1"]),
        "validate_fails" => call(None, Some(validate_fails), &[]),
        "register_fails" => call(Some(register_fails), None, &[]),
        "parse_fails" => call(Some(register_adds_option), None, &["--zzraw=v"]),
        other => panic!("unknown child case {other}"),
    }
}

fn run(case: &str) -> Option<child::Outcome> {
    child::run_child(ENTRY, case)
}

#[test]
fn baseline_a_run_with_a_good_option_returns_zero() {
    let Some(out) = run("baseline") else {
        return;
    };
    assert!(out.stdout.contains("CHILD-RC 0"), "{}", out.stdout);
}

#[test]
fn a_failing_validate_options_returns_exit_code_zero_in_clingo() {
    let Some(out) = run("validate_fails") else {
        return;
    };
    // U39. If this becomes 1, clingo was fixed: update the documentation of
    // `Application::validate_options` and change this expectation.
    assert!(out.stdout.contains("CHILD-RC 0"), "{}", out.stdout);
    assert!(
        out.stderr
            .contains("*** ERROR: (clingo): raw validate message"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_failing_register_options_returns_exit_code_one() {
    let Some(out) = run("register_fails") else {
        return;
    };
    assert!(out.stdout.contains("CHILD-RC 1"), "{}", out.stdout);
    assert!(
        out.stderr
            .contains("*** ERROR: (clingo): raw register message"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_failing_parse_returns_exit_code_one_and_clingo_drops_the_message() {
    let Some(out) = run("parse_fails") else {
        return;
    };
    assert!(out.stdout.contains("CHILD-RC 1"), "{}", out.stdout);
    assert!(
        out.stderr.contains("'v' invalid value for: 'zzraw'"),
        "{}",
        out.stderr
    );
    // If clingo starts to keep the message, the wrapper could stop replacing
    // it.
    assert!(!out.stderr.contains("raw parse message"), "{}", out.stderr);
}

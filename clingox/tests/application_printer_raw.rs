//! The crash that makes `Application::print_model` require `main`,
//! read at the C level through `clingox_sys`, independent of `Application`.
//!
//! U40 (pyclingo 5.8.2, clingo 5.8.2, clasp 3.4.1): with a `printer` and no
//! `main`, `clingo_model_symbols` inside the printer dies of SIGSEGV
//! (`ClingoModel::lp()` dereferences a logic program the default main does not
//! keep), while `clingo_model_number` and `clingo_model_cost_size` work. With a
//! `main` the same call works.
//!
//! When clingo stops crashing, the crash case below fails and this file tells
//! the maintainer to lift the refusal in `Application::run` and to rewrite
//! `api_application_printer.rs::p1_*`. Every case runs in a child process
//! (`common/child.rs`).

#![cfg(unix)]
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

unsafe extern "C" fn main_solves(
    control: *mut ffi::clingo_control_t,
    files: *const *const c_char,
    size: usize,
    _data: *mut c_void,
) -> bool {
    // SAFETY: clingo passes a live control and `size` file names for the call.
    unsafe {
        for i in 0..size {
            if !ffi::clingo_control_load(control, *files.add(i)) {
                return false;
            }
        }
        let part = ffi::clingo_part_t {
            name: c"base".as_ptr(),
            params: std::ptr::null(),
            size: 0,
        };
        if !ffi::clingo_control_ground(control, &raw const part, 1, None, std::ptr::null_mut()) {
            return false;
        }
        let mut handle = std::ptr::null_mut();
        if !ffi::clingo_control_solve(
            control,
            0,
            std::ptr::null(),
            0,
            None,
            std::ptr::null_mut(),
            &raw mut handle,
        ) {
            return false;
        }
        ffi::clingo_solve_handle_close(handle)
    }
}

/// Reads only what works without `main`, and says so.
unsafe extern "C" fn printer_safe(
    model: *const ffi::clingo_model_t,
    _default: ffi::clingo_default_model_printer_t,
    _default_data: *mut c_void,
    _data: *mut c_void,
) -> bool {
    let mut number = 0_u64;
    let mut costs = 0_usize;
    // SAFETY: `model` is the live model clingo passes to the printer.
    let ok = unsafe {
        ffi::clingo_model_number(model, &raw mut number)
            && ffi::clingo_model_cost_size(model, &raw mut costs)
    };
    println!("RAW-SAFE {ok} {number} {costs}");
    ok
}

/// Reads the shown symbols, which crashes without `main`.
unsafe extern "C" fn printer_symbols(
    model: *const ffi::clingo_model_t,
    _default: ffi::clingo_default_model_printer_t,
    _default_data: *mut c_void,
    _data: *mut c_void,
) -> bool {
    let mut size = 0_usize;
    // SAFETY: `model` is the live model clingo passes to the printer.
    let ok = unsafe {
        ffi::clingo_model_symbols_size(model, ffi::clingo_show_type_shown, &raw mut size)
    };
    println!("RAW-SYMBOLS {ok} {size}");
    ok
}

fn call(printer: ffi::clingo_model_printer_t, with_main: bool) {
    let file = child::fixture("printer_raw.lp", "{x(1..2)}. #show x/1.");
    let arguments = [child::arg(&file), "0".to_owned()];
    let mut application = ffi::clingo_application_t {
        program_name: None,
        version: None,
        message_limit: None,
        main: with_main.then_some(main_solves as _),
        logger: None,
        printer,
        register_options: None,
        validate_options: None,
    };
    let owned: Vec<CString> = arguments
        .iter()
        .map(|a| CString::new(a.as_str()).unwrap())
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
    // Ends libtest's unfinished "test child_entry ... " line.
    println!();
    match case.as_str() {
        "safe_reads_without_main" => call(Some(printer_safe), false),
        "symbols_without_main" => call(Some(printer_symbols), false),
        "symbols_with_main" => call(Some(printer_symbols), true),
        other => panic!("unknown child case {other}"),
    }
}

fn run(case: &str) -> Option<child::Outcome> {
    child::run_child(ENTRY, case)
}

#[test]
fn r1_number_and_cost_work_in_the_printer_without_main() {
    let Some(out) = run("safe_reads_without_main") else {
        return;
    };
    assert!(out.stdout.contains("RAW-SAFE true 1 0"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-RC 30"), "{}", out.stdout);
}

#[test]
fn r1_symbols_in_the_printer_without_main_still_crash() {
    let Some(out) = run("symbols_without_main") else {
        return;
    };
    // U40. If this fails, clingo was fixed: lift the refusal of `print_model`
    // without `main` in `Application::run`, rewrite `p1_*` and `p2_*` of
    // `api_application_printer.rs`, and update U40 and the rustdoc.
    assert_eq!(
        out.signal,
        Some(11),
        "expected SIGSEGV; stdout: {} stderr: {}",
        out.stdout,
        out.stderr
    );
    assert!(!out.stdout.contains("CHILD-RC"), "{}", out.stdout);
}

#[test]
fn r1_symbols_in_the_printer_work_with_main() {
    let Some(out) = run("symbols_with_main") else {
        return;
    };
    assert!(out.stdout.contains("RAW-SYMBOLS true 0"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-RC 30"), "{}", out.stdout);
}

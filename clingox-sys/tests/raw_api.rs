//! Smoke tests of the raw C API through the generated bindings.
//!
//! They add, ground and solve programs and check error reporting directly against
//! `clingo.h`, with no safe wrapper in between. They run unchanged on every target
//! (Linux, Android, WebAssembly under Node.js and in browsers), so a platform build
//! problem shows up here before it can reach the safe API.

use std::ffi::{CStr, CString, c_char};
use std::ptr;

use clingox_sys::*;

/// Creates a control with the given command-line arguments and no logger.
fn new_control(args: &[&str]) -> *mut clingo_control_t {
    let owned: Vec<CString> = args
        .iter()
        .map(|a| CString::new(*a).expect("arguments contain no NUL"))
        .collect();
    let pointers: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
    let mut control = ptr::null_mut();
    // SAFETY: `pointers` holds `pointers.len()` valid C strings that outlive the call.
    // clingo parses them into its own storage during construction
    // (libclingo/src/clingocontrol.cc:989), so they may be freed afterwards.
    // A null logger is allowed (clingo.h:2984).
    let ok = unsafe {
        clingo_control_new(
            pointers.as_ptr(),
            pointers.len(),
            None,
            ptr::null_mut(),
            20,
            &raw mut control,
        )
    };
    assert!(ok, "clingo_control_new failed: {}", last_error_message());
    assert!(!control.is_null());
    control
}

/// Copies clingo's thread-local error message, which the next failure overwrites.
fn last_error_message() -> String {
    // SAFETY: clingo_error_message returns null or a valid C string that stays valid
    // until the next error or message query on this thread; we copy it at once.
    unsafe {
        let msg = clingo_error_message();
        if msg.is_null() {
            String::new()
        } else {
            CStr::from_ptr(msg).to_string_lossy().into_owned()
        }
    }
}

fn symbol_text(symbol: clingo_symbol_t) -> String {
    let mut size = 0;
    // SAFETY: `size` is a valid out-pointer.
    assert!(unsafe { clingo_symbol_to_string_size(symbol, &raw mut size) });
    let mut buf = vec![0 as c_char; size];
    // SAFETY: `buf` has exactly the size clingo asked for, including the NUL.
    assert!(unsafe { clingo_symbol_to_string(symbol, buf.as_mut_ptr(), size) });
    // SAFETY: clingo wrote a NUL-terminated string into `buf`.
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn add_and_ground(control: *mut clingo_control_t, program: &str) {
    let base = CString::new("base").expect("the part name contains no NUL");
    let program = CString::new(program).expect("the program contains no NUL");
    // SAFETY: `control` is live; the strings are valid and outlive the call; no
    // parameters are passed, so a null parameter array with size 0 is allowed.
    let ok =
        unsafe { clingo_control_add(control, base.as_ptr(), ptr::null(), 0, program.as_ptr()) };
    assert!(ok, "clingo_control_add failed: {}", last_error_message());
    let part = clingo_part_t {
        name: base.as_ptr(),
        params: ptr::null(),
        size: 0,
    };
    // SAFETY: one valid part; no ground callback.
    let ok = unsafe { clingo_control_ground(control, &raw const part, 1, None, ptr::null_mut()) };
    assert!(ok, "clingo_control_ground failed: {}", last_error_message());
}

/// Solves in yield mode and returns every model as a sorted list of shown atoms.
fn all_models(control: *mut clingo_control_t) -> Vec<Vec<String>> {
    let mut handle = ptr::null_mut();
    // SAFETY: `control` is live and not solving; no assumptions, no event callback.
    let ok = unsafe {
        clingo_control_solve(
            control,
            clingo_solve_mode_yield as clingo_solve_mode_bitset_t,
            ptr::null(),
            0,
            None,
            ptr::null_mut(),
            &raw mut handle,
        )
    };
    assert!(ok, "clingo_control_solve failed: {}", last_error_message());

    let mut models = Vec::new();
    loop {
        // SAFETY: `handle` is live until the close below.
        assert!(unsafe { clingo_solve_handle_resume(handle) });
        let mut model = ptr::null();
        // SAFETY: as above; `model` is a valid out-pointer.
        assert!(unsafe { clingo_solve_handle_model(handle, &raw mut model) });
        if model.is_null() {
            break;
        }
        let show = clingo_show_type_shown as clingo_show_type_bitset_t;
        let mut size = 0;
        // SAFETY: `model` is valid until the next resume.
        assert!(unsafe { clingo_model_symbols_size(model, show, &raw mut size) });
        let mut symbols = vec![0 as clingo_symbol_t; size];
        // SAFETY: `symbols` has the size clingo reported.
        assert!(unsafe { clingo_model_symbols(model, show, symbols.as_mut_ptr(), size) });
        let mut atoms: Vec<String> = symbols.into_iter().map(symbol_text).collect();
        atoms.sort();
        models.push(atoms);
    }
    // SAFETY: `handle` is live and is not used after this call.
    assert!(unsafe { clingo_solve_handle_close(handle) });
    models.sort();
    models
}

#[test]
fn enumerates_both_models_of_a_choice_program() {
    let control = new_control(&["0"]);
    add_and_ground(control, "a :- not b. b :- not a.");
    assert_eq!(
        all_models(control),
        vec![vec!["a".to_owned()], vec!["b".to_owned()]]
    );
    // SAFETY: `control` is live and not used afterwards.
    unsafe { clingo_control_free(control) };
}

#[test]
fn syntax_error_is_reported_as_runtime_error() {
    let control = new_control(&[]);
    let base = CString::new("base").unwrap();
    // "a :- ." would be valid (an empty body makes `a` a fact); two body atoms
    // without a separator are a genuine syntax error.
    let program = CString::new("a :- b c.").unwrap();
    // SAFETY: as in `add_and_ground`.
    let ok =
        unsafe { clingo_control_add(control, base.as_ptr(), ptr::null(), 0, program.as_ptr()) };
    assert!(!ok, "a syntax error must make clingo_control_add fail");
    // SAFETY: reads this thread's error code.
    let code = unsafe { clingo_error_code() };
    assert_eq!(
        code,
        clingo_error_t::try_from(clingo_error_runtime).expect("error codes fit in an int")
    );
    assert!(!last_error_message().is_empty());
    // SAFETY: `control` is live and not used afterwards.
    unsafe { clingo_control_free(control) };
}

#[test]
fn a_second_control_works_after_the_first_is_freed() {
    let first = new_control(&["0"]);
    add_and_ground(first, "p.");
    assert_eq!(all_models(first), vec![vec!["p".to_owned()]]);
    // SAFETY: `first` is live and not used afterwards.
    unsafe { clingo_control_free(first) };

    let second = new_control(&["0"]);
    add_and_ground(second, "q(1..2).");
    assert_eq!(
        all_models(second),
        vec![vec!["q(1)".to_owned(), "q(2)".to_owned()]]
    );
    // SAFETY: `second` is live and not used afterwards.
    unsafe { clingo_control_free(second) };
}

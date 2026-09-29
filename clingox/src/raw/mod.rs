//! The only module that may contain `unsafe` (RULES 2).
//!
//! It holds the primitives every call into clingo is built from, and thin
//! wrappers, one per C function, that the safe layer uses. Nothing outside this
//! module names `clingox_sys`.
//!
//! The primitives are:
//! - [`call`]: reset clingo's error state, make the call, and turn a `false`
//!   into an [`Error`] (DESIGN S1);
//! - [`fill_vec`] and [`fill_string`]: the query-size-then-fill pattern (S17);
//! - [`c_str`] and [`borrowed_str`]: C strings in and out, with the NUL and
//!   UTF-8 rules;
//! - [`raw_slice`]: a pointer and length from C, null-safe;
//! - [`application`] and [`process`]: `clingo_main` and the process-wide guards
//!   around it (S18);
//! - [`trampoline`]: the `catch_unwind` guard every callback runs in, and the
//!   logger trampoline built on it (S8, S9).

mod application;
mod ast;
// Written by `cargo xtask ast-codegen` through prettyplease, which `cargo xtask
// check` compares byte for byte; rustfmt must not reflow it.
#[rustfmt::skip]
mod ast_generated;
mod atoms;
mod backend;
mod capture;
mod config;
mod control;
mod events;
mod exit_options;
mod interrupt;
mod model;
mod observer;
mod process;
mod program_builder;
mod propagate;
mod script;
mod solve;
mod stats;
mod symbol;
mod theory;
mod trampoline;

use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::sync::OnceLock;

use clingox_sys as ffi;

use crate::error::{Error, ErrorKind};

pub(crate) use application::{
    AppLogger, AppMain, AppPrinter, AppRegister, AppValidate, OptionsHandle, PrinterHandle,
    Settings, run as run_application,
};
pub(crate) use ast::{
    BuildArg, RawAst, UNPOOL_CONDITION, UNPOOL_OTHER, ast_acquire, ast_attribute_delete_ast_at,
    ast_attribute_delete_string_at, ast_attribute_get_ast, ast_attribute_get_ast_at,
    ast_attribute_get_location, ast_attribute_get_number, ast_attribute_get_optional_ast,
    ast_attribute_get_string, ast_attribute_get_string_at, ast_attribute_get_symbol,
    ast_attribute_insert_ast_at, ast_attribute_insert_string_at, ast_attribute_set_ast,
    ast_attribute_set_ast_at, ast_attribute_set_location, ast_attribute_set_number,
    ast_attribute_set_optional_ast, ast_attribute_set_string, ast_attribute_set_string_at,
    ast_attribute_set_symbol, ast_attribute_size_ast_array, ast_attribute_size_string_array,
    ast_attribute_type, ast_build, ast_copy, ast_deep_copy, ast_equal, ast_has_attribute, ast_hash,
    ast_less_than, ast_release, ast_to_string, ast_type, intern, parse_files, parse_string, unpool,
};
pub(crate) use atoms::{AtomData, AtomIterator, Atoms};
pub(crate) use backend::{WeightedLiteral, weighted_literal};
pub(crate) use capture::{MESSAGE_LIMIT, UserLogger};
pub(crate) use control::{ControlHandle, GroundError, SolveOutcome};
pub(crate) use interrupt::SolveSync;
pub(crate) use model::{
    ConsequenceValue, ModelData, ModelType, RawSolveControl, model_contains, model_context,
    model_cost, model_extend, model_is_consequence, model_is_true, model_number,
    model_optimality_proven, model_priority, model_symbols, model_thread_id, model_type, show,
};
pub(crate) use propagate::{Init, RawAssignment, RawPropagateControl};
pub(crate) use script::{register as register_script, version as script_version};
#[cfg(test)]
pub(crate) use solve::panic_while_closing;
pub(crate) use solve::settle;
pub(crate) use stats::{MutableStats, Stats};
pub(crate) use symbol::{
    RawSignature, SymbolType, create_function, create_infimum, create_number, create_signature,
    create_string, create_supremum, parse_term, signature_arity, signature_hash,
    signature_is_equal_to, signature_is_less_than, signature_is_positive, signature_name,
    symbol_arguments, symbol_hash, symbol_is_equal_to, symbol_is_less_than, symbol_is_positive,
    symbol_name, symbol_number, symbol_string, symbol_to_string, symbol_type,
};
pub(crate) use theory::{TermType, Theory};

/// Whether clasp is built with threads, which asynchronous solving, timeouts
/// and more than one solver thread need.
///
/// It is what `clingox-sys` reports about the clingo it built or found, not a
/// guess from the target: a vendored build without the feature `threads`, or
/// a system clingo built without threads, has none on any target.
pub(crate) const HAS_THREADS: bool = ffi::HAS_THREADS;

/// The raw symbol value, which `Symbol` wraps transparently.
pub(crate) type RawSymbol = ffi::clingo_symbol_t;

/// Where [`call`] reads and resets the error state.
///
/// The real state is clingo's thread-local one. The indirection exists so the
/// ordering rules of S1 (reset first, copy the message at once) can be tested
/// under Miri, which cannot call into C.
pub(crate) trait ErrorState {
    /// Sets the state, as a callback must before it returns `false` (S8).
    fn set(code: c_uint, message: &CStr);
    /// Sets the state to success, so a later `false` cannot report a stale
    /// error.
    fn reset() {
        Self::set(ffi::clingo_error_success, c"");
    }
    /// The current error code.
    fn code() -> c_int;
    /// The current message, copied. The buffer behind it is only valid until
    /// the next failure or message query on this thread.
    fn message() -> Option<String>;
}

/// clingo's own thread-local error state.
pub(crate) enum ClingoErrorState {}

impl ErrorState for ClingoErrorState {
    fn set(code: c_uint, message: &CStr) {
        // SAFETY: clingo_set_error takes an error code and a NUL-terminated
        // string, which it copies into a new exception object
        // (control.cc:212-219), so the string need not outlive the call.
        unsafe { ffi::clingo_set_error(c_int_of(code), message.as_ptr()) };
    }

    fn code() -> c_int {
        // SAFETY: clingo_error_code only reads a thread-local integer
        // (control.cc:234).
        unsafe { ffi::clingo_error_code() }
    }

    fn message() -> Option<String> {
        // SAFETY: clingo_error_message returns null or a NUL-terminated string
        // that stays valid until the next failure or message query on this
        // thread (clingo.h:158, control.cc:220-232). `borrowed_str_lossy`
        // copies it before any other clingo call can run (S1).
        unsafe { borrowed_str_lossy(ffi::clingo_error_message()) }
    }
}

/// Calls a clingo function that returns a success flag (DESIGN S1).
///
/// The error state is reset first, because clingo never resets it on success.
/// On `false`, the code is read and the message copied at once. The first call
/// on any thread also runs the runtime version check (S18).
pub(crate) fn call(f: impl FnOnce() -> bool) -> Result<(), Error> {
    check_version()?;
    call_with::<ClingoErrorState>(f)
}

/// [`call`] for a pure query: a clingo function that only reads, so calling it
/// twice with the same arguments gives the same answer and changes nothing.
///
/// [`call`] resets clingo's error state before every call, and
/// `clingo_set_error` allocates an exception object to do it, which costs more
/// than most of the queries it guards. A query that succeeds reads no error
/// state at all, so here the reset is skipped. A query that fails runs again
/// through [`call`], which resets first, so a failure is reported from a
/// fresh error state exactly as before (S1): the first, unreset call may have
/// left a stale code or message, and none of it is ever read.
///
/// Use it only for calls that are idempotent. A call that adds, grounds,
/// solves, or runs a callback of the user's goes through [`call`].
#[inline]
pub(crate) fn query(f: impl FnMut() -> bool) -> Result<(), Error> {
    check_version()?;
    query_with::<ClingoErrorState>(f)
}

/// [`query`] against a given error state, without the version check.
#[inline]
fn query_with<S: ErrorState>(mut f: impl FnMut() -> bool) -> Result<(), Error> {
    if f() {
        return Ok(());
    }
    call_with::<S>(f)
}

/// [`call`] against a given error state, without the version check.
pub(crate) fn call_with<S: ErrorState>(f: impl FnOnce() -> bool) -> Result<(), Error> {
    S::reset();
    if f() {
        return Ok(());
    }
    let code = S::code();
    // Copied here, before anything else can reach clingo on this thread.
    let message = S::message().filter(|m| !m.is_empty());
    let kind = error_kind(code);
    let message = match (code_is_success(code), message) {
        // S1: a `false` with a success code has no message of its own.
        (true, _) | (false, None) => "no message".to_owned(),
        (false, Some(message)) => message,
    };
    Err(Error::new(kind, message))
}

fn code_is_success(code: c_int) -> bool {
    c_uint::try_from(code) == Ok(ffi::clingo_error_success)
}

/// Maps clingo's error codes (clingo.h:140-146) to kinds. A success code on a
/// failed call and any code clingo may add later are `Unknown`.
fn error_kind(code: c_int) -> ErrorKind {
    match c_uint::try_from(code) {
        Ok(ffi::clingo_error_runtime) => ErrorKind::Runtime,
        Ok(ffi::clingo_error_logic) => ErrorKind::Logic,
        Ok(ffi::clingo_error_bad_alloc) => ErrorKind::BadAlloc,
        _ => ErrorKind::Unknown,
    }
}

/// Converts one of the header's enum constants, which bindgen types as
/// `c_uint`, to the `c_int` the functions take.
pub(super) fn c_int_of(value: c_uint) -> c_int {
    // The header's constants are small non-negative numbers (clingo.h:140-173).
    c_int::try_from(value).unwrap_or(c_int::MAX)
}

/// The lowest patch level accepted at run time: concurrent symbol creation
/// became safe in clingo 5.8.1, which the build script also requires of a
/// system library, and 5.8.1 and 5.8.2 have the same C API.
const MIN_PATCH: c_int = 1;

/// Whether a linked clingo `linked` can serve bindings generated for
/// `expected`: the same major and minor version, and a patch level of at
/// least [`MIN_PATCH`]. It is the range the build script accepts.
fn version_accepted(linked: (c_int, c_int, c_int), expected: (c_int, c_int, c_int)) -> bool {
    linked.0 == expected.0 && linked.1 == expected.1 && linked.2 >= MIN_PATCH
}

/// The runtime version check (DESIGN S18): the linked library must be in the
/// range the build accepts, the major and minor version the bindings were
/// generated from with a patch level of at least 1 (for clingo 5.8: 5.8.1 or
/// newer within 5.8).
#[inline]
pub(crate) fn check_version() -> Result<(), Error> {
    #[cfg(not(test))]
    if VERSION_ACCEPTED.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }
    check_version_slow()
}

/// The first, and every failing, run of [`check_version`].
#[cold]
fn check_version_slow() -> Result<(), Error> {
    // The linked library cannot change while the process runs, so once the
    // check has passed it need not be made again. A failing check is made
    // every time, and so reports every time. Tests fake the version, so they
    // always make the check.
    let linked = linked_version();
    let (major, minor, revision) = ffi::CLINGO_VERSION;
    let expected = (c_int_of(major), c_int_of(minor), c_int_of(revision));
    if version_accepted(linked, expected) {
        #[cfg(not(test))]
        VERSION_ACCEPTED.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::Version,
            format!(
                "clingo {}.{}.{} is linked, but clingox needs {major}.{minor}.{MIN_PATCH} or \
                 newer within {major}.{minor} (it is built for {major}.{minor}.{revision})",
                linked.0, linked.1, linked.2
            ),
        ))
    }
}

/// Whether [`check_version`] has passed once.
#[cfg(not(test))]
static VERSION_ACCEPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The version of the linked clingo as unsigned numbers, for the public
/// `version`.
pub(crate) fn library_version() -> (u32, u32, u32) {
    let (major, minor, revision) = linked_version();
    let unsigned = |part: c_int| u32::try_from(part).unwrap_or_default();
    (unsigned(major), unsigned(minor), unsigned(revision))
}

/// The version of the linked clingo, read once.
fn linked_version() -> (c_int, c_int, c_int) {
    static LINKED: OnceLock<(c_int, c_int, c_int)> = OnceLock::new();
    #[cfg(test)]
    if let Some(linked) = tests::LINKED_VERSION.get() {
        return linked;
    }
    *LINKED.get_or_init(|| {
        let (mut major, mut minor, mut revision) = (0, 0, 0);
        // SAFETY: clingo_version writes three integers through valid
        // out-pointers and cannot fail (clingo.h:194).
        unsafe { ffi::clingo_version(&raw mut major, &raw mut minor, &raw mut revision) };
        (major, minor, revision)
    })
}

/// Converts a Rust string for clingo, rejecting interior NUL bytes.
pub(crate) fn c_str(s: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|e| nul_error(s, e.nul_position()))
}

fn nul_error(s: &str, position: usize) -> Error {
    Error::new(
        ErrorKind::Nul,
        format!("{s:?} contains a NUL byte at position {position}"),
    )
}

/// Runs `f` on a C string made from `s`, with the NUL check of [`c_str`].
///
/// A short string is copied to the stack, so the many calls that pass a name
/// or a path to clingo and keep nothing of it do not allocate. A long one is
/// converted with [`c_str`].
pub(crate) fn with_c_str<T>(
    s: &str,
    f: impl FnOnce(&CStr) -> Result<T, Error>,
) -> Result<T, Error> {
    const STACK: usize = 64;
    if s.len() >= STACK {
        return f(&c_str(s)?);
    }
    if let Some(position) = s.bytes().position(|b| b == 0) {
        return Err(nul_error(s, position));
    }
    let mut buffer = [0u8; STACK];
    buffer[..s.len()].copy_from_slice(s.as_bytes());
    // The buffer ends at the NUL written above, and `s` has none inside.
    let text = CStr::from_bytes_with_nul(&buffer[..=s.len()]).map_err(|_| nul_error(s, s.len()))?;
    f(text)
}

/// Borrows a C string from clingo as UTF-8.
///
/// # Safety
///
/// `ptr` must be null or point to a NUL-terminated string that stays valid and
/// unchanged for `'a`.
pub(crate) unsafe fn borrowed_str<'a>(ptr: *const c_char) -> Result<&'a str, Error> {
    if ptr.is_null() {
        return Err(Error::new(
            ErrorKind::Unknown,
            "clingo returned a null string",
        ));
    }
    // SAFETY: `ptr` is non-null, and the caller guarantees it is NUL-terminated
    // and valid for 'a.
    let text = unsafe { CStr::from_ptr(ptr) };
    text.to_str().map_err(|e| {
        Error::new(
            ErrorKind::Utf8,
            format!("clingo returned a string that is {e}"),
        )
    })
}

/// Copies a C string from clingo, replacing invalid UTF-8.
///
/// # Safety
///
/// `ptr` must be null or point to a NUL-terminated string that is valid for the
/// duration of the call.
pub(crate) unsafe fn borrowed_str_lossy(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` is non-null, and the caller guarantees it is NUL-terminated
    // and valid while it is copied here.
    let text = unsafe { CStr::from_ptr(ptr) };
    Some(text.to_string_lossy().into_owned())
}

/// Views a pointer and length from clingo as a slice.
///
/// A null pointer or a zero length gives an empty slice, because
/// `slice::from_raw_parts` requires a non-null, aligned pointer even for length
/// zero, and clingo returns null for empty arrays.
///
/// # Safety
///
/// Unless `ptr` is null or `len` is zero, `ptr` must point to `len` initialized
/// values of `T` that stay valid and unchanged for `'a`.
pub(crate) unsafe fn raw_slice<'a, T>(ptr: *const T, len: usize) -> &'a [T] {
    if ptr.is_null() || len == 0 {
        return &[];
    }
    // SAFETY: `ptr` is non-null and `len` is non-zero, and the caller
    // guarantees it points to `len` values valid for 'a. clingo's arrays come
    // from C++ vectors of the element type, so they are aligned.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// Runs a size query, then fills a buffer of that size (S17).
///
/// `size` writes the number of elements; `fill` receives a buffer of exactly
/// that many default values and its length.
pub(crate) fn fill_vec<T: Copy + Default>(
    size: impl FnOnce(&mut usize) -> Result<(), Error>,
    fill: impl FnOnce(*mut T, usize) -> Result<(), Error>,
) -> Result<Vec<T>, Error> {
    let mut len = 0;
    size(&mut len)?;
    let mut buffer = vec![T::default(); len];
    fill(buffer.as_mut_ptr(), len)?;
    Ok(buffer)
}

/// Runs a size query for a string, including its NUL, then fills it (S17).
///
/// The buffer is filled as bytes and becomes the returned string in place, so
/// the text is allocated once; the UTF-8 check and the check for a
/// terminating NUL stay.
pub(crate) fn fill_string(
    size: impl FnOnce(&mut usize) -> Result<(), Error>,
    fill: impl FnOnce(*mut c_char, usize) -> Result<(), Error>,
) -> Result<String, Error> {
    let mut len = 0;
    size(&mut len)?;
    let mut bytes = vec![0u8; len];
    // `c_char` is `i8` or `u8` depending on the target; the cast reinterprets
    // each byte without changing it.
    fill(bytes.as_mut_ptr().cast::<c_char>(), len)?;
    let Some(nul) = bytes.iter().position(|&b| b == 0) else {
        return Err(Error::new(
            ErrorKind::Unknown,
            "clingo filled a string buffer without a terminating NUL",
        ));
    };
    bytes.truncate(nul);
    String::from_utf8(bytes).map_err(|e| {
        Error::new(
            ErrorKind::Utf8,
            format!("clingo returned a string that is {}", e.utf8_error()),
        )
    })
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::ffi::CString;

    use super::*;

    thread_local! {
        /// A linked version for `check_version` to see instead of clingo's, so
        /// that its decision can be tested on versions no build links.
        pub(super) static LINKED_VERSION: Cell<Option<(c_int, c_int, c_int)>> =
            const { Cell::new(None) };

        /// The fake error state: a code and an owned message buffer. Setting a
        /// new message frees the old buffer, as clingo's does, so a message
        /// read lazily from it would be a use-after-free that Miri reports.
        static FAKE: RefCell<(c_int, Option<CString>)> = const { RefCell::new((0, None)) };
    }

    enum Fake {}

    impl Fake {
        fn put(code: c_uint, message: &str) {
            FAKE.with(|f| {
                *f.borrow_mut() = (c_int_of(code), Some(CString::new(message).unwrap()));
            });
        }
    }

    impl ErrorState for Fake {
        fn set(code: c_uint, message: &CStr) {
            Fake::put(code, message.to_str().unwrap());
        }
        fn code() -> c_int {
            FAKE.with(|f| f.borrow().0)
        }
        fn message() -> Option<String> {
            let ptr = FAKE.with(|f| {
                f.borrow()
                    .1
                    .as_ref()
                    .map_or(std::ptr::null(), |m| m.as_ptr())
            });
            // SAFETY: the pointer is null or points into the CString held by
            // FAKE, which nothing replaces while it is copied here.
            unsafe { borrowed_str_lossy(ptr) }
        }
    }

    #[test]
    fn a_successful_call_is_ok() {
        assert!(call_with::<Fake>(|| true).is_ok());
    }

    #[test]
    fn a_failed_call_reports_its_code_and_message() {
        let err = call_with::<Fake>(|| {
            Fake::put(ffi::clingo_error_runtime, "it broke");
            false
        })
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime);
        assert_eq!(err.to_string(), "it broke");
    }

    #[test]
    fn a_stale_error_is_not_reported_for_a_bare_false() {
        Fake::put(ffi::clingo_error_logic, "stale");
        let err = call_with::<Fake>(|| false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unknown);
        assert_eq!(err.to_string(), "no message");
    }

    /// A `false` with a success code reports no message, even
    /// when the buffer still holds one.
    #[test]
    fn a_success_code_hides_a_stale_message() {
        Fake::put(ffi::clingo_error_success, "stale");
        let err = call_with::<Fake>(|| {
            Fake::put(ffi::clingo_error_success, "stale");
            false
        })
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unknown);
        assert_eq!(err.to_string(), "no message");
    }

    /// A linked clingo outside the accepted range is refused
    /// with `ErrorKind::Version`, by `check_version` and by every `call`,
    /// before the call is made.
    #[test]
    fn a_linked_version_outside_the_range_is_a_version_error() {
        LINKED_VERSION.set(Some((5, 7, 2)));
        let checked = check_version();
        let mut called = false;
        let through_call = call(|| {
            called = true;
            true
        });
        LINKED_VERSION.set(None);
        let err = checked.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Version);
        assert!(err.to_string().contains("5.7.2"), "{err}");
        assert_eq!(through_call.unwrap_err().kind(), ErrorKind::Version);
        assert!(!called, "the call is not made");
    }

    /// A query that succeeds does not touch the error state: that is the
    /// saving over `call`, which resets it before every call.
    #[test]
    fn a_successful_query_leaves_the_error_state_alone() {
        Fake::put(ffi::clingo_error_runtime, "left as it was");
        assert!(query_with::<Fake>(|| true).is_ok());
        assert_eq!(Fake::code(), c_int_of(ffi::clingo_error_runtime));
    }

    /// A query that fails runs again through `call`, which resets first, so a
    /// stale error from before the query is never reported for it (S1).
    #[test]
    fn a_failed_query_reports_a_fresh_error_never_a_stale_one() {
        Fake::put(ffi::clingo_error_logic, "stale");
        let mut runs = 0;
        let err = query_with::<Fake>(|| {
            runs += 1;
            false
        })
        .unwrap_err();
        assert_eq!(runs, 2, "the failing query runs once more, after the reset");
        assert_eq!(err.kind(), ErrorKind::Unknown);
        assert_eq!(err.to_string(), "no message");
    }

    #[test]
    fn a_failed_query_reports_the_error_of_its_second_run() {
        let mut runs = 0;
        let err = query_with::<Fake>(|| {
            runs += 1;
            Fake::put(ffi::clingo_error_runtime, &format!("failure {runs}"));
            false
        })
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime);
        assert_eq!(err.to_string(), "failure 2");
    }

    #[test]
    fn a_query_that_fails_only_once_succeeds() {
        let mut runs = 0;
        assert!(
            query_with::<Fake>(|| {
                runs += 1;
                runs > 1
            })
            .is_ok()
        );
    }

    #[test]
    fn with_c_str_gives_the_string_and_rejects_a_nul_byte_at_every_length() {
        for len in [0, 1, 63, 64, 65, 200] {
            let text = "a".repeat(len);
            let seen = with_c_str(&text, |c| Ok(c.to_str().unwrap().to_owned())).unwrap();
            assert_eq!(seen, text);
            for position in [0, len / 2, len.saturating_sub(1)] {
                if len == 0 {
                    continue;
                }
                let mut bytes = text.clone().into_bytes();
                bytes[position] = 0;
                let with_nul = String::from_utf8(bytes).unwrap();
                let err = with_c_str(&with_nul, |_| Ok(())).unwrap_err();
                assert_eq!(err.kind(), ErrorKind::Nul);
                assert_eq!(err.to_string(), c_str(&with_nul).unwrap_err().to_string());
            }
        }
    }

    #[test]
    fn with_c_str_passes_the_error_of_the_closure_on() {
        let err =
            with_c_str("x", |_| Err::<(), _>(Error::new(ErrorKind::Logic, "no"))).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Logic);
    }

    #[test]
    fn a_stale_error_does_not_fail_a_successful_call() {
        Fake::put(ffi::clingo_error_runtime, "stale");
        assert!(call_with::<Fake>(|| true).is_ok());
        assert_eq!(Fake::code(), c_int_of(ffi::clingo_error_success));
    }

    #[test]
    fn the_message_is_copied_before_the_buffer_changes() {
        let first = call_with::<Fake>(|| {
            Fake::put(ffi::clingo_error_runtime, "first");
            false
        })
        .unwrap_err();
        let second = call_with::<Fake>(|| {
            Fake::put(ffi::clingo_error_bad_alloc, "second");
            false
        })
        .unwrap_err();
        assert_eq!(first.to_string(), "first");
        assert_eq!(second.kind(), ErrorKind::BadAlloc);
        assert_eq!(second.to_string(), "second");
    }

    #[test]
    fn every_error_code_maps_to_a_kind() {
        assert_eq!(
            error_kind(c_int_of(ffi::clingo_error_runtime)),
            ErrorKind::Runtime
        );
        assert_eq!(
            error_kind(c_int_of(ffi::clingo_error_logic)),
            ErrorKind::Logic
        );
        assert_eq!(
            error_kind(c_int_of(ffi::clingo_error_bad_alloc)),
            ErrorKind::BadAlloc
        );
        assert_eq!(
            error_kind(c_int_of(ffi::clingo_error_unknown)),
            ErrorKind::Unknown
        );
        assert_eq!(error_kind(-1), ErrorKind::Unknown);
        assert_eq!(error_kind(99), ErrorKind::Unknown);
    }

    #[test]
    fn c_str_rejects_nul() {
        assert_eq!(c_str("ab").unwrap().as_bytes(), b"ab");
        assert_eq!(c_str("a\0b").unwrap_err().kind(), ErrorKind::Nul);
    }

    #[test]
    fn borrowed_str_reads_utf8_and_rejects_the_rest() {
        let ok = CString::new("héllo").unwrap();
        // SAFETY: `ok` is a valid C string that outlives the borrow.
        assert_eq!(unsafe { borrowed_str(ok.as_ptr()) }.unwrap(), "héllo");
        let bad = CString::new(vec![b'a', 0xff]).unwrap();
        // SAFETY: `bad` is a valid C string that outlives the borrow.
        let err = unsafe { borrowed_str(bad.as_ptr()) }.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Utf8);
        // SAFETY: null is allowed.
        let err = unsafe { borrowed_str(std::ptr::null()) }.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unknown);
    }

    #[test]
    fn borrowed_str_lossy_copies() {
        let bad = CString::new(vec![b'a', 0xff]).unwrap();
        // SAFETY: `bad` is a valid C string for the duration of the call.
        let copied = unsafe { borrowed_str_lossy(bad.as_ptr()) }.unwrap();
        drop(bad);
        assert_eq!(copied, "a\u{fffd}");
        // SAFETY: null is allowed.
        assert_eq!(unsafe { borrowed_str_lossy(std::ptr::null()) }, None);
    }

    #[test]
    fn raw_slice_is_null_safe() {
        // SAFETY: null with any length is allowed.
        let empty: &[u64] = unsafe { raw_slice(std::ptr::null(), 3) };
        assert!(empty.is_empty());
        let data = [1_u64, 2, 3];
        // SAFETY: `data` has three elements and outlives the slice.
        assert_eq!(unsafe { raw_slice(data.as_ptr(), 3) }, &data);
        // SAFETY: a zero length never reads through the pointer.
        let none: &[u64] = unsafe { raw_slice(data.as_ptr(), 0) };
        assert!(none.is_empty());
    }

    #[test]
    fn fill_vec_queries_then_fills() {
        let v = fill_vec::<u32>(
            |len| {
                *len = 3;
                Ok(())
            },
            |ptr, len| {
                assert_eq!(len, 3);
                for i in 0..len {
                    // SAFETY: fill_vec passes a buffer of `len` elements.
                    unsafe { ptr.add(i).write(u32::try_from(i).unwrap() * 10) };
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(v, [0, 10, 20]);
    }

    #[test]
    fn fill_vec_passes_errors_through() {
        let err = fill_vec::<u32>(
            |_| Err(Error::new(ErrorKind::BadAlloc, "size")),
            |_, _| unreachable!("fill runs only after a successful size query"),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::BadAlloc);
        let err = fill_vec::<u32>(
            |len| {
                *len = 1;
                Ok(())
            },
            |_, _| Err(Error::new(ErrorKind::Runtime, "fill")),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime);
    }

    fn fake_fill(text: &'static [u8]) -> Result<String, Error> {
        fill_string(
            |len| {
                *len = text.len();
                Ok(())
            },
            |ptr, len| {
                for (i, byte) in text.iter().take(len).enumerate() {
                    // SAFETY: fill_string passes a buffer of `len` elements,
                    // and `i < len`.
                    unsafe { ptr.add(i).write(c_char::from_ne_bytes([*byte])) };
                }
                Ok(())
            },
        )
    }

    #[test]
    fn fill_string_reads_up_to_the_nul() {
        assert_eq!(fake_fill(b"p(1)\0").unwrap(), "p(1)");
        assert_eq!(fake_fill(b"\0").unwrap(), "");
    }

    #[test]
    fn fill_string_rejects_a_missing_nul_and_bad_utf8() {
        assert_eq!(fake_fill(b"abc").unwrap_err().kind(), ErrorKind::Unknown);
        assert_eq!(fake_fill(b"").unwrap_err().kind(), ErrorKind::Unknown);
        assert_eq!(fake_fill(b"a\xff\0").unwrap_err().kind(), ErrorKind::Utf8);
    }

    /// The real error state: a stale error set directly in clingo is not
    /// reported for a later bare `false`.
    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn clingo_error_state_is_reset_before_each_call() {
        // SAFETY: as in `ClingoErrorState::reset`.
        unsafe { ffi::clingo_set_error(c_int_of(ffi::clingo_error_runtime), c"stale".as_ptr()) };
        let err = call(|| false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unknown);
        assert_eq!(err.to_string(), "no message");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn the_linked_version_matches() {
        assert!(check_version().is_ok());
    }

    #[test]
    fn the_runtime_check_accepts_the_range_the_build_accepts() {
        let built_for = (5, 8, 2);
        for linked in [(5, 8, 1), (5, 8, 2), (5, 8, 3)] {
            assert!(version_accepted(linked, built_for), "{linked:?}");
        }
        for linked in [(5, 8, 0), (5, 7, 2), (5, 9, 0), (6, 8, 2), (4, 8, 2)] {
            assert!(!version_accepted(linked, built_for), "{linked:?}");
        }
    }
}

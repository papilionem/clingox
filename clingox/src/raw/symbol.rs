//! Wrappers for clingo's symbol functions (clingo.h:316-511).
//!
//! Symbols live in a global table that clingo never frees (DESIGN S12), so the
//! names, strings and argument arrays read from a symbol are valid for the rest
//! of the process, which is what makes the `'static` borrows below sound.

use std::ffi::{c_char, c_int, c_void};

use clingox_sys as ffi;

use super::capture::{Capture, MESSAGE_LIMIT};
use super::trampoline::logger;
use super::{RawSymbol, borrowed_str, c_str, call, fill_bytes, query, raw_slice, with_c_str};
use crate::Symbol;
use crate::error::{Error, ErrorKind, Message, MessageCode};

/// The type of a symbol (clingo.h:316-322).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SymbolType {
    Infimum,
    Number,
    String,
    Function,
    Supremum,
    /// A type clingo does not expose through its public constructors.
    Other(c_int),
}

pub(crate) fn create_number(number: i32) -> RawSymbol {
    let mut symbol = 0;
    // SAFETY: `symbol` is a valid out-pointer; the call cannot fail
    // (clingo.h:339).
    unsafe { ffi::clingo_symbol_create_number(number, &raw mut symbol) };
    symbol
}

pub(crate) fn create_supremum() -> RawSymbol {
    let mut symbol = 0;
    // SAFETY: `symbol` is a valid out-pointer; the call cannot fail
    // (clingo.h:343).
    unsafe { ffi::clingo_symbol_create_supremum(&raw mut symbol) };
    symbol
}

pub(crate) fn create_infimum() -> RawSymbol {
    let mut symbol = 0;
    // SAFETY: `symbol` is a valid out-pointer; the call cannot fail
    // (clingo.h:347).
    unsafe { ffi::clingo_symbol_create_infimum(&raw mut symbol) };
    symbol
}

pub(crate) fn create_string(string: &str) -> Result<RawSymbol, Error> {
    with_c_str(string, |string| {
        let mut symbol = 0;
        // SAFETY: `string` is a NUL-terminated string that outlives the call,
        // and clingo copies it into its symbol table (clingo.h:354). Creating
        // a symbol only interns it, so the call is a pure query.
        query(|| unsafe { ffi::clingo_symbol_create_string(string.as_ptr(), &raw mut symbol) })?;
        Ok(symbol)
    })
}

pub(crate) fn create_function(
    name: &str,
    arguments: &[Symbol],
    positive: bool,
) -> Result<RawSymbol, Error> {
    // `Symbol` is `repr(transparent)` over `clingo_symbol_t`, so the arguments
    // are already an array of `clingo_symbol_t`.
    let (pointer, len) = (arguments.as_ptr().cast::<RawSymbol>(), arguments.len());
    with_c_str(name, |name| {
        let mut symbol = 0;
        // SAFETY: `name` is a NUL-terminated string and `pointer` points to
        // `len` symbols, both outliving the call; clingo copies them into its
        // symbol table (clingo.h:377). An empty slice's dangling pointer is
        // never read, because the size is zero. Creating a symbol only interns
        // it, so the call is a pure query.
        query(|| unsafe {
            ffi::clingo_symbol_create_function(
                name.as_ptr(),
                pointer,
                len,
                positive,
                &raw mut symbol,
            )
        })?;
        Ok(symbol)
    })
}

pub(crate) fn symbol_type(symbol: RawSymbol) -> SymbolType {
    // SAFETY: any symbol value from clingo can be passed; the call only reads its
    // tag (clingo.h:440).
    let raw = unsafe { ffi::clingo_symbol_type(symbol) };
    match u32::try_from(raw) {
        Ok(ffi::clingo_symbol_type_infimum) => SymbolType::Infimum,
        Ok(ffi::clingo_symbol_type_number) => SymbolType::Number,
        Ok(ffi::clingo_symbol_type_string) => SymbolType::String,
        Ok(ffi::clingo_symbol_type_function) => SymbolType::Function,
        Ok(ffi::clingo_symbol_type_supremum) => SymbolType::Supremum,
        _ => SymbolType::Other(raw),
    }
}

pub(crate) fn symbol_number(symbol: RawSymbol) -> Result<i32, Error> {
    let mut number = 0;
    // SAFETY: `number` is a valid out-pointer; a symbol of another type is
    // reported as a runtime error, not undefined behaviour (clingo.h:392).
    query(|| unsafe { ffi::clingo_symbol_number(symbol, &raw mut number) })?;
    Ok(number)
}

pub(crate) fn symbol_name(symbol: RawSymbol) -> Result<&'static str, Error> {
    let mut name: *const c_char = std::ptr::null();
    // SAFETY: `name` is a valid out-pointer; a symbol of another type is reported
    // as a runtime error (clingo.h:402).
    query(|| unsafe { ffi::clingo_symbol_name(symbol, &raw mut name) })?;
    // SAFETY: the name is internalized and valid for the duration of the process
    // (clingo.h:396).
    Ok(unsafe { static_str(name) })
}

pub(crate) fn symbol_string(symbol: RawSymbol) -> Result<&'static str, Error> {
    let mut string: *const c_char = std::ptr::null();
    // SAFETY: `string` is a valid out-pointer; a symbol of another type is
    // reported as a runtime error (clingo.h:412).
    query(|| unsafe { ffi::clingo_symbol_string(symbol, &raw mut string) })?;
    // SAFETY: the string is internalized and valid for the duration of the
    // process (clingo.h:406).
    Ok(unsafe { static_str(string) })
}

pub(crate) fn symbol_is_positive(symbol: RawSymbol) -> Result<bool, Error> {
    let mut positive = false;
    // SAFETY: `positive` is a valid out-pointer; a symbol of another type is
    // reported as a runtime error (clingo.h:419).
    query(|| unsafe { ffi::clingo_symbol_is_positive(symbol, &raw mut positive) })?;
    Ok(positive)
}

pub(crate) fn symbol_arguments(symbol: RawSymbol) -> Result<&'static [Symbol], Error> {
    let mut arguments: *const RawSymbol = std::ptr::null();
    let mut size = 0;
    // SAFETY: both out-pointers are valid; a symbol of another type is reported as
    // a runtime error (clingo.h:434).
    query(|| unsafe { ffi::clingo_symbol_arguments(symbol, &raw mut arguments, &raw mut size) })?;
    // SAFETY: the arguments of a function symbol are stored with it in the symbol
    // table, which is never freed (DESIGN S12), so they are valid for 'static.
    // `Symbol` is `repr(transparent)` over `clingo_symbol_t`, so the cast keeps
    // size, alignment and validity. raw_slice handles the null pointer clingo
    // may return for no arguments.
    Ok(unsafe { raw_slice(arguments.cast::<Symbol>(), size) })
}

/// The text clingo prints for `symbol`; `Utf8` if a string in it is not
/// valid UTF-8, which only a string read from a file can be.
pub(crate) fn symbol_to_string(symbol: RawSymbol) -> Result<String, Error> {
    String::from_utf8(symbol_to_bytes(symbol)?).map_err(|e| {
        Error::new(
            ErrorKind::Utf8,
            format!("clingo printed a symbol that is {}", e.utf8_error()),
        )
    })
}

/// As [`symbol_to_string`], with U+FFFD in place of invalid UTF-8, the rule
/// of [`static_str`]: for display, which must not fail on such a string.
pub(crate) fn symbol_to_string_lossy(symbol: RawSymbol) -> Result<String, Error> {
    let bytes = symbol_to_bytes(symbol)?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    })
}

fn symbol_to_bytes(symbol: RawSymbol) -> Result<Vec<u8>, Error> {
    fill_bytes(
        // SAFETY: `size` is a valid out-pointer (clingo.h:447).
        |size| query(|| unsafe { ffi::clingo_symbol_to_string_size(symbol, size) }),
        // SAFETY: fill_bytes passes a buffer of exactly the size clingo asked for,
        // including the NUL (clingo.h:457).
        |buffer, size| query(|| unsafe { ffi::clingo_symbol_to_string(symbol, buffer, size) }),
    )
}

pub(crate) fn symbol_is_equal_to(a: RawSymbol, b: RawSymbol) -> bool {
    // SAFETY: a value query on two symbols that cannot fail (clingo.h:469).
    unsafe { ffi::clingo_symbol_is_equal_to(a, b) }
}

pub(crate) fn symbol_is_less_than(a: RawSymbol, b: RawSymbol) -> bool {
    // SAFETY: a value query on two symbols that cannot fail (clingo.h:479).
    unsafe { ffi::clingo_symbol_is_less_than(a, b) }
}

pub(crate) fn symbol_hash(symbol: RawSymbol) -> usize {
    // SAFETY: a value query on a symbol that cannot fail (clingo.h:484).
    unsafe { ffi::clingo_symbol_hash(symbol) }
}

/// The raw signature value, which `Signature` wraps.
pub(crate) type RawSignature = ffi::clingo_signature_t;

/// Creates a signature (clingo.h:267). A name with a NUL byte is
/// `ErrorKind::Nul`.
pub(crate) fn create_signature(
    name: &str,
    arity: u32,
    positive: bool,
) -> Result<RawSignature, Error> {
    with_c_str(name, |name| {
        let mut signature = 0;
        // SAFETY: `name` is a NUL-terminated string that outlives the call,
        // and clingo copies it into its symbol table (clingo.h:267). Creating
        // a signature only interns it, so the call is a pure query.
        query(|| unsafe {
            ffi::clingo_signature_create(name.as_ptr(), arity, positive, &raw mut signature)
        })?;
        Ok(signature)
    })
}

pub(crate) fn signature_name(signature: RawSignature) -> &'static str {
    // SAFETY: a signature's name lives in clingo's symbol table, which is never
    // freed (clingo.h:276, DESIGN S12).
    unsafe { static_str(ffi::clingo_signature_name(signature)) }
}

pub(crate) fn signature_arity(signature: RawSignature) -> u32 {
    // SAFETY: a value query on a signature that cannot fail (clingo.h:281).
    unsafe { ffi::clingo_signature_arity(signature) }
}

pub(crate) fn signature_is_positive(signature: RawSignature) -> bool {
    // SAFETY: a value query on a signature that cannot fail (clingo.h:286).
    unsafe { ffi::clingo_signature_is_positive(signature) }
}

pub(crate) fn signature_is_equal_to(a: RawSignature, b: RawSignature) -> bool {
    // SAFETY: a value query on two signatures that cannot fail (clingo.h:297).
    unsafe { ffi::clingo_signature_is_equal_to(a, b) }
}

pub(crate) fn signature_is_less_than(a: RawSignature, b: RawSignature) -> bool {
    // SAFETY: a value query on two signatures that cannot fail (clingo.h:306).
    unsafe { ffi::clingo_signature_is_less_than(a, b) }
}

pub(crate) fn signature_hash(signature: RawSignature) -> usize {
    // SAFETY: a value query on a signature that cannot fail (clingo.h:311).
    unsafe { ffi::clingo_signature_hash(signature) }
}

/// Parses a term with `clingo_parse_term`, capturing what clingo logs.
///
/// A failure clingo reports as a runtime error is a syntax error, since that is
/// the only runtime error the function raises (clingo.h:511).
pub(crate) fn parse_term(text: &str) -> Result<RawSymbol, Error> {
    let string = c_str(text)?;
    let capture = Capture::default();
    let data = std::ptr::from_ref(&capture).cast_mut().cast::<c_void>();
    let mut symbol = 0;
    // SAFETY: `string` is a NUL-terminated string and `symbol` a valid
    // out-pointer. The logger is called only during this call, on this thread,
    // with `data`, which points to `capture` on this stack frame and so outlives
    // the call (clingo.h:511, control.cc:1181-1199).
    let result = call(|| unsafe {
        ffi::clingo_parse_term(
            string.as_ptr(),
            Some(logger::<Capture>),
            data,
            MESSAGE_LIMIT,
            &raw mut symbol,
        )
    });
    capture.resume_panic();
    match result {
        Ok(()) => Ok(symbol),
        Err(err) if err.kind() == ErrorKind::Runtime => {
            // The term parser throws its syntax error instead of logging it, so the
            // error's own text, which has the same position prefix, is the message.
            let mut messages = capture.take();
            if messages.is_empty() {
                messages.push(Message::from_clingo(
                    MessageCode::RuntimeError,
                    err.raw_message(),
                ));
            }
            Err(err.with_kind(ErrorKind::Parse).with_messages(messages))
        }
        Err(err) => Err(err.with_messages(capture.take())),
    }
}

/// Reads a process-lifetime string from clingo's symbol table.
///
/// Program text passed through clingox is UTF-8, but a file clingo reads itself
/// may put other bytes into a name or string. Such a string is replaced by its
/// lossy UTF-8 form, internalized with `clingo_add_string` so it too lives for
/// the process and repeated reads return the same text without leaking. If even
/// that fails, the result is the replacement character.
///
/// # Safety
///
/// `ptr` must be null or point to a NUL-terminated string that is valid for the
/// rest of the process.
pub(super) unsafe fn static_str(ptr: *const c_char) -> &'static str {
    // SAFETY: the caller guarantees `ptr` is null or valid for the process.
    match unsafe { borrowed_str(ptr) } {
        Ok(text) => text,
        Err(_) if ptr.is_null() => "",
        // SAFETY: `ptr` is non-null and NUL-terminated (the caller's contract).
        Err(_) => unsafe { internalize_lossy(ptr) }.unwrap_or("\u{fffd}"),
    }
}

/// Internalizes the lossy UTF-8 form of a C string with `clingo_add_string`.
///
/// # Safety
///
/// `ptr` must point to a NUL-terminated string valid for the call.
unsafe fn internalize_lossy(ptr: *const c_char) -> Option<&'static str> {
    // SAFETY: the caller guarantees `ptr` is a valid NUL-terminated string.
    let lossy = unsafe { super::borrowed_str_lossy(ptr) }?;
    let lossy = c_str(&lossy).ok()?;
    let mut result: *const c_char = std::ptr::null();
    // SAFETY: `lossy` is NUL-terminated and outlives the call; clingo copies it
    // into its string table, which is never freed (clingo.h:497).
    call(|| unsafe { ffi::clingo_add_string(lossy.as_ptr(), &raw mut result) }).ok()?;
    // SAFETY: the internalized string is valid for the rest of the process
    // (clingo.h:490-491), and it is valid UTF-8 because `lossy` was.
    unsafe { borrowed_str(result) }.ok()
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn invalid_utf8_from_clingo_is_read_lossily_and_internalized() {
        let bad = CString::new(vec![b'n', 0xff, b'x']).unwrap();
        // SAFETY: `bad` is a valid C string for the duration of the call.
        let first = unsafe { internalize_lossy(bad.as_ptr()) }.unwrap();
        // SAFETY: as above.
        let second = unsafe { internalize_lossy(bad.as_ptr()) }.unwrap();
        drop(bad);
        assert_eq!(first, "n\u{fffd}x");
        // clingo internalizes equal strings once, so reading again does not leak.
        assert_eq!(first.as_ptr(), second.as_ptr());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn static_str_reads_valid_text_and_null() {
        let name = symbol_name(create_function("p", &[], true).unwrap()).unwrap();
        assert_eq!(name, "p");
        // SAFETY: null is allowed.
        assert_eq!(unsafe { static_str(std::ptr::null()) }, "");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn accessors_of_the_wrong_type_fail() {
        let number = create_number(1);
        assert_eq!(symbol_name(number).unwrap_err().kind(), ErrorKind::Runtime);
        assert_eq!(
            symbol_string(number).unwrap_err().kind(),
            ErrorKind::Runtime
        );
        assert_eq!(
            symbol_is_positive(number).unwrap_err().kind(),
            ErrorKind::Runtime
        );
        assert_eq!(
            symbol_arguments(number).unwrap_err().kind(),
            ErrorKind::Runtime
        );
        let function = create_function("f", &[], true).unwrap();
        assert_eq!(
            symbol_number(function).unwrap_err().kind(),
            ErrorKind::Runtime
        );
    }
}

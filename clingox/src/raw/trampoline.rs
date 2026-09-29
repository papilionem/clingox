//! Callbacks from C into Rust (DESIGN S8, S9).
//!
//! Every trampoline runs its Rust code inside [`guard`], so a panic never
//! unwinds into clingo's C++ frames, which would abort the process. A caught
//! panic goes into a first-writer-wins [`PanicSlot`] held by the callback
//! context, never by thread-local storage, because callbacks can run on solver
//! threads. The API method that made the C call resumes it on the caller's
//! thread.

use std::any::Any;
use std::borrow::Cow;
use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use clingox_sys as ffi;

use super::{ErrorState, borrowed_str_lossy, raw_slice};
use crate::error::{Error, ErrorKind};
use crate::{FunctionCall, Symbol};

/// A first-writer-wins slot in a callback context (S8): the first value
/// stored stays until it is taken, and later ones are dropped.
pub(crate) struct Slot<T> {
    /// Whether `value` holds something, changed only under the lock. Every
    /// callback asks [`Slot::is_set`] before it runs and the answer is almost
    /// always no, so that question is one atomic load instead of a lock.
    set: AtomicBool,
    value: Mutex<Option<T>>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Slot {
            set: AtomicBool::new(false),
            value: Mutex::new(None),
        }
    }
}

impl<T> Slot<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<T>> {
        // A poisoned lock only means another thread panicked while holding it,
        // and this lock is never held across user code, so the data is intact.
        self.value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records a value unless one is already recorded.
    pub(crate) fn store(&self, value: T) {
        let mut slot = self.lock();
        slot.get_or_insert(value);
        self.set.store(true, Ordering::Release);
    }

    /// Whether a value is recorded.
    #[inline]
    pub(crate) fn is_set(&self) -> bool {
        self.set.load(Ordering::Acquire)
    }

    /// Takes the recorded value, leaving the slot empty.
    pub(crate) fn take(&self) -> Option<T> {
        if !self.is_set() {
            return None;
        }
        let mut slot = self.lock();
        let value = slot.take();
        self.set.store(false, Ordering::Release);
        value
    }

    /// Reads the recorded value through `f` without removing it, so a later
    /// call sees the same value again (`raw::events:: EventHandlerSlots`'s own
    /// doc comment): unlike [`Slot::take`], this never empties the slot.
    pub(crate) fn peek_with<R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        if !self.is_set() {
            return None;
        }
        self.lock().as_ref().map(f)
    }
}

impl<T> fmt::Debug for Slot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slot").field("set", &self.is_set()).finish()
    }
}

/// The first panic caught in a callback, waiting to be resumed by the API
/// method on the caller's thread.
pub(crate) type PanicSlot = Slot<Box<dyn Any + Send>>;

impl PanicSlot {
    /// Resumes a recorded panic on the current thread (S8).
    pub(crate) fn resume(&self) {
        if let Some(payload) = self.take() {
            resume_unwind(payload);
        }
    }
}

/// Runs the Rust side of a callback and catches any panic into `slot`.
///
/// Returns `None` if `f` panicked, or if an earlier callback already did: after
/// the first failure every later callback stops at once (S8).
pub(crate) fn guard<R>(slot: &PanicSlot, f: impl FnOnce() -> R) -> Option<R> {
    if slot.is_set() {
        return None;
    }
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => Some(value),
        Err(payload) => {
            slot.store(payload);
            None
        }
    }
}

/// The receiver of clingo's log messages.
///
/// clingo may call the logger from any thread, so a sink is `Sync`.
pub(crate) trait LogSink: Sync {
    /// Receives one message with its warning code.
    fn message(&self, code: c_int, text: String);
    /// Where a panic in [`LogSink::message`] is recorded.
    fn panic_slot(&self) -> &PanicSlot;
}

/// The `clingo_logger_t` trampoline for a sink of type `S`.
///
/// The logger returns nothing, so it cannot report a failure to clingo (S9). A
/// panic is recorded in the sink's slot and resumed by the API method after
/// the C call returns.
///
/// # Safety
///
/// `data` must point to an `S` that is valid for the duration of the call, and
/// `message` must be null or a NUL-terminated string valid for the call
/// (clingo.h:187).
pub(crate) unsafe extern "C" fn logger<S: LogSink>(
    code: c_int,
    message: *const c_char,
    data: *mut c_void,
) {
    // SAFETY: the caller passes the sink pointer it registered with clingo,
    // which points to a live `S`. Only shared access is taken, and `S: Sync`,
    // so calls from several solver threads at once are sound.
    let sink = unsafe { &*data.cast::<S>().cast_const() };
    // SAFETY: `message` is null or a NUL-terminated string valid for this call
    // (the caller's contract); it is copied at once.
    let text = unsafe { borrowed_str_lossy(message) }.unwrap_or_default();
    // A panic is already recorded in the slot; nothing can be reported here.
    let _ = guard(sink.panic_slot(), || sink.message(code, text));
}

/// The user's function behind a ground callback: it receives each call of an
/// external function `@name(args)` and pushes the values.
pub(crate) type GroundFunction<'f> = dyn FnMut(&mut FunctionCall<'_>) -> Result<(), Error> + 'f;

/// What the data pointer of a ground callback points to (DESIGN S8).
///
/// Grounding runs on the thread that called `clingo_control_ground`, so the
/// function is borrowed and need not be `Send` (S10). The slots are first
/// writer wins: once one is set, every later call fails at once.
pub(crate) struct GroundContext<'f> {
    function: RefCell<&'f mut GroundFunction<'f>>,
    /// The buffer the values of each call are collected in, kept empty
    /// between calls so that only the first call allocates.
    values: RefCell<Vec<Symbol>>,
    pub(crate) error: Slot<Error>,
    pub(crate) panic: PanicSlot,
}

impl<'f> GroundContext<'f> {
    pub(crate) fn new(function: &'f mut GroundFunction<'f>) -> Self {
        GroundContext {
            function: RefCell::new(function),
            values: RefCell::new(Vec::new()),
            error: Slot::default(),
            panic: PanicSlot::default(),
        }
    }

    /// Runs the user's function on one call and returns the values it pushed.
    fn call(&self, name: Cow<'_, str>, args: &[Symbol]) -> Result<Vec<Symbol>, Error> {
        // Nothing reachable from the function can ground this control again,
        // since `ground_with` borrows it mutably, so the cell is never borrowed
        // twice; the error only guards that reasoning.
        let mut function = self.function.try_borrow_mut().map_err(|_| {
            Error::new(
                ErrorKind::Logic,
                "a ground callback was entered while it was running",
            )
        })?;
        let mut call = FunctionCall::new(name, args, self.values.take());
        (*function)(&mut call)?;
        Ok(call.into_values())
    }

    /// Takes back the buffer of a call whose values clingo has copied.
    fn recycle(&self, mut values: Vec<Symbol>) {
        values.clear();
        self.values.replace(values);
    }
}

impl fmt::Debug for GroundContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroundContext")
            .field("error", &self.error)
            .field("panic", &self.panic)
            .finish_non_exhaustive()
    }
}

/// The `clingo_ground_callback_t` trampoline (clingo.h:2914-2956), setting
/// the error state `S` (clingo's own, except in tests under Miri).
///
/// It calls the user's function inside [`guard`] and passes the pushed values
/// to clingo through `symbol_callback`. A returned error or a panic goes into
/// the context's slots, and the trampoline returns `false` after setting
/// clingo's error state (S8); `ground_with` then reports the slot, never
/// clingo's own message.
///
/// # Safety
///
/// `data` must point to a [`GroundContext`] that is valid for the call.
/// `name` must be null or a NUL-terminated string, and `arguments` null or
/// `arguments_size` symbols, both valid for the call. `symbol_callback` and
/// `symbol_callback_data` must be what clingo passed for this call
/// (clingo.h:2914-2956).
pub(crate) unsafe extern "C" fn ground_function<S: ErrorState>(
    _location: *const ffi::clingo_location_t,
    name: *const c_char,
    arguments: *const ffi::clingo_symbol_t,
    arguments_size: usize,
    data: *mut c_void,
    symbol_callback: ffi::clingo_symbol_callback_t,
    symbol_callback_data: *mut c_void,
) -> bool {
    // SAFETY: `data` is the context `ground_with` registered for this call to
    // clingo_control_ground, which outlives the call (the caller's contract).
    // Grounding runs on one thread, and only shared access is taken.
    let context = unsafe { &*data.cast::<GroundContext<'_>>().cast_const() };
    if context.error.is_set() {
        fail::<S>(c"an earlier ground callback failed");
        return false;
    }
    let name = if name.is_null() {
        Cow::Borrowed("")
    } else {
        // SAFETY: `name` is a non-null NUL-terminated string valid for this
        // call (the caller's contract); the borrow ends with the call.
        unsafe { CStr::from_ptr(name) }.to_string_lossy()
    };
    // SAFETY: `arguments` is null or points to `arguments_size` symbols valid
    // for this call (the caller's contract). `Symbol` is `repr(transparent)`
    // over `clingo_symbol_t`.
    let args = unsafe { raw_slice(arguments.cast::<Symbol>(), arguments_size) };
    let values = match guard(&context.panic, || context.call(name, args)) {
        Some(Ok(values)) => values,
        Some(Err(err)) => {
            context.error.store(err);
            fail::<S>(c"a ground callback returned an error");
            return false;
        }
        None => {
            fail::<S>(c"a ground callback panicked");
            return false;
        }
    };
    let Some(symbol_callback) = symbol_callback else {
        fail::<S>(c"clingo passed no symbol callback");
        return false;
    };
    if values.is_empty() {
        // clingo treats a call that passes no values as an empty result.
        context.recycle(values);
        return true;
    }
    // SAFETY: `symbol_callback` and its data are the ones clingo passed for
    // this call, and `values` holds `values.len()` symbols (`Symbol` is
    // `repr(transparent)`) that outlive it; clingo copies them. On failure
    // clingo has set its own error state (clingo.h:681-693).
    let accepted = unsafe {
        symbol_callback(
            values.as_ptr().cast::<ffi::clingo_symbol_t>(),
            values.len(),
            symbol_callback_data,
        )
    };
    context.recycle(values);
    accepted
}

/// Sets clingo's error state before a trampoline returns `false` (S8). The
/// code is `unknown`, as clingo.h asks for errors not related to clingo
/// (clingo.h:2919-2921); the API method reports the slot instead.
///
/// `pub(crate)` so the observer's 19 trampolines (`raw::observer`) share it
/// instead of redefining the same two lines.
pub(crate) fn fail<S: ErrorState>(message: &CStr) {
    S::set(ffi::clingo_error_unknown, message);
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::{CString, c_uint};
    use std::sync::Mutex;

    use super::*;

    thread_local! {
        /// The code the trampoline set last, in place of clingo's error state.
        static SET: Cell<Option<c_uint>> = const { Cell::new(None) };
    }

    #[test]
    fn a_slot_keeps_the_first_value_and_says_whether_it_holds_one() {
        let slot = Slot::<u32>::default();
        assert!(!slot.is_set());
        assert_eq!(slot.take(), None);
        assert_eq!(slot.peek_with(|v| *v), None);
        slot.store(1);
        slot.store(2);
        assert!(slot.is_set());
        assert_eq!(slot.peek_with(|v| *v), Some(1));
        assert!(slot.is_set(), "peeking leaves the value");
        assert_eq!(slot.take(), Some(1), "the first value wins");
        assert!(!slot.is_set());
        assert_eq!(slot.take(), None);
        slot.store(3);
        assert_eq!(slot.take(), Some(3), "a taken slot can be filled again");
    }

    #[test]
    #[allow(
        clippy::print_stderr,
        reason = "a visible skip, as the other thread tests"
    )]
    fn a_value_stored_on_another_thread_is_seen_as_set() {
        if !clingox_sys::HAS_THREADS {
            eprintln!("skipped: this build has no threads");
            return;
        }
        let slot = Slot::<u32>::default();
        std::thread::scope(|scope| {
            scope.spawn(|| slot.store(7));
        });
        assert!(slot.is_set());
        assert_eq!(slot.take(), Some(7));
    }

    /// An error state that records the code, so the trampolines run under
    /// Miri without clingo.
    enum Recorded {}

    impl ErrorState for Recorded {
        fn set(code: c_uint, _: &CStr) {
            SET.with(|s| s.set(Some(code)));
        }
        fn code() -> c_int {
            0
        }
        fn message() -> Option<String> {
            None
        }
    }

    /// A fake `clingo_symbol_callback_t` whose data is a `Vec<u64>` that
    /// receives the symbols.
    unsafe extern "C" fn collect(symbols: *const u64, size: usize, data: *mut c_void) -> bool {
        // SAFETY: the tests pass a `Vec<u64>` as data, and the trampoline
        // passes `size` symbols.
        let (out, symbols) = unsafe { (&mut *data.cast::<Vec<u64>>(), raw_slice(symbols, size)) };
        out.extend_from_slice(symbols);
        true
    }

    /// Calls the ground trampoline as clingo would, with fake arguments.
    fn evaluate(
        context: &GroundContext<'_>,
        name: &CStr,
        args: &[u64],
        out: &mut Vec<u64>,
    ) -> bool {
        SET.with(|s| s.set(None));
        // SAFETY: the context, the name and the arguments outlive the call,
        // and `collect` takes `out` as its data, as it expects.
        unsafe {
            ground_function::<Recorded>(
                std::ptr::null(),
                name.as_ptr(),
                args.as_ptr(),
                args.len(),
                std::ptr::from_ref(context).cast_mut().cast(),
                Some(collect),
                std::ptr::from_mut(out).cast(),
            )
        }
    }

    #[test]
    fn a_ground_callback_passes_its_name_arguments_and_values() {
        let mut seen = Vec::new();
        let mut function = |call: &mut FunctionCall<'_>| {
            seen.push((call.name().to_owned(), call.args().len()));
            for arg in call.args().to_vec() {
                call.push(arg)?;
            }
            call.push(Symbol::from_clingo(7))
        };
        let context = GroundContext::new(&mut function);
        let mut out = Vec::new();
        assert!(evaluate(&context, c"f", &[1, 2], &mut out));
        assert_eq!(out, [1, 2, 7]);
        assert_eq!(SET.with(Cell::get), None, "no error on success");
        drop(context);
        assert_eq!(seen, [("f".to_owned(), 2)]);
    }

    #[test]
    fn a_ground_callback_without_values_passes_none() {
        let mut function = |_: &mut FunctionCall<'_>| Ok(());
        let context = GroundContext::new(&mut function);
        let mut out = Vec::new();
        assert!(evaluate(&context, c"none", &[], &mut out));
        assert!(out.is_empty());
    }

    /// The buffer that collects a call's values is kept for the next call, so
    /// a later call must never see what an earlier one pushed.
    #[test]
    fn each_ground_callback_call_starts_with_no_values() {
        let mut function = |call: &mut FunctionCall<'_>| {
            for arg in call.args().to_vec() {
                call.push(arg)?;
            }
            Ok(())
        };
        let context = GroundContext::new(&mut function);
        for args in [&[1, 2, 3][..], &[][..], &[9][..], &[4, 5][..]] {
            let mut out = Vec::new();
            assert!(evaluate(&context, c"f", args, &mut out));
            assert_eq!(out, args);
        }
    }

    #[test]
    fn invalid_utf8_in_a_function_name_is_replaced() {
        let mut names = Vec::new();
        let mut function = |call: &mut FunctionCall<'_>| {
            names.push(call.name().to_owned());
            Ok(())
        };
        let context = GroundContext::new(&mut function);
        let bad = CString::new(vec![b'f', 0xff]).unwrap();
        assert!(evaluate(&context, &bad, &[], &mut Vec::new()));
        drop(context);
        assert_eq!(names, ["f\u{fffd}"]);
    }

    #[test]
    fn an_error_from_a_ground_callback_is_kept_and_stops_later_calls() {
        let mut calls = 0;
        let mut function = |_: &mut FunctionCall<'_>| {
            calls += 1;
            Err(Error::new(ErrorKind::Conversion, "not a number"))
        };
        let context = GroundContext::new(&mut function);
        let mut out = Vec::new();
        assert!(!evaluate(&context, c"f", &[], &mut out));
        assert_eq!(SET.with(Cell::get), Some(ffi::clingo_error_unknown));
        assert!(
            !evaluate(&context, c"f", &[], &mut out),
            "the slot stays set"
        );
        assert!(out.is_empty());
        let err = context.error.take().unwrap();
        assert_eq!(err.kind(), ErrorKind::Conversion);
        drop(context);
        assert_eq!(calls, 1, "the second call fails before the function runs");
    }

    #[test]
    fn a_panic_in_a_ground_callback_is_caught() {
        let mut function = |_: &mut FunctionCall<'_>| -> Result<(), Error> { panic!("stop") };
        let context = GroundContext::new(&mut function);
        assert!(!evaluate(&context, c"f", &[], &mut Vec::new()));
        assert_eq!(SET.with(Cell::get), Some(ffi::clingo_error_unknown));
        let payload = context.panic.take().unwrap();
        assert_eq!(*payload.downcast::<&str>().unwrap(), "stop");
    }

    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<(c_int, String)>>,
        slot: PanicSlot,
    }

    impl LogSink for Recorder {
        fn message(&self, code: c_int, text: String) {
            assert!(text != "panic", "the sink panicked on purpose");
            self.seen.lock().unwrap().push((code, text));
        }
        fn panic_slot(&self) -> &PanicSlot {
            &self.slot
        }
    }

    fn send(recorder: &Recorder, code: c_int, text: Option<&str>) {
        let owned = text.map(|t| CString::new(t).unwrap());
        let ptr = owned.as_ref().map_or(std::ptr::null(), |m| m.as_ptr());
        let data = std::ptr::from_ref(recorder).cast_mut().cast::<c_void>();
        // SAFETY: `data` points to a live Recorder and `ptr` is null or a valid
        // C string, both outliving the call.
        unsafe { logger::<Recorder>(code, ptr, data) };
    }

    #[test]
    fn the_logger_delivers_messages() {
        let recorder = Recorder::default();
        send(
            &recorder,
            2,
            Some("<block>:1:1-2: info: atom does not occur"),
        );
        send(&recorder, 6, None);
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            [
                (2, "<block>:1:1-2: info: atom does not occur".to_owned()),
                (6, String::new())
            ]
        );
    }

    #[test]
    fn a_panic_in_the_logger_is_caught_and_stops_later_calls() {
        let recorder = Recorder::default();
        send(&recorder, 0, Some("panic"));
        send(&recorder, 0, Some("after"));
        assert!(recorder.slot.is_set());
        assert!(recorder.seen.lock().unwrap().is_empty());
        let payload = recorder.slot.take().unwrap();
        assert!(payload.downcast_ref::<&str>().is_some() || payload.is::<String>());
        assert!(!recorder.slot.is_set());
    }

    #[test]
    fn the_first_panic_wins() {
        let slot = PanicSlot::default();
        slot.store(Box::new("first"));
        slot.store(Box::new("second"));
        assert_eq!(*slot.take().unwrap().downcast::<&str>().unwrap(), "first");
    }

    #[test]
    fn a_resumed_panic_reaches_the_caller() {
        let slot = PanicSlot::default();
        assert_eq!(guard(&slot, || 7), Some(7));
        assert_eq!(guard(&slot, || panic!("boom")), None::<()>);
        let caught = catch_unwind(AssertUnwindSafe(|| slot.resume())).unwrap_err();
        assert_eq!(*caught.downcast::<&str>().unwrap(), "boom");
        // The slot is empty again, so resuming twice does nothing.
        slot.resume();
    }
}

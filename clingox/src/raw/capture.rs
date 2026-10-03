//! The internal logger that captures clingo's messages during a call
//! (DESIGN S2), and passes each one on to the user's logger or the `log`
//! crate.

use std::ffi::c_int;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use clingox_sys as ffi;

use super::trampoline::{LogSink, PanicSlot};
use crate::error::{Message, MessageCode};

/// The most messages clingo passes to the logger per call by default, as in
/// clingo's own C++ API (`g_message_limit`, clingo.hh:47).
pub(crate) const MESSAGE_LIMIT: u32 = 20;

/// A logger supplied by the user. The control owns it, and clingo may call it
/// from any thread (DESIGN S10), hence `Send`.
pub(crate) type UserLogger = Box<dyn FnMut(MessageCode, &str) + Send>;

/// Collects the messages clingo logs, and passes each one on to the user's
/// logger if there is one, and to the `log` crate otherwise.
///
/// Its address is registered with clingo as the logger's data, so it lives in a
/// `Box` (or on the stack for a single call) and never moves while registered.
#[derive(Default)]
pub(crate) struct Capture {
    messages: Mutex<Vec<Message>>,
    /// Whether `messages` holds any, changed only under its lock: almost
    /// every call logs nothing, and [`Capture::take`] then needs no lock.
    any: AtomicBool,
    /// The user's logger. The mutex serialises the calls, because clingo may
    /// log from several solver threads at once and an `FnMut` must not be
    /// entered twice.
    user: Option<Mutex<UserLogger>>,
    /// A panic in the capture itself, caught by the trampoline (S8).
    panic: PanicSlot,
    /// A panic in the user's logger or a `log` backend. It has its own slot so
    /// that the messages after it are still captured; only passing them on
    /// stops until the panic has resumed (S9).
    forward_panic: PanicSlot,
}

impl Capture {
    /// A capture that passes messages to `user` instead of the `log` crate.
    pub(super) fn with_logger(user: Option<UserLogger>) -> Self {
        Capture {
            user: user.map(Mutex::new),
            ..Capture::default()
        }
    }

    /// Takes the messages captured so far.
    pub(super) fn take(&self) -> Vec<Message> {
        if !self.any.load(Ordering::Acquire) {
            return Vec::new();
        }
        let mut messages = self
            .messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.any.store(false, Ordering::Release);
        std::mem::take(&mut *messages)
    }

    /// Resumes a panic raised while clingo called the logger (S8, S9): by the
    /// user's logger, by a `log` implementation, or by the capture itself.
    /// Both slots are emptied, so a second panic cannot resume later in an
    /// unrelated call; the first one recorded is resumed.
    pub(super) fn resume_panic(&self) {
        let own = self.panic.take();
        let forwarded = self.forward_panic.take();
        if let Some(payload) = own.or(forwarded) {
            std::panic::resume_unwind(payload);
        }
    }

    /// Records `payload` as if the logger had panicked, for tests of what
    /// clingo cannot be made to do, such as logging while a search closes.
    #[cfg(test)]
    pub(super) fn inject_logger_panic(&self, payload: Box<dyn std::any::Any + Send>) {
        self.forward_panic.store(payload);
    }

    /// Passes a message on to the user's logger, or to `log` without one. A
    /// panic is recorded, and later messages are not passed on until it has
    /// resumed.
    fn forward(&self, message: &Message) {
        if self.forward_panic.is_set() {
            return;
        }
        let outcome = match &self.user {
            Some(user) => {
                // The panic is caught while the guard is held, so the mutex is
                // never poisoned by the user's code.
                let mut logger = user
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                catch_unwind(AssertUnwindSafe(|| {
                    logger(message.code(), message.text());
                }))
            }
            None => catch_unwind(|| forward_to_log(message)),
        };
        if let Err(payload) = outcome {
            self.forward_panic.store(payload);
        }
    }
}

impl fmt::Debug for Capture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Capture")
            .field("user_logger", &self.user.is_some())
            .field("panic", &self.panic)
            .field("forward_panic", &self.forward_panic)
            .finish_non_exhaustive()
    }
}

impl LogSink for Capture {
    fn message(&self, code: c_int, text: String) {
        let message = Message::from_clingo(message_code(code), &text);
        self.forward(&message);
        let mut messages = self
            .messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        messages.push(message);
        self.any.store(true, Ordering::Release);
    }

    fn panic_slot(&self) -> &PanicSlot {
        &self.panic
    }
}

/// Sends a message to the `log` crate: errors at `Error`, everything else at
/// `Warn`, which is how clingo's own application ranks them.
#[cfg(feature = "log")]
fn forward_to_log(message: &Message) {
    let level = if message.is_error() {
        log::Level::Error
    } else {
        log::Level::Warn
    };
    log::log!(target: "clingox", level, "{}", message.text());
}

/// Without the feature `log`, messages are only captured.
#[cfg(not(feature = "log"))]
fn forward_to_log(_: &Message) {}

/// Maps clingo's `clingo_warning_t` (clingo.h:165-173). A code clingo may add
/// later is `Other`.
pub(super) fn message_code(code: c_int) -> MessageCode {
    match u32::try_from(code) {
        Ok(ffi::clingo_warning_operation_undefined) => MessageCode::OperationUndefined,
        Ok(ffi::clingo_warning_runtime_error) => MessageCode::RuntimeError,
        Ok(ffi::clingo_warning_atom_undefined) => MessageCode::AtomUndefined,
        Ok(ffi::clingo_warning_file_included) => MessageCode::FileIncluded,
        Ok(ffi::clingo_warning_variable_unbounded) => MessageCode::VariableUnbounded,
        Ok(ffi::clingo_warning_global_variable) => MessageCode::GlobalVariable,
        _ => MessageCode::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::super::trampoline::guard;
    use super::*;

    #[test]
    fn warning_codes_map_to_message_codes() {
        let code = |c: u32| message_code(c_int::try_from(c).unwrap());
        assert_eq!(
            code(ffi::clingo_warning_operation_undefined),
            MessageCode::OperationUndefined
        );
        assert_eq!(
            code(ffi::clingo_warning_runtime_error),
            MessageCode::RuntimeError
        );
        assert_eq!(
            code(ffi::clingo_warning_atom_undefined),
            MessageCode::AtomUndefined
        );
        assert_eq!(
            code(ffi::clingo_warning_file_included),
            MessageCode::FileIncluded
        );
        assert_eq!(
            code(ffi::clingo_warning_variable_unbounded),
            MessageCode::VariableUnbounded
        );
        assert_eq!(
            code(ffi::clingo_warning_global_variable),
            MessageCode::GlobalVariable
        );
        assert_eq!(code(ffi::clingo_warning_other), MessageCode::Other);
        assert_eq!(message_code(-1), MessageCode::Other);
        assert_eq!(message_code(100), MessageCode::Other);
    }

    #[test]
    fn captured_messages_are_taken_once() {
        let capture = Capture::default();
        capture.message(
            ffi_code(ffi::clingo_warning_runtime_error),
            "<block>:1:8-9: error: x\n".into(),
        );
        let taken = capture.take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].code(), MessageCode::RuntimeError);
        assert_eq!(taken[0].text(), "<block>:1:8-9: error: x");
        assert_eq!(taken[0].location().map(crate::Location::line), Some(1));
        assert_eq!(capture.take(), []);
    }

    #[test]
    fn a_panic_in_the_user_logger_stops_forwarding_but_not_capturing() {
        let calls = std::sync::Arc::new(Mutex::new(0));
        let counter = std::sync::Arc::clone(&calls);
        let capture = Capture::with_logger(Some(Box::new(move |_, _: &str| {
            *counter.lock().unwrap() += 1;
            panic!("the logger failed on purpose");
        })));
        let code = ffi_code(ffi::clingo_warning_atom_undefined);
        capture.message(code, "first".into());
        capture.message(code, "second".into());
        assert_eq!(*calls.lock().unwrap(), 1, "no call after the panic");
        assert_eq!(capture.take().len(), 2, "both messages are captured");
        let resumed = catch_unwind(AssertUnwindSafe(|| capture.resume_panic()));
        assert!(resumed.is_err(), "the panic resumes");
        capture.message(code, "third".into());
        assert_eq!(*calls.lock().unwrap(), 2, "the logger is called again");
        drop(catch_unwind(AssertUnwindSafe(|| capture.resume_panic())));
    }

    /// A panic inside the capture itself, which the logger
    /// trampoline catches into `panic_slot`, resumes from `resume_panic`.
    #[test]
    fn a_panic_in_the_capture_itself_resumes() {
        let capture = Capture::default();
        // What `logger::<Capture>` does around `Capture::message`.
        let caught = guard(capture.panic_slot(), || -> () {
            panic!("the capture failed on purpose");
        });
        assert!(caught.is_none());
        let resumed = catch_unwind(AssertUnwindSafe(|| capture.resume_panic()))
            .expect_err("the panic resumes");
        assert_eq!(
            resumed.downcast_ref::<&str>(),
            Some(&"the capture failed on purpose")
        );
        // It resumes once.
        capture.resume_panic();
    }

    fn ffi_code(code: u32) -> c_int {
        c_int::try_from(code).unwrap()
    }
}

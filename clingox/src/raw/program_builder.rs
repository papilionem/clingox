//! Wrappers for the program builder: `clingo_program_builder_init`, `_begin`,
//! `_end` and `_add` (clingo.h:4080-4130).
//!
//! No callback runs during any of these calls except the logger, which
//! [`ControlHandle::captured`] already collects, so there is no trampoline
//! here.

use std::ptr::NonNull;

use clingox_sys as ffi;

use super::RawAst;
use super::call;
use super::control::ControlHandle;
use super::script::ScriptFrame;
use crate::error::{Error, ErrorKind};

/// clingo raises a runtime error from these calls only for a statement or a
/// parse it cannot accept, which is a [`ErrorKind::Parse`] failure here, as
/// for `Control::add`.
fn parse_kind(err: Error) -> Error {
    match err.kind() {
        ErrorKind::Runtime => err.with_kind(ErrorKind::Parse),
        _ => err,
    }
}

impl ControlHandle {
    /// Gets the control's program builder and begins it (clingo.h:4080-4099).
    /// Any leftover search, backend or program builder is closed first (S4).
    pub(crate) fn open_program_builder(&self) -> Result<(), Error> {
        // A leftover search, not the session this call opens: discard its
        // handler's own failure, never promote.
        self.close_solve(false)?;
        self.close_backend()?;
        self.close_program_builder()?;
        let mut ptr: *mut ffi::clingo_program_builder_t = std::ptr::null_mut();
        self.captured(|| {
            // SAFETY: `self.ptr` is the live control this handle owns, and no
            // search or session is open (all closed above). `ptr` is a valid
            // out-pointer (clingo.h:4080-4085).
            unsafe { ffi::clingo_program_builder_init(self.ptr.as_ptr(), &raw mut ptr) }
        })?;
        let ptr = NonNull::new(ptr).ok_or_else(|| {
            Error::new(
                ErrorKind::Unknown,
                "clingo reported success but returned no program builder",
            )
        })?;
        self.captured(|| {
            // SAFETY: `ptr` is the builder clingo just returned for this
            // control, not begun yet, used on the thread that owns the control
            // (clingo.h:4087-4092).
            unsafe { ffi::clingo_program_builder_begin(ptr.as_ptr()) }
        })
        // `begin` fails with a runtime error when the control has already
        // logged an error ("parsing failed").
        .map_err(parse_kind)?;
        self.builder.set(Some(ptr));
        Ok(())
    }

    /// Ends the open program builder, if any (clingo.h:4094-4099). A session
    /// still open is ended at most once, whatever the outcome: the record is
    /// taken out before the call.
    ///
    /// Like [`ControlHandle::close_backend`], this does not resume a pending
    /// logger panic itself, because it also runs from [`Drop`].
    pub(crate) fn close_program_builder(&self) -> Result<(), Error> {
        let Some(ptr) = self.builder.take() else {
            return Ok(());
        };
        drop(self.capture.take());
        let result = call(|| {
            // SAFETY: `ptr` was begun by `open_program_builder` and is ended
            // here exactly once: it was taken out of the `Cell`, its only
            // copy, just above (clingo.h:4094-4099).
            unsafe { ffi::clingo_program_builder_end(ptr.as_ptr()) }
        });
        let messages = self.capture.take();
        result.map_err(|err| err.with_messages(messages))
    }

    /// Adds a statement to the open program builder (clingo.h:4101-4108).
    /// clingo acquires its own reference to `ast`, which stays valid.
    pub(crate) fn program_builder_add(&self, ast: RawAst) -> Result<(), Error> {
        let ptr = self.builder.get().map(NonNull::as_ptr).ok_or_else(|| {
            Error::new(
                ErrorKind::Logic,
                "no program builder is open on this control",
            )
        })?;
        // A `#script` statement runs its `execute` during the call.
        let frame = ScriptFrame::open();
        let result = self.captured(|| {
            // SAFETY: `ptr` is the open, begun builder of this control, and
            // `ast` a live node the caller borrows for the call.
            unsafe { ffi::clingo_program_builder_add(ptr, ast.as_ptr()) }
        });
        self.keep_script_failure(frame);
        result.map_err(parse_kind)
    }
}

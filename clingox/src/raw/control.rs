//! Wrappers for the control object, grounding, blocking solving and externals
//! (clingo.h:2908-3137, 2577-2643, 587-683).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt;
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use clingox_sys as ffi;

use super::capture::{Capture, UserLogger};
use super::events::EventHandlerSlots;
use super::interrupt::SolveSync;
use super::observer::ObserverSlots;
use super::propagate::PropagatorSlots;
use super::script::ScriptFrame;
use super::solve::ActiveSolve;
use super::trampoline::{GroundContext, GroundFunction, PanicSlot, Slot, ground_function, logger};
use super::{ClingoErrorState, RawSymbol, c_int_of, call};
use crate::error::{Error, ErrorKind};
use crate::{Symbol, TruthValue};

/// An owned `clingo_control_t` with its internal logger.
///
/// It is `Send` (see the `unsafe impl` below) and `!Sync`, through the `Cell`
/// and the raw pointer (DESIGN S12). The capture
/// is boxed so the address registered with clingo as logger data stays fixed
/// when the handle moves, and it is dropped only after the control is freed.
///
/// A search is recorded in `solve` from its start until it is closed, because
/// nothing may rely on a guard's `Drop` to close it (DESIGN S4). Every call on
/// the control closes a recorded search first, as does `Drop`, so no control
/// function ever runs while a search is open (clingo.h:2981-2983). It is a
/// `Cell` so that calls taking `&self` can close it too.
///
/// `sync` is shared with every `InterruptHandle` and registered as the data of
/// each search's event handler; it decides when an interrupt may reach clingo
/// (see `raw::interrupt`). It outlives the control: the handle drops its
/// reference only after `clingo_control_free`.
pub(crate) struct ControlHandle {
    pub(super) ptr: NonNull<ffi::clingo_control_t>,
    /// Whether `Drop` frees `ptr`. Fixed at construction: `false` only for the
    /// control clingo owns and lends to an application's `main`
    /// ([`ControlHandle::borrowed`]).
    owned: bool,
    /// For a borrowed handle: where the registered observers and propagators
    /// go when the handle is dropped, so they live until `clingo_main` has
    /// returned (see [`Arena`]). `None` for an owned handle.
    arena: Option<Arc<Arena>>,
    pub(super) capture: Box<Capture>,
    pub(super) solve: Cell<Option<ActiveSolve>>,
    pub(super) sync: Arc<SolveSync>,
    /// The backend, from `clingo_backend_begin` until `clingo_backend_end`
    /// (DESIGN S4). It is a `Cell` for the same reason `solve` is: closing a
    /// leftover backend must work from `&self`, so a forgotten or
    /// panic-abandoned one is finished by the next entry point or by `Drop`,
    /// never by a guard's own `Drop` (`raw::backend`).
    pub(super) backend: Cell<Option<NonNull<ffi::clingo_backend_t>>>,
    /// The program builder, from `clingo_program_builder_begin` until
    /// `clingo_program_builder_end`: its own flag, separate
    /// from the backend's, and a `Cell` for the same reason. The next entry
    /// point or `Drop` ends a session a panic unwound past, because a
    /// session left open silently loses statements.
    pub(super) builder: Cell<Option<NonNull<ffi::clingo_program_builder_t>>>,
    /// Every ground program observer registered on this control
    /// (`raw::observer::ControlHandle::register_observer`), kept alive until
    /// this handle is dropped, after `clingo_control_free` (ownership design:
    /// DESIGN S4, S8, S10): clingo may call any of them up to that point, and
    /// clingo itself never removes a registration, only composes repeated ones
    /// (`libgringo/src/output/output.cc:588-597`).
    ///
    /// Wrapped in `AssertUnwindSafe` so `Control` stays `UnwindSafe`, as it was
    /// before observers existed. An observer may be left inconsistent by a
    /// panic in one of its callbacks, but that panic poisons the control (S3),
    /// and a poisoned control never grounds again, so clingo never calls that
    /// observer again.
    pub(super) observers: AssertUnwindSafe<Vec<Box<dyn ObserverSlots + Send>>>,
    /// Every propagator registered on this control
    /// (`raw::propagate::ControlHandle::register_propagator`), kept alive until
    /// this handle is dropped, after `clingo_control_free`: clingo may call any
    /// of them up to that point, and never removes a registration on its own
    /// (clingo.h:3159-3175).
    ///
    /// `Send + Sync` on the trait object bound, not only `Send` as `observers`
    /// has: `Propagator: Send + Sync` (S11), since two different solver threads
    /// can call `propagate` on the very same registered propagator's `&self` at
    /// once. Wrapped in `AssertUnwindSafe` for the same reason `observers` is:
    /// a panic inside one of a propagator's callbacks is caught by the
    /// trampoline's own `guard` before it ever unwinds through this cell.
    pub(super) propagators: AssertUnwindSafe<Vec<Box<dyn PropagatorSlots + Send + Sync>>>,
    /// The user's [`SolveEventHandler`](crate::SolveEventHandler), while the
    /// search that installed one is open (`raw::solve::
    /// start_search_with_handler`). **Dropped as soon as that search ends, on
    /// every path** (`ControlHandle::take_event_handler`, called from
    /// `close_active` and from `start_search_with_handler`'s own failure path):
    /// a handler may borrow the caller's own locals for a blocking search
    /// (`solve_with_events` takes one), and keeping the box alive past the
    /// search that installed it let safe code write through the handler's stale
    /// borrow after the call that owned it had returned. So nothing here may
    /// depend on `Drop` running "eventually"; it is taken and dropped the
    /// moment the search closes, never deferred to the next search or the
    /// control's own drop. `None` for a plain search, which behaves exactly as
    /// before this part.
    ///
    /// Wrapped in `AssertUnwindSafe` so `Control` stays `UnwindSafe`, as it was
    /// before handlers existed, for the same reason `observers` above is: a
    /// panic inside the handler is caught by the trampoline's own `guard`
    /// before it ever unwinds through this cell, so nothing here is actually
    /// observed mid-unwind.
    pub(super) event_handler: AssertUnwindSafe<RefCell<Option<Box<dyn EventHandlerSlots + Send>>>>,
    /// The handler's own error, moved here by `take_event_handler` at the
    /// same moment the handler itself is dropped, so it stays readable
    /// after that (first writer wins, S8, like every other trampoline
    /// slot). Read by [`ControlHandle::event_handler_error`] alongside the
    /// still-open handler's own slot, so a caller does not need to know
    /// whether the search it is reading has closed yet.
    pub(super) event_handler_error: Slot<Error>,
    /// As `event_handler_error`, for a caught panic.
    pub(super) event_handler_panic: PanicSlot,
    /// What a script callback returned or raised during the last call
    /// that can run one (`add`, `load`, the program builder's `add`), moved
    /// here from the call's thread-local frame (`raw::script::ScriptFrame`).
    /// Read and emptied by the safe layer right after the call.
    pub(super) script_error: Slot<Error>,
    pub(super) script_panic: PanicSlot,
}

/// Why grounding failed: a registered callback (the ground function or a ground
/// program observer) returned an error, clingo failed on its own, or a callback
/// panicked. Only `Clingo` is poisoned by kind (DESIGN S3); `Callback` poisons
/// unconditionally, and `Panic` is resumed by the safe layer after it poisons,
/// so it never reaches here as an `Error` at all.
pub(crate) enum GroundError {
    Callback(Error),
    Clingo(Error),
    /// A caught panic from a ground callback or an observer callback. Kept
    /// separate from `Callback` because it has no `Error` to poison with;
    /// the safe layer poisons directly, then resumes it with
    /// `std::panic::resume_unwind` (S8: resumed on the caller's thread once
    /// clingo has returned, never inside this trampoline call).
    Panic(Box<dyn Any + Send>),
}

impl fmt::Debug for GroundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GroundError::Callback(err) => f.debug_tuple("Callback").field(err).finish(),
            GroundError::Clingo(err) => f.debug_tuple("Clingo").field(err).finish(),
            GroundError::Panic(_) => f.debug_tuple("Panic").field(&"<payload>").finish(),
        }
    }
}

/// The flags of a finished search (clingo.h:2482-2487).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "it mirrors clingo's result bitset, one flag per bit"
)]
pub(crate) struct SolveOutcome {
    pub(crate) satisfiable: bool,
    pub(crate) unsatisfiable: bool,
    pub(crate) exhausted: bool,
    pub(crate) interrupted: bool,
}

impl SolveOutcome {
    pub(crate) fn from_bits(bits: ffi::clingo_solve_result_bitset_t) -> Self {
        SolveOutcome {
            satisfiable: bits & ffi::clingo_solve_result_satisfiable != 0,
            unsatisfiable: bits & ffi::clingo_solve_result_unsatisfiable != 0,
            exhausted: bits & ffi::clingo_solve_result_exhausted != 0,
            interrupted: bits & ffi::clingo_solve_result_interrupted != 0,
        }
    }

    /// clingo's bitset for these flags (clingo.h:2482-2487).
    pub(crate) fn to_bits(self) -> u32 {
        [
            (self.satisfiable, ffi::clingo_solve_result_satisfiable),
            (self.unsatisfiable, ffi::clingo_solve_result_unsatisfiable),
            (self.exhausted, ffi::clingo_solve_result_exhausted),
            (self.interrupted, ffi::clingo_solve_result_interrupted),
        ]
        .into_iter()
        .filter(|(set, _)| *set)
        .fold(0, |bits, (_, bit)| bits | bit)
    }
}

// SAFETY: moving a control to another thread between calls is sound because
// nothing the control depends on is tied to the thread that created it (item
// 3):
// - libclingo, libgringo, clasp and libpotassco keep no thread-local or
//   thread-affine state for a control. A search of their sources for
//   `thread_local`, `__thread`, `this_thread`, `pthread_self` and thread ids
//   finds only clingo's error state (control.cc:113-139) and clasp's per-thread
//   CPU timers (solve_algorithms.cpp:327), which only make the reported CPU
//   time span two threads. clasp holds no mutex across C API calls.
// - The error state is set and read within one call on one thread (`call`,
//   DESIGN S1), never across calls, so a move between calls cannot mix it up.
// - The logger data is the heap address of `capture`, which does not move with
//   the handle, and the user's logger is `Send` (`UserLogger`); the capture's
//   own state is behind mutexes.
// - `solve` records a search that stays open across calls, and `ActiveSolve`'s
//   pointers are only used through `&mut self` or, when closing, through the
//   `Cell` of `&self`; `Cell` keeps the handle `!Sync`, so no two threads use
//   them at once. `sync` is an `Arc` of a `Send + Sync` state.
// - A control moved through every phase (created, added, grounded, solved,
//   models of a yield search read on alternating threads, an async search
//   waited for on one thread and read on another, dropped on another) ran clean
//   under ThreadSanitizer with 1, 4 and 8 solver threads (item 3,
//   `tests/api_threads.rs` repeats it under `cargo xtask sanitize`). The handle
//   stays `!Sync`: clingo's control functions are not reentrant.
unsafe impl Send for ControlHandle {}

// The `Send` impl above relies on the capture and the interrupt state being
// `Send` themselves, as the parts a moved control takes along; these fail to
// compile if a later field makes either of them `!Send`.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<Capture>();
    assert_send::<SolveSync>();
};

impl ControlHandle {
    /// Creates a control from command-line arguments, with the user's logger
    /// if there is one, and clingo's message limit (clingo.h:2995).
    pub(crate) fn new(
        arguments: &[CString],
        user_logger: Option<UserLogger>,
        message_limit: u32,
    ) -> Result<Self, Error> {
        let capture = Box::new(Capture::with_logger(user_logger));
        let argv: Vec<*const c_char> = arguments.iter().map(|a| a.as_ptr()).collect();
        let data = std::ptr::from_ref::<Capture>(&capture)
            .cast_mut()
            .cast::<c_void>();
        let mut control: *mut ffi::clingo_control_t = std::ptr::null_mut();
        // No script may be registered from here on, even if this call fails
        // (`raw::script::freeze`).
        super::script::freeze();
        // SAFETY: `argv` holds `argv.len()` NUL-terminated strings that outlive
        // the call; clingo parses them into its own storage. The logger data
        // points into the heap allocation of `capture`, which the returned
        // handle owns and frees only after clingo_control_free (see Drop), so
        // it outlives every logger call. `control` is a valid out-pointer
        // (clingo.h:2976-2995).
        let result = call(|| unsafe {
            ffi::clingo_control_new(
                argv.as_ptr(),
                argv.len(),
                Some(logger::<Capture>),
                data,
                message_limit,
                &raw mut control,
            )
        });
        capture.resume_panic();
        let messages = capture.take();
        result.map_err(|err| err.with_messages(messages))?;
        let ptr = NonNull::new(control).ok_or_else(|| {
            Error::new(
                ErrorKind::Unknown,
                "clingo reported success but returned no control",
            )
        })?;
        Ok(ControlHandle {
            ptr,
            owned: true,
            arena: None,
            capture,
            solve: Cell::new(None),
            sync: Arc::new(SolveSync::new(ptr)),
            backend: Cell::new(None),
            builder: Cell::new(None),
            observers: AssertUnwindSafe(Vec::new()),
            propagators: AssertUnwindSafe(Vec::new()),
            event_handler: AssertUnwindSafe(RefCell::new(None)),
            event_handler_error: Slot::default(),
            event_handler_panic: PanicSlot::default(),
            script_error: Slot::default(),
            script_panic: PanicSlot::default(),
        })
    }

    /// Wraps the control clingo owns and lends to an application's `main`
    /// (`clingo_main`'s `main` callback). The handle never frees it, and hands
    /// its registered observers and propagators to `arena` when it is dropped.
    ///
    /// The messages of an error from this control carry no captured text:
    /// clingo owns the control and its logger, which reports through the
    /// application's own logger or standard error.
    ///
    /// # Safety
    ///
    /// `ptr` must be the live control clingo passed to the `main` callback,
    /// and it must stay live until this handle is dropped. The handle must be
    /// dropped before the callback returns to clingo.
    pub(crate) unsafe fn borrowed(ptr: NonNull<ffi::clingo_control_t>, arena: Arc<Arena>) -> Self {
        ControlHandle {
            ptr,
            owned: false,
            arena: Some(arena),
            capture: Box::new(Capture::with_logger(None)),
            solve: Cell::new(None),
            sync: Arc::new(SolveSync::new(ptr)),
            backend: Cell::new(None),
            builder: Cell::new(None),
            observers: AssertUnwindSafe(Vec::new()),
            propagators: AssertUnwindSafe(Vec::new()),
            event_handler: AssertUnwindSafe(RefCell::new(None)),
            event_handler_error: Slot::default(),
            event_handler_panic: PanicSlot::default(),
            script_error: Slot::default(),
            script_panic: PanicSlot::default(),
        }
    }

    /// The recorded solve-event handler error, if any (first writer wins,
    /// S8). Checked in the still-open handler's own slot first (a mid-search
    /// caller such as `SolveHandle::next_model`, before the search has
    /// closed and `take_event_handler` has run), then in the promoted slot
    /// `take_event_handler` moved it to if the search has since closed; a
    /// caller does not need to know which case it is in. **A peek, not a
    /// take**: a
    /// later call on the same search, open or closed, sees the same error
    /// again, not clingo's own generic one once the slot has already been
    /// read.
    pub(crate) fn event_handler_error(&self) -> Option<Error> {
        self.event_handler
            .borrow()
            .as_deref()
            .and_then(EventHandlerSlots::peek_error)
            .or_else(|| self.event_handler_error.peek_with(Error::repeatable_copy))
    }

    /// As [`ControlHandle::event_handler_error`], for a caught panic.
    pub(crate) fn event_handler_panic(&self) -> Option<Box<dyn Any + Send>> {
        self.event_handler
            .borrow()
            .as_deref()
            .and_then(EventHandlerSlots::take_panic)
            .or_else(|| self.event_handler_panic.take())
    }

    /// Ends the current search's handler, if any: moves its recorded error
    /// and panic (if either is set) into the slots above, which outlive it,
    /// then drops it, ending any borrow it held. Called
    /// from [`ControlHandle::close_active`] when a search closes normally,
    /// and from `raw::solve::start_search_with_handler`'s own failure path,
    /// for the one case that does not go through `close_active` at all: in
    /// mode 0 (no threads) the whole search, every event included, can run
    /// and finish inside the call that starts it, so a handler that fails
    /// can make that very call return `false` before any search was ever
    /// recorded to close.
    pub(super) fn take_event_handler(&self) {
        let Some(handler) = self.event_handler.borrow_mut().take() else {
            return;
        };
        if let Some(err) = handler.peek_error() {
            self.event_handler_error.store(err);
        }
        if let Some(payload) = handler.take_panic() {
            self.event_handler_panic.store(payload);
        }
        // `handler` is dropped here, ending any borrow it held.
    }

    /// Runs one fallible call and attaches the messages clingo logged during
    /// it.
    pub(super) fn captured(&self, f: impl FnOnce() -> bool) -> Result<(), Error> {
        // Messages logged since the last call (none, unless clingo logs outside
        // calls) belong to no call and are dropped; they already went to `log`.
        drop(self.capture.take());
        let result = call(f);
        self.capture.resume_panic();
        let messages = self.capture.take();
        result.map_err(|err| err.with_messages(messages))
    }

    /// Runs one fallible call on the control, after closing any search still
    /// open, and attaches the messages clingo logged during it.
    pub(super) fn logged(
        &self,
        f: impl FnOnce(*mut ffi::clingo_control_t) -> bool,
    ) -> Result<(), Error> {
        // A leftover search, not what `f` cares about: discard its handler's
        // own failure, never promote.
        self.close_solve(false)?;
        let ptr = self.ptr.as_ptr();
        self.captured(|| f(ptr))
    }

    /// Adds a program block (clingo.h:3044).
    pub(crate) fn add(
        &mut self,
        name: &CStr,
        parameters: &[CString],
        program: &CStr,
    ) -> Result<(), Error> {
        let params: Vec<*const c_char> = parameters.iter().map(|p| p.as_ptr()).collect();
        // A `#script` block in the program runs its `execute` during the call.
        let frame = ScriptFrame::open();
        let result = self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and `&mut
            // self` rules out a concurrent call. The name, the `params.len()`
            // parameter strings and the program are NUL-terminated and outlive
            // the call; clingo copies what it keeps (clingo.h:3044).
            unsafe {
                ffi::clingo_control_add(
                    control,
                    name.as_ptr(),
                    params.as_ptr(),
                    params.len(),
                    program.as_ptr(),
                )
            }
        });
        self.keep_script_failure(frame);
        result
    }

    /// Grounds the given parts, each a name and its parameters, without ground
    /// callbacks (clingo.h:3066).
    ///
    /// A registered observer can still fail or panic during this call;
    /// its error or panic is reported the same way `ground_with` reports a
    /// ground function's, through [`GroundError`], never through clingo's own
    /// report of the failed callback (DESIGN S8).
    pub(crate) fn ground(&mut self, parts: &[(&CStr, &[Symbol])]) -> Result<(), GroundError> {
        // A leftover search, not this grounding call: discard, never
        // promote.
        self.close_solve(false).map_err(GroundError::Clingo)?;
        drop(self.capture.take());
        let frame = ScriptFrame::open();
        let result = call(|| self.ground_parts(parts, None, std::ptr::null_mut()));
        self.keep_script_failure(frame);
        self.capture.resume_panic();
        let messages = self.capture.take();
        self.resolve_ground(result, None, None, messages)
    }

    /// Grounds the given parts and calls `function` for each external function
    /// term (clingo.h:3066, 2914-2956).
    ///
    /// An error the function or a registered observer returned, or
    /// either one's panic, comes from the respective callback context's
    /// slots, never from clingo's report of the failed callback (DESIGN S8).
    /// A panic is returned as [`GroundError::Panic`] rather than resumed
    /// here, so the safe layer can poison the control before resuming it on
    /// the caller's thread. The function's error carries the
    /// messages clingo logged during the call, in front of any it had, as
    /// clingo's own errors do.
    pub(crate) fn ground_with(
        &mut self,
        parts: &[(&CStr, &[Symbol])],
        function: &mut GroundFunction<'_>,
    ) -> Result<(), GroundError> {
        // As `ground` above: discard, never promote.
        self.close_solve(false).map_err(GroundError::Clingo)?;
        let context = GroundContext::new(function);
        let data = std::ptr::from_ref(&context).cast_mut().cast::<c_void>();
        drop(self.capture.take());
        // With a callback, clingo asks it first for every `@` term and never
        // reaches a script, but the frame costs nothing and keeps the reading
        // below the same for both.
        let frame = ScriptFrame::open();
        let result =
            call(|| self.ground_parts(parts, Some(ground_function::<ClingoErrorState>), data));
        self.keep_script_failure(frame);
        self.capture.resume_panic();
        let messages = self.capture.take();
        self.resolve_ground(result, context.panic.take(), context.error.take(), messages)
    }

    /// Combines what clingo itself reported with what a ground function's
    /// context (if any) or any registered observer recorded, in priority
    /// order: a panic first, since it must poison and resume, never be
    /// treated as a plain error; then a returned callback error, which
    /// poisons unconditionally regardless of its kind;
    /// otherwise clingo's own result, poisoned by kind as usual (S3). Only
    /// one of these is ever actually set for a single grounding call, since
    /// clingo stops calling any trampoline as soon as one returns `false`,
    /// but every source is still checked, defensively.
    fn resolve_ground(
        &self,
        result: Result<(), Error>,
        function_panic: Option<Box<dyn Any + Send>>,
        function_error: Option<Error>,
        messages: Vec<crate::error::Message>,
    ) -> Result<(), GroundError> {
        if let Some(payload) = function_panic
            .or_else(|| self.take_observer_panic())
            .or_else(|| self.take_script_panic())
        {
            return Err(GroundError::Panic(payload));
        }
        if let Some(err) = function_error
            .or_else(|| self.take_observer_error())
            .or_else(|| self.take_script_error())
        {
            return Err(GroundError::Callback(err.with_logged_messages(messages)));
        }
        result.map_err(|err| GroundError::Clingo(err.with_messages(messages)))
    }

    /// The one call to `clingo_control_ground`, with or without a callback.
    fn ground_parts(
        &self,
        parts: &[(&CStr, &[Symbol])],
        callback: ffi::clingo_ground_callback_t,
        data: *mut c_void,
    ) -> bool {
        let parts: Vec<ffi::clingo_part_t> = parts
            .iter()
            .map(|(name, params)| ffi::clingo_part_t {
                name: name.as_ptr(),
                // `Symbol` is `repr(transparent)` over `clingo_symbol_t`.
                params: params.as_ptr().cast::<RawSymbol>(),
                size: params.len(),
            })
            .collect();
        // SAFETY: `self.ptr` is the live control this handle owns, and every
        // caller holds `&mut self` and has closed any open search. `parts`
        // holds `parts.len()` parts whose names and parameter arrays are
        // borrowed from the caller and outlive the call. The callback is either
        // none (clingo then reports external functions as undefined) or
        // `ground_function` with `data` pointing to a `GroundContext` that
        // outlives the call (clingo.h:3066, 2914-2956).
        unsafe {
            ffi::clingo_control_ground(
                self.ptr.as_ptr(),
                parts.as_ptr(),
                parts.len(),
                callback,
                data,
            )
        }
    }

    /// The program literal of an atom, or `None` if the atom does not occur in
    /// the grounding (clingo.h:3315, 587, 682, 643).
    pub(crate) fn atom_literal(&mut self, atom: RawSymbol) -> Result<Option<i32>, Error> {
        self.find_atom(atom, false)
    }

    /// The program literal of an external atom, or `None` if the atom does not
    /// occur in the grounding or is not an external (clingo.h:630).
    pub(crate) fn external_literal(&mut self, atom: RawSymbol) -> Result<Option<i32>, Error> {
        self.find_atom(atom, true)
    }

    /// Looks up an atom in the symbolic atoms and returns its program literal,
    /// or `None` if it does not occur or, when `external` is set, is not an
    /// external.
    fn find_atom(&mut self, atom: RawSymbol, external: bool) -> Result<Option<i32>, Error> {
        // `symbolic_atoms` closes any open search first.
        let found = self.symbolic_atoms()?.find(atom)?;
        Ok(found
            .filter(|data| data.external || !external)
            .map(|data| data.literal))
    }

    /// Assigns a truth value to the external atom with the given program
    /// literal; clingo ignores a literal that is not an external
    /// (clingo.h:3123).
    pub(crate) fn assign_external(&mut self, literal: i32, value: TruthValue) -> Result<(), Error> {
        let value = c_int_of(match value {
            TruthValue::True => ffi::clingo_truth_value_true,
            TruthValue::False => ffi::clingo_truth_value_false,
            TruthValue::Free => ffi::clingo_truth_value_free,
        });
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and no
            // search is open (`logged` closed it). Any literal is accepted
            // (control.cc:2039-2053).
            unsafe { ffi::clingo_control_assign_external(control, literal, value) }
        })
    }

    /// Releases the external atom with the given program literal, making it
    /// permanently false; clingo ignores a literal that is not an external
    /// (clingo.h:3137).
    pub(crate) fn release_external(&mut self, literal: i32) -> Result<(), Error> {
        self.logged(|control| {
            // SAFETY: as in `assign_external` (control.cc:2055-2058).
            unsafe { ffi::clingo_control_release_external(control, literal) }
        })
    }

    /// Reads a file as ordinary program text (clingo.h:3013).
    pub(crate) fn load(&mut self, file: &CStr) -> Result<(), Error> {
        // A `#script` block in the file runs its `execute` during the call.
        let frame = ScriptFrame::open();
        let result = self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and no
            // search is open (`logged` closed it). `file` is NUL-terminated
            // and outlives the call; clingo opens and reads the named file
            // itself (clingo.h:3013).
            unsafe { ffi::clingo_control_load(control, file.as_ptr()) }
        });
        self.keep_script_failure(frame);
        result
    }

    /// Loads ground programs in aspif format, merged into one if more than one
    /// file is given (clingo.h:3027).
    pub(crate) fn load_aspif(&mut self, files: &[CString]) -> Result<(), Error> {
        let mut pointers: Vec<*const c_char> = files.iter().map(|f| f.as_ptr()).collect();
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and no
            // search is open (`logged` closed it). `pointers` holds
            // `pointers.len()` NUL-terminated strings that outlive the call;
            // clingo only reads through the outer pointer, which bindgen
            // types as mutable only because the header does not mark it
            // `char const *const *` (clingo.h:3027).
            unsafe {
                ffi::clingo_control_load_aspif(control, pointers.as_mut_ptr(), pointers.len())
            }
        })
    }

    /// Cleans up the grounding using the solver's current assignment
    /// (clingo.h:3110).
    pub(crate) fn cleanup(&mut self) -> Result<(), Error> {
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and no
            // search is open (`logged` closed it) (clingo.h:3110).
            unsafe { ffi::clingo_control_cleanup(control) }
        })
    }

    /// Enables or disables automatic cleanup after solving (clingo.h:3271).
    pub(crate) fn set_enable_cleanup(&mut self, enable: bool) -> Result<(), Error> {
        self.logged(|control| {
            // SAFETY: as in `cleanup`; the call only writes a flag
            // (clingo.h:3271).
            unsafe { ffi::clingo_control_set_enable_cleanup(control, enable) }
        })
    }

    /// Whether automatic cleanup after solving is enabled (clingo.h:3281).
    pub(crate) fn enable_cleanup(&self) -> bool {
        // SAFETY: `self.ptr` is the live control this handle owns; the call
        // only reads a flag on the control object and cannot fail, although
        // the header declares a non-const pointer for this plain getter
        // (clingo.h:3281, control.cc:2189).
        unsafe { ffi::clingo_control_get_enable_cleanup(self.ptr.as_ptr()) }
    }

    /// Removes every minimize constraint from the program (clingo.h:3145).
    pub(crate) fn remove_minimize(&mut self) -> Result<(), Error> {
        self.logged(|control| {
            // SAFETY: as in `cleanup` (clingo.h:3145).
            unsafe { ffi::clingo_control_remove_minimize(control) }
        })
    }

    /// Replaces or extends the set of projection atoms (clingo.h:3157).
    pub(crate) fn update_project(
        &mut self,
        atoms: &[ffi::clingo_atom_t],
        append: bool,
    ) -> Result<(), Error> {
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and no
            // search is open (`logged` closed it). `atoms` holds
            // `atoms.len()` ids and outlives the call; clingo copies what it
            // keeps (clingo.h:3157).
            unsafe {
                ffi::clingo_control_update_project(control, atoms.as_ptr(), atoms.len(), append)
            }
        })
    }

    /// Whether the program's internal representation is already known to be
    /// conflicting (clingo.h:3186). A `false` never proves satisfiability:
    /// conflicts first have to be detected.
    pub(crate) fn is_conflicting(&self) -> bool {
        // SAFETY: `self.ptr` is the live control this handle owns; the call
        // only reads a flag and cannot fail (clingo.h:3186, control.cc:2201).
        unsafe { ffi::clingo_control_is_conflicting(self.ptr.as_ptr()) }
    }

    /// Enables or disables the enumeration assumption (clingo.h:3249).
    pub(crate) fn set_enable_enumeration_assumption(&mut self, enable: bool) -> Result<(), Error> {
        self.logged(|control| {
            // SAFETY: as in `set_enable_cleanup` (clingo.h:3249).
            unsafe { ffi::clingo_control_set_enable_enumeration_assumption(control, enable) }
        })
    }

    /// Whether the enumeration assumption is enabled (clingo.h:3260).
    pub(crate) fn enable_enumeration_assumption(&self) -> bool {
        // SAFETY: as in `enable_cleanup` (clingo.h:3260, control.cc:2175).
        unsafe { ffi::clingo_control_get_enable_enumeration_assumption(self.ptr.as_ptr()) }
    }

    /// Whether a `#const` definition exists for `name` (clingo.h:3303).
    pub(crate) fn has_const(&self, name: &CStr) -> Result<bool, Error> {
        let mut exists = false;
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and no
            // search is open (`logged` closed it). `name` is NUL-terminated
            // and outlives the call, and `exists` is a valid out-pointer
            // (clingo.h:3303).
            unsafe { ffi::clingo_control_has_const(control, name.as_ptr(), &raw mut exists) }
        })?;
        Ok(exists)
    }

    /// The symbol of a `#const` definition (clingo.h:3288). Call
    /// [`ControlHandle::has_const`] first: on an undefined name clingo 5.8.2
    /// returns the name itself as a 0-arity function symbol rather than
    /// failing.
    pub(crate) fn get_const(&self, name: &CStr) -> Result<RawSymbol, Error> {
        let mut symbol = 0;
        self.logged(|control| {
            // SAFETY: as in `has_const`; `symbol` is a valid out-pointer
            // (clingo.h:3288).
            unsafe { ffi::clingo_control_get_const(control, name.as_ptr(), &raw mut symbol) }
        })?;
        Ok(symbol)
    }
}

impl Drop for ControlHandle {
    fn drop(&mut self) {
        // A search, a backend or a program builder left open by a forgotten
        // handle, or one a panic unwound past, is closed before the control is
        // freed (S4).
        // Drop cannot report a failure, and the control is gone afterwards
        // either way. A handler's own recorded error or panic is discarded
        // with it, never promoted: there is no caller left to
        // blame it on.
        drop(self.close_solve(false));
        drop(self.close_backend());
        drop(self.close_program_builder());
        // An interrupt in progress holds the lock, so this waits for it, and no
        // later one reaches the control (S13).
        self.sync.detach();
        if !self.owned {
            // The control belongs to clingo, which frees it after `main` has
            // returned and may still call an observer or a propagator until
            // then: hand them to the run's arena instead of dropping them.
            if let Some(arena) = self.arena.take() {
                arena.absorb(
                    std::mem::take(&mut self.observers.0),
                    std::mem::take(&mut self.propagators.0),
                );
            }
            return;
        }
        // SAFETY: `ptr` came from clingo_control_new and is freed exactly once,
        // here (`owned` is true). No solve handle is open: every search is
        // recorded until it is closed, and the recorded one was closed just
        // above (clingo.h:3001). No interrupt can reach it any more
        // (`detach`). The capture box and the event handler's data are dropped
        // after this call, so no callback sees a dangling pointer.
        unsafe { ffi::clingo_control_free(self.ptr.as_ptr()) };
    }
}

/// The registered state of a borrowed control that clingo may still call
/// after the handle is gone.
///
/// A borrowed handle is dropped by the `main` trampoline when the callback
/// ends, but clingo tears its control down only after that, and until then
/// it can call a registered observer or propagator. The handle therefore
/// moves them here when it is dropped, and the run drops them after
/// `clingo_main` has returned (`Arena::drain`, and `Drop` on any other path).
/// Solve-event handlers need no such treatment: the handle's `Drop` closes
/// the search, and with it the handler, before returning to clingo.
#[derive(Default)]
pub(crate) struct Arena {
    observers: Mutex<Vec<Box<dyn ObserverSlots + Send>>>,
    propagators: Mutex<Vec<Box<dyn PropagatorSlots + Send + Sync>>>,
}

impl Arena {
    fn absorb(
        &self,
        observers: Vec<Box<dyn ObserverSlots + Send>>,
        propagators: Vec<Box<dyn PropagatorSlots + Send + Sync>>,
    ) {
        lock(&self.observers).extend(observers);
        lock(&self.propagators).extend(propagators);
    }

    /// Drops everything kept, once.
    pub(crate) fn drain(&self) {
        let observers = std::mem::take(&mut *lock(&self.observers));
        let propagators = std::mem::take(&mut *lock(&self.propagators));
        drop(observers);
        drop(propagators);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        lock(&self.observers).len() + lock(&self.propagators).len()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // Nothing runs user code under this lock, so a poisoned one is intact.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl fmt::Debug for ControlHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlHandle")
            .field("ptr", &self.ptr)
            .field("solving", &self.solve_is_active())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::raw::propagate::PropagatorSlots;

    /// Counts its own drops; stands in for a registered observer or propagator.
    struct Kept(Arc<AtomicUsize>);

    impl Drop for Kept {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl ObserverSlots for Kept {
        fn take_error(&self) -> Option<Error> {
            None
        }
        fn take_panic(&self) -> Option<Box<dyn Any + Send>> {
            None
        }
    }

    impl PropagatorSlots for Kept {
        fn take_error(&self) -> Option<Error> {
            None
        }
        fn take_panic(&self) -> Option<(bool, Box<dyn Any + Send>)> {
            None
        }
    }

    /// A borrowed handle over a pointer that is never dereferenced: any call
    /// into clingo would be undefined behaviour, so these tests (and Miri)
    /// show that dropping the handle makes none.
    fn fake_borrowed(arena: &Arc<Arena>) -> ControlHandle {
        // SAFETY: the pointer is never used for a call: the handle records no
        // search, backend or program builder, and is only dropped.
        unsafe { ControlHandle::borrowed(NonNull::dangling(), Arc::clone(arena)) }
    }

    #[test]
    fn dropping_a_borrowed_handle_never_frees_the_control() {
        let arena = Arc::new(Arena::default());
        let handle = fake_borrowed(&arena);
        assert!(!handle.owned);
        // `clingo_control_free` on a dangling pointer would crash here.
        drop(handle);
        assert_eq!(arena.len(), 0);
    }

    #[test]
    fn registered_state_outlives_the_borrowed_handle_and_is_dropped_once_at_drain() {
        let drops = Arc::new(AtomicUsize::new(0));
        let arena = Arc::new(Arena::default());
        let mut handle = fake_borrowed(&arena);
        handle.observers.push(Box::new(Kept(Arc::clone(&drops))));
        handle.propagators.push(Box::new(Kept(Arc::clone(&drops))));
        drop(handle);
        assert_eq!(
            drops.load(Ordering::SeqCst),
            0,
            "the handle's drop keeps them"
        );
        assert_eq!(arena.len(), 2);
        arena.drain();
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        arena.drain();
        drop(arena);
        assert_eq!(
            drops.load(Ordering::SeqCst),
            2,
            "each is dropped exactly once"
        );
    }

    #[test]
    fn state_left_in_the_arena_is_dropped_with_it_on_any_other_path() {
        let drops = Arc::new(AtomicUsize::new(0));
        let arena = Arc::new(Arena::default());
        let mut handle = fake_borrowed(&arena);
        handle.observers.push(Box::new(Kept(Arc::clone(&drops))));
        drop(handle);
        drop(arena);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

//! Wrappers for searches: starting one in yield or async mode, stepping
//! through its models, waiting for it, and closing it (clingo.h:2566-2643,
//! 3089; DESIGN S4, S6, S7, S13, S14).
//!
//! The open search lives in [`ControlHandle`], not in the safe `SolveHandle`,
//! so that closing it never depends on a guard's `Drop` (S4). The model clingo
//! lent last is recorded next to it, and it is forgotten before every call that
//! moves the search on, so a lent `&Model` can only be taken while it is valid.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clingox_sys as ffi;

use super::control::{ControlHandle, SolveOutcome};
use super::interrupt::{SolveSync, solve_event};
use super::{call, raw_slice};
use crate::Model;
use crate::error::{Error, ErrorKind};

#[cfg(test)]
thread_local! {
    /// A panic for the logger to have raised while the next search on this
    /// thread closes. clingo logs nothing while closing, so tests inject it.
    static PANIC_WHILE_CLOSING: std::cell::RefCell<Option<Box<dyn std::any::Any + Send>>> =
        const { std::cell::RefCell::new(None) };
}

/// Makes the logger panic with `payload` while the next search on this
/// thread closes (tests only).
#[cfg(test)]
pub(crate) fn panic_while_closing(payload: Box<dyn std::any::Any + Send>) {
    PANIC_WHILE_CLOSING.set(Some(payload));
}

/// Combines what a search returned with the result of closing it (DESIGN
/// S3, S7). A failed close poisons whatever its kind, because clingo's
/// state after it is unknown: alone, its error is returned and poisons;
/// after a failed `got`, `got`'s error is returned and poisons with the
/// close error as its cause.
pub(crate) fn settle<T>(got: Result<T, Error>, closed: Result<(), Error>) -> Result<T, Error> {
    match (got, closed) {
        (got, Ok(())) => got,
        (Ok(_), Err(close)) => Err(close.poisoning()),
        (Err(got), Err(close)) => Err(got.poisoned_by(&close)),
    }
}

/// A search started by [`ControlHandle::start_search`] and not closed yet.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ActiveSolve {
    handle: NonNull<ffi::clingo_solve_handle_t>,
    /// The model clingo returned last, until the search moves on.
    model: Option<NonNull<ffi::clingo_model_t>>,
}

impl ControlHandle {
    /// Runs `f` as one call of the API: messages logged meanwhile are attached
    /// to its error, and a panic from the logger or an event handler resumes
    /// after `f` has returned, so `f` can close what it opened first (S8).
    pub(crate) fn logged_call<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        drop(self.capture.take());
        let result = f(self);
        self.capture.resume_panic();
        self.sync.resume_panic();
        let messages = self.capture.take();
        result.map_err(|err| err.with_messages(messages))
    }

    /// Starts a search in `mode` under program-literal assumptions and records
    /// it (clingo.h:3089). A search still open is closed first.
    ///
    /// The search's event handler is `solve_event` with the control's
    /// `SolveSync`, whose phase is `Starting` for the duration of the call and
    /// `Running` after it, unless the search has already finished; this is
    /// what keeps interrupts inside the search (see `raw::interrupt`). In
    /// yield mode no model is computed yet; in async mode the search runs on
    /// clasp's own thread; in mode 0 it runs to its end inside the call.
    pub(crate) fn start_search(
        &mut self,
        mode: ffi::clingo_solve_mode_bitset_t,
        assumptions: &[i32],
    ) -> Result<(), Error> {
        let sync = Arc::as_ptr(&self.sync).cast_mut().cast::<c_void>();
        // SAFETY: the data pointer is `self.sync`'s own address, which this
        // handle keeps alive until after the control is freed, and
        // `SolveSync` is `Sync` for calls from clasp's threads (matching
        // `start_search_raw`'s own contract).
        unsafe { self.start_search_raw(mode, assumptions, Some(solve_event::<SolveSync>), sync) }
    }

    /// As [`ControlHandle::start_search`], but with the event handler and its
    /// data given explicitly rather than fixed to `solve_event::<SolveSync>`:
    /// every search still installs exactly one `notify`/`data` pair
    /// (`clingo_control_solve` accepts no more), so a search that also wants a
    /// user's `SolveEventHandler` must compose the two into one trampoline
    /// rather than register a second handler. `start_search` itself is this
    /// function with `solve_event::<SolveSync>` and `self.sync`'s address
    /// plugged in, so plain searches are unaffected by anything below.
    ///
    /// # Safety
    ///
    /// `data` must be a pointer `notify` can be called with, safely, for as
    /// long as the search stays open: from this call until the search this
    /// starts is closed (`ControlHandle::close_active`), on every path, because
    /// clingo may call `notify` at any point in between, not only during this
    /// call (clingo.h:3089-3092).
    pub(crate) unsafe fn start_search_raw(
        &mut self,
        mode: ffi::clingo_solve_mode_bitset_t,
        assumptions: &[i32],
        notify: ffi::clingo_solve_event_callback_t,
        data: *mut c_void,
    ) -> Result<(), Error> {
        // A leftover search this call cleans up first, not the one this call
        // is starting: discard, never promote, its handler's own failure.
        self.close_solve(false)?;
        let control = self.ptr.as_ptr();
        let mut handle: *mut ffi::clingo_solve_handle_t = std::ptr::null_mut();
        // In mode 0 the whole search runs inside the call below.
        let inside = mode == 0;
        self.sync.starting(inside);
        // SAFETY: `control` is the live control this handle owns, `&mut self`
        // rules out a concurrent call, and no search is open (closed above).
        // The `assumptions.len()` literals outlive the call, which copies
        // them; an empty slice's dangling pointer is not read. `data` is
        // valid for `notify` to be called with for the whole search, per this
        // function's own contract, which every caller of it upholds.
        // `handle` is a valid out-pointer (clingo.h:3089).
        let started = call(|| unsafe {
            ffi::clingo_control_solve(
                control,
                mode,
                assumptions.as_ptr(),
                assumptions.len(),
                notify,
                data,
                &raw mut handle,
            )
        })
        .and_then(|()| {
            NonNull::new(handle).ok_or_else(|| {
                Error::new(
                    ErrorKind::Unknown,
                    "clingo started a search but returned no solve handle",
                )
            })
        });
        match started {
            Ok(handle) => {
                self.solve.set(Some(ActiveSolve {
                    handle,
                    model: None,
                }));
                if inside {
                    // The search has already ended.
                    self.sync.finished();
                } else {
                    self.sync.started();
                }
                Ok(())
            }
            Err(err) => {
                self.sync.finished();
                Err(err)
            }
        }
    }

    /// Starts a search in yield mode, for `SolveHandle` (DESIGN S6).
    pub(crate) fn start_yield(&mut self, assumptions: &[i32]) -> Result<(), Error> {
        self.logged_call(|handle| handle.start_search(ffi::clingo_solve_mode_yield, assumptions))
    }

    /// Starts a search on clasp's own thread, for `AsyncSolveHandle`. It needs
    /// a build of clasp with threads; clingo reports the mode as a logic error
    /// otherwise.
    pub(crate) fn start_async(&mut self, assumptions: &[i32]) -> Result<(), Error> {
        self.logged_call(|handle| handle.start_search(ffi::clingo_solve_mode_async, assumptions))
    }

    /// Starts a search in `mode` with a composed event trampoline that also
    /// runs the user's `handler`. Unlike [`ControlHandle:: start_search`],
    /// `handler` need not be `'static`: only `raw::solve::solve_with_handler`
    /// (behind `Control::solve_with_events`) ever passes one that is not, and
    /// that function returns to its own caller only after this search has
    /// closed and `handler` has been dropped with it, on every path (see
    /// `ControlHandle::take_event_handler`, called both by
    /// [`ControlHandle::close_active`] and, below, when this call itself
    /// fails). An earlier version of this function stored the handler without
    /// ever dropping it before the search that installed it closed, which let
    /// safe code observe a stale borrow once the call that owned it had already
    /// returned; nothing here may again assume the box's own `Drop` runs
    /// "eventually" instead of being run deliberately at that one point.
    ///
    /// **Precondition: no search may already be open on `self`**. The new
    /// handler's box is stored below, before `start_search_raw` closes any
    /// leftover search internally (`start_search_raw`'s own
    /// `self.close_solve(false)`); if a search were already open with its own
    /// handler, storing the new box would drop the old one at once, while
    /// clingo could still call into it through the data pointer a still-open
    /// search captured for it: a use-after-free. Every current caller
    /// (`solve_with_handler`, `start_yield_with_handler`,
    /// `start_async_with_handler`) is reached only through
    /// `Control::guarded`/`Control::guarded_with_events`, which already run
    /// `finish_search()` (closing any leftover search) before calling here, so
    /// this is unreachable today; the `debug_assert!` below makes a future
    /// caller that breaks this precondition fail in tests instead of silently
    /// reintroducing the bug.
    fn start_search_with_handler<'h, H>(
        &mut self,
        mode: ffi::clingo_solve_mode_bitset_t,
        assumptions: &[i32],
        handler: H,
    ) -> Result<(), Error>
    where
        H: crate::solve_events::SolveEventHandler + Send + 'h,
    {
        debug_assert!(
            self.solve.get().is_none(),
            "start_search_with_handler must not be called with a search already open"
        );
        let sync = Arc::clone(&self.sync);
        let boxed: Box<super::events::EventData<H>> =
            Box::new(super::events::EventData::new(sync, handler));
        let data = std::ptr::from_ref(boxed.as_ref())
            .cast_mut()
            .cast::<c_void>();
        // Stored before the call below, not after: in mode 0 the whole
        // search, every event included, runs and finishes inside that one
        // call. Since U26 no handler
        // failure on any event ever makes the trampoline return `false` to
        // clingo any more (`raw::events::stop` always takes the
        // `*goon = false` path), so this call itself is not currently known
        // to fail just because a handler did; the box is still stored first
        // as a defensive measure, so that if a handler failure ever did make
        // this call return an error again (a future clingo, or a path this
        // audit missed), the recorded error or panic would not be dropped,
        // unread, right here, with the caller seeing only clingo's own
        // generic failure instead. `data`'s address does not move when
        // `boxed` does (a `Box`'s heap allocation is stable across moves),
        // so this is safe to do before the call that needs `data` to stay
        // valid.
        //
        // SAFETY: widening the trait object's lifetime bound from `'h` to
        // `'static` is sound because every path that can reach this line
        // with an `H` that is not already `'static` is
        // `raw::solve::solve_with_handler`, called only from
        // `Control::solve_with_events`; that function's own body runs
        // `close_active` (success) or, below, `take_event_handler`
        // (failure) before it returns, which drops this exact box, ending
        // `'h`, strictly before `solve_with_handler` returns to
        // `solve_with_events`, and so strictly before the borrow that
        // produced `handler` could be used again by that caller's own
        // caller. The yielding and asynchronous entry points never reach
        // this concern at all: their own public signatures already require
        // `H: 'static`, so widening the bound there changes
        // nothing observable. `Box<dyn Trait + 'a>` and `Box<dyn Trait +
        // 'b>` share the same layout regardless of the lifetime annotation,
        // which is erased at run time, so the transmute itself is a no-op
        // on the bits it touches.
        let erased: Box<dyn super::events::EventHandlerSlots + Send + 'static> = unsafe {
            std::mem::transmute::<
                Box<dyn super::events::EventHandlerSlots + Send + 'h>,
                Box<dyn super::events::EventHandlerSlots + Send + 'static>,
            >(boxed)
        };
        // A fresh search starts with a clean slate: since
        // `ControlHandle::event_handler_error`/`event_handler_panic` are a peek
        // rather than a take (so a later call on the *same* search keeps
        // seeing the same value), an earlier, already-closed search's own
        // promoted error or panic would otherwise stay in these two slots
        // forever, unless some caller happened to read it in between,
        // leaking into every later, unrelated search too. Discarded, not
        // read: nothing here needs the old value, and this search has not
        // failed yet.
        self.event_handler_error.take();
        self.event_handler_panic.take();
        *self.event_handler.borrow_mut() = Some(erased);
        // SAFETY: `data` is the address of the box just stored above, which
        // is not freed before this call returns (S4, S7): only
        // `close_active` and the failure path just below ever drop it, and
        // neither can run concurrently with the FFI call itself.
        let started = unsafe {
            self.start_search_raw(
                mode,
                assumptions,
                Some(super::events::solve_event_with_handler::<H>),
                data,
            )
        };
        if started.is_err() {
            // In mode 0 this call can itself fail because the handler
            // failed during it, with no search ever recorded for
            // `close_active` to close later: drop the handler here instead,
            // moving whatever it recorded into the slots that outlive it.
            self.take_event_handler();
        }
        started
    }

    /// Starts a search in yield mode with a composed event trampoline that
    /// also runs the user's `SolveEventHandler`.
    pub(crate) fn start_yield_with_handler<'h, H>(
        &mut self,
        assumptions: &[i32],
        handler: H,
    ) -> Result<(), Error>
    where
        H: crate::solve_events::SolveEventHandler + Send + 'h,
    {
        self.logged_call(|handle| {
            handle.start_search_with_handler(ffi::clingo_solve_mode_yield, assumptions, handler)
        })
    }

    /// As [`ControlHandle::start_yield_with_handler`], on clasp's own
    /// thread. `handler` must be `'static` here: it is stored for the
    /// search's whole duration and its callbacks run on a thread the caller
    /// does not control (DESIGN S10).
    pub(crate) fn start_async_with_handler<H>(
        &mut self,
        assumptions: &[i32],
        handler: H,
    ) -> Result<(), Error>
    where
        H: crate::solve_events::SolveEventHandler + Send + 'static,
    {
        self.logged_call(|handle| {
            handle.start_search_with_handler(ffi::clingo_solve_mode_async, assumptions, handler)
        })
    }

    /// Solves under program-literal assumptions with a composed event
    /// trampoline, and waits for the result (mirrors [`ControlHandle::solve`]).
    pub(crate) fn solve_with_handler<'h, H>(
        &mut self,
        assumptions: &[i32],
        handler: H,
    ) -> Result<SolveOutcome, Error>
    where
        H: crate::solve_events::SolveEventHandler + Send + 'h,
    {
        let mode = self.blocking_mode();
        self.logged_call(|handle| {
            handle.start_search_with_handler(mode, assumptions, handler)?;
            let outcome = handle.solve_get();
            // This call's own search, closed deliberately, right after it
            // ran: the handler's own error or panic, if any, is promoted so
            // `Control::solve_with_events` can report it.
            settle(outcome, handle.close_active(true))
        })
    }

    /// The mode of a blocking solve: clingo's mode 0 when nothing can
    /// interrupt the search, async mode otherwise.
    ///
    /// Mode 0 runs the whole search inside `clingo_control_solve` on the
    /// calling thread, which saves the thread clasp starts for every async
    /// search. It is used when the build has no threads (async mode does not
    /// exist there), and when the build has threads and
    /// `Arc::get_mut(&mut self.sync)` succeeds, that is, when no other
    /// `Arc<SolveSync>` exists: no `InterruptHandle`, no timeout thread
    /// (which holds a handle), no printer slot of an application, and no open
    /// search (an event handler's box holds one).
    ///
    /// That is sound because a new holder of the shared state can only be
    /// made by [`ControlHandle::interrupt_state`], which takes `&self`, and
    /// the solve holds `&mut self` for its whole length: none can appear
    /// while a mode-0 search runs, and the borrow checker enforces it. The
    /// phase `Inside`, in which `SolveSync::interrupt` does nothing, is
    /// therefore only reached when nobody can call it, so the invariant of
    /// DESIGN S13 is untouched. The test fails safe: a holder it cannot see
    /// makes `get_mut` fail, and the search stays async. A weak reference
    /// would not be seen, and none exists. An interrupt queued before the
    /// solve is still drained by clasp in every mode
    /// (`clasp_facade.cpp:322`).
    ///
    /// This must run before anything clones `self.sync` for the search
    /// (`start_search_with_handler` does), and it is used only by the
    /// blocking entry points: [`ControlHandle::solve_timed`], the yield paths
    /// and the async entry points choose their own mode.
    fn blocking_mode(&mut self) -> ffi::clingo_solve_mode_bitset_t {
        if !super::HAS_THREADS || Arc::get_mut(&mut self.sync).is_some() {
            0
        } else {
            ffi::clingo_solve_mode_async
        }
    }

    /// Solves under program-literal assumptions and waits for the result
    /// (DESIGN S7).
    ///
    /// The mode is [`ControlHandle::blocking_mode`]'s: clingo's blocking mode
    /// 0 whenever nothing can interrupt the search, so the whole search runs
    /// inside `clingo_control_solve` on this thread, and async mode otherwise,
    /// where clasp runs the search on a thread of its own and this thread
    /// waits for it in `clingo_solve_handle_get`. The search itself runs as
    /// fast in either, since both run clasp's `solve` to its end in one go
    /// (65 536 models: 17.2 ms either way). The difference is a fixed cost per
    /// call: clasp starts a new thread for every async search
    /// (`clasp_facade.cpp:378`), 28 to 34 us of a 33 to 40 us trivial
    /// multi-shot solve, which is negligible for a real search and
    /// significant for many tiny solves on one control
    /// (`docs/dev/BENCHMARKS.md`).
    ///
    /// Async mode is kept for a search that something can interrupt because it
    /// separates the start of the search from its run, which the interrupt
    /// design needs (`raw::interrupt`, DESIGN S13 and S14): an
    /// `InterruptHandle` or a timeout can reach the search only once the
    /// strategy is known to be running, never before or after. A yield search
    /// would be driven model by model from this thread instead, up to 21 times
    /// slower with several solver threads, and clasp would not give the
    /// warnings of a blocking solve.
    ///
    /// The search is closed before returning, so none can be left over (S4),
    /// and a panic from the logger resumes only after that.
    pub(crate) fn solve(&mut self, assumptions: &[i32]) -> Result<SolveOutcome, Error> {
        let mode = self.blocking_mode();
        self.logged_call(|handle| {
            handle.start_search(mode, assumptions)?;
            let outcome = handle.solve_get();
            // No handler is ever set on this path; `true` vs `false` makes
            // no difference here, but this is still this call's own search,
            // closed deliberately, not a leftover.
            settle(outcome, handle.close_active(true))
        })
    }

    /// Solves in async mode and waits at most `budget` for the result; a
    /// search still running then is cancelled, so its result is interrupted
    /// (DESIGN S14). The search is closed before returning. Cancelling never
    /// queues an interrupt for a later search (`clasp_facade.cpp:476`).
    pub(crate) fn solve_timed(
        &mut self,
        assumptions: &[i32],
        budget: Duration,
    ) -> Result<SolveOutcome, Error> {
        self.logged_call(|handle| {
            handle.start_search(ffi::clingo_solve_mode_async, assumptions)?;
            let outcome = if handle.solve_wait(budget) {
                handle.solve_get()
            } else {
                handle.solve_cancel().and_then(|()| handle.solve_get())
            };
            // As `solve` above: no handler on this path, but still this
            // call's own search.
            settle(outcome, handle.close_active(true))
        })
    }

    /// Waits up to `timeout` for the open async search to finish and says
    /// whether it has (clingo.h:2587). Without an open search it is `true`.
    ///
    /// clasp's timed wait may return early, and it counts whole milliseconds,
    /// so this waits in steps until the deadline. `Duration::ZERO` only polls.
    /// It must only be called on an async search: clasp aborts the process on
    /// a timed wait for a yield search (`Timed wait not supported!`, which
    /// `clingo_solve_handle_wait` turns into `std::terminate`).
    pub(crate) fn solve_wait(&mut self, timeout: Duration) -> bool {
        /// clasp converts the timeout to whole milliseconds as an integer; a
        /// bounded step keeps that conversion in range for any `Duration`.
        const LONGEST_STEP: Duration = Duration::from_secs(3600);
        let Some(active) = self.solve.get() else {
            return true;
        };
        let deadline = Instant::now().checked_add(timeout);
        loop {
            let remaining = deadline.map_or(LONGEST_STEP, |d| {
                d.saturating_duration_since(Instant::now())
            });
            // Whole milliseconds, rounded up so a short remainder still waits,
            // plus half a millisecond so that clasp's truncation of the
            // floating-point value gives the intended count.
            let millis = remaining.min(LONGEST_STEP).as_micros().div_ceil(1000);
            #[expect(
                clippy::cast_precision_loss,
                reason = "at most 3.6 million milliseconds, which an f64 holds exactly"
            )]
            let seconds = if millis == 0 {
                0.0
            } else {
                (millis as f64 + 0.5) / 1000.0
            };
            let mut finished = false;
            // SAFETY: `active.handle` is the open async search of this control
            // (the only kind a timeout or `AsyncSolveHandle` starts), and
            // `finished` is a valid out-pointer. A timed wait on an async
            // search does not throw (clasp_facade.cpp:385-403) (clingo.h:2587).
            unsafe {
                ffi::clingo_solve_handle_wait(active.handle.as_ptr(), seconds, &raw mut finished);
            }
            if finished {
                return true;
            }
            if remaining.is_zero() {
                return false;
            }
        }
    }

    /// Whether a search is open.
    pub(crate) fn solve_is_active(&self) -> bool {
        self.solve.get().is_some()
    }

    /// The shared state for `InterruptHandle` (DESIGN S13).
    pub(crate) fn interrupt_state(&self) -> Arc<SolveSync> {
        Arc::clone(&self.sync)
    }

    /// Closes the open search, if any, without touching the captured
    /// messages (clingo.h:2643). Closing cancels it.
    ///
    /// The record is taken out before the call, so the handle is closed at most
    /// once whatever the outcome. clingo delivers the search's finish event
    /// before the close returns, which ends the interrupt phase; it is set
    /// again here so no path can leave it running.
    ///
    /// `promote_handler` decides what happens to the handler's own recorded
    /// error or panic, if either is set:
    /// with `true`, [`ControlHandle::take_event_handler`] moves it to the
    /// control-level slots that outlive the handler, exactly as before this
    /// review, for `SolveHandle::close`/`AsyncSolveHandle::close`
    /// (`Control::get_and_close`) and the deliberate, in-place close a
    /// blocking search gives itself right after it runs (`ControlHandle::
    /// solve`, `solve_timed`, `solve_with_handler`, below): all three have an
    /// explicit caller to report to. With `false`, it is dropped along with
    /// the handler and never promoted: every other path that reaches this
    /// call closes a search that is not what the caller who triggered the
    /// close is asking about at all, but a leftover this call's own
    /// contract requires it to clean up first (DESIGN S4): a forgotten
    /// handle's search, closed by the next entry point or by
    /// `SolveHandle`'s/`AsyncSolveHandle`'s own `Drop`. `Drop` cannot report
    /// a failure, and surfacing it later would blame whatever unrelated call
    /// happens to close the search or read the slots next (the stale-state
    /// bug fixed here; see `SolveHandle`'s and `AsyncSolveHandle`'s own
    /// rustdoc).
    pub(super) fn close_active(&self, promote_handler: bool) -> Result<(), Error> {
        let Some(active) = self.solve.take() else {
            return Ok(());
        };
        // SAFETY: `active.handle` came from clingo_control_solve on this
        // control and was never closed: it is closed only here, after being
        // taken out of the record, which is its only copy. The lent model, if
        // any, dies with it; no `&Model` can be alive, since lending one
        // borrows `self` mutably (clingo.h:2643).
        let result = call(|| unsafe { ffi::clingo_solve_handle_close(active.handle.as_ptr()) });
        self.sync.finished();
        // The search is closed, so clingo cannot call back into its handler
        // (if any) again: dropped now, on this, the only path that closes a
        // recorded search.
        if promote_handler {
            self.take_event_handler();
        } else {
            // Discard the handler, and whatever it recorded,
            // together. `take_event_handler` is not called, so nothing is
            // moved into `event_handler_error`/`event_handler_panic`.
            drop(self.event_handler.borrow_mut().take());
        }
        result
    }

    /// Closes the open search, if any, and attaches the messages clingo logged
    /// while closing to its error.
    ///
    /// A panic from the logger stays recorded and resumes with the next call,
    /// because this also runs in `Drop`. `promote_handler` is
    /// [`ControlHandle::close_active`]'s own parameter, passed straight
    /// through.
    pub(crate) fn close_solve(&self, promote_handler: bool) -> Result<(), Error> {
        if self.solve.get().is_none() {
            return Ok(());
        }
        drop(self.capture.take());
        let result = self.close_active(promote_handler);
        #[cfg(test)]
        if let Some(payload) = PANIC_WHILE_CLOSING.take() {
            self.capture.inject_logger_panic(payload);
        }
        let messages = self.capture.take();
        result.map_err(|err| err.with_messages(messages))
    }

    /// Resumes a panic that the logger raised during
    /// [`ControlHandle::close_solve`].
    pub(crate) fn resume_logger_panic(&self) {
        self.capture.resume_panic();
    }

    /// The open search, or an error if there is none.
    fn active(&self) -> Result<ActiveSolve, Error> {
        self.solve
            .get()
            .ok_or_else(|| Error::new(ErrorKind::Logic, "no search is open on this control"))
    }

    /// Runs one call on the open search's handle, forgetting the lent model
    /// first when `moves_on` is set, because the call invalidates it.
    fn on_handle(
        &mut self,
        moves_on: bool,
        f: impl FnOnce(*mut ffi::clingo_solve_handle_t) -> bool,
    ) -> Result<(), Error> {
        let mut active = self.active()?;
        if moves_on {
            active.model = None;
            self.solve.set(Some(active));
        }
        let handle = active.handle.as_ptr();
        call(|| f(handle))
    }

    /// Records the model an out-pointer received, and whether there is one.
    fn record_model(&mut self, model: *const ffi::clingo_model_t) -> Result<bool, Error> {
        let mut active = self.active()?;
        active.model = NonNull::new(model.cast_mut());
        self.solve.set(Some(active));
        Ok(active.model.is_some())
    }

    /// Discards the lent model and lets the search look for the next one
    /// (clingo.h:2627). Once the search has finished, this does nothing.
    pub(crate) fn solve_resume(&mut self) -> Result<(), Error> {
        self.on_handle(true, |handle| {
            // SAFETY: `handle` is the open search of this control
            // (clingo.h:2627). The lent model was forgotten before this call.
            unsafe { ffi::clingo_solve_handle_resume(handle) }
        })
    }

    /// Waits for the current model, running the search up to it if none is
    /// current, and records it; `false` once there are no more models
    /// (clingo.h:2595).
    pub(crate) fn solve_wait_model(&mut self) -> Result<bool, Error> {
        let mut model: *const ffi::clingo_model_t = std::ptr::null();
        self.on_handle(false, |handle| {
            // SAFETY: `handle` is the open search of this control, and `model`
            // is a valid out-pointer (clingo.h:2595). A model that is still
            // current is returned again, so the recorded one stays valid.
            unsafe { ffi::clingo_solve_handle_model(handle, &raw mut model) }
        })?;
        self.record_model(model)
    }

    /// Records the last model of a finished satisfiable search; `false` if the
    /// search is not finished or found none (clingo.h:2616).
    pub(crate) fn solve_last_model(&mut self) -> Result<bool, Error> {
        let mut model: *const ffi::clingo_model_t = std::ptr::null();
        // clingo reuses one model object for the current and the last model
        // (clingocontrol.cc:933-948), so the current one is forgotten first.
        self.on_handle(true, |handle| {
            // SAFETY: `handle` is the open search of this control, and `model`
            // is a valid out-pointer (clingo.h:2616).
            unsafe { ffi::clingo_solve_handle_last(handle, &raw mut model) }
        })?;
        self.record_model(model)
    }

    /// The recorded model, borrowed for as long as the control is borrowed
    /// mutably, which rules out every call that could move the search on or
    /// close it (DESIGN S6).
    pub(crate) fn solve_model(&mut self) -> Option<&Model> {
        let model = self.solve.get()?.model?;
        // SAFETY: `model` was returned by clingo_solve_handle_model or
        // clingo_solve_handle_last for the open search and has not been
        // invalidated: every call that resumes, cancels or closes the search
        // forgets it first. It stays valid while the search is open
        // (clingocontrol.cc:933-948), and the returned borrow of `self` keeps
        // the search open and unmoved. `Model` is a zero-sized, align-1 view
        // of the model object (see `ModelData`), so the reference covers no
        // bytes that clingo could change.
        Some(unsafe { model.cast::<Model>().as_ref() })
    }

    /// Waits for the result of the search (clingo.h:2577). While a model is
    /// current, it is a partial result and the model stays current.
    pub(crate) fn solve_get(&mut self) -> Result<SolveOutcome, Error> {
        let mut bits = 0;
        self.on_handle(false, |handle| {
            // SAFETY: `handle` is the open search of this control, and `bits`
            // is a valid out-pointer (clingo.h:2577). With a model current, the
            // call does not move the search on (clasp_facade.cpp:230-234,
            // 258-268), so the recorded model stays valid.
            unsafe { ffi::clingo_solve_handle_get(handle, &raw mut bits) }
        })?;
        Ok(SolveOutcome::from_bits(bits))
    }

    /// Stops the search and waits until it has stopped (clingo.h:2634).
    pub(crate) fn solve_cancel(&mut self) -> Result<(), Error> {
        self.on_handle(true, |handle| {
            // SAFETY: `handle` is the open search of this control
            // (clingo.h:2634). The lent model was forgotten before this call.
            unsafe { ffi::clingo_solve_handle_cancel(handle) }
        })
    }

    /// The unsat core clingo has decided so far: empty before the search is
    /// known to be unsatisfiable, and for a satisfiable search, since the two
    /// are the same observable state to this call (clingo.h:2596-2606: "If
    /// the program is not unsatisfiable, core is set to NULL and size to
    /// zero").
    ///
    /// This reads decided state without moving the search on, so it does not
    /// forget a lent model, and it takes `&self`: closing, resuming or
    /// cancelling the search are the only calls that can invalidate what it
    /// returns, and none of them can run while this borrow is alive.
    pub(crate) fn solve_core(&self) -> Result<Vec<i32>, Error> {
        let active = self.active()?;
        let mut core: *const ffi::clingo_literal_t = std::ptr::null();
        let mut size: usize = 0;
        // SAFETY: `active.handle` is the open search of this control, and
        // `core` and `size` are valid out-pointers (clingo.h:2596-2606).
        call(|| unsafe {
            ffi::clingo_solve_handle_core(active.handle.as_ptr(), &raw mut core, &raw mut size)
        })?;
        // SAFETY: `core` is null with `size == 0` when clingo has nothing to
        // report, and otherwise points to `size` literals owned by the solve
        // handle (clingo.h:2596-2606). The array is copied into a `Vec` at
        // once, before any later call (a resumed or cancelled search, or a
        // closed handle) can overwrite or free clingo's buffer; nothing
        // between the FFI call above and this line can reach clingo.
        Ok(unsafe { raw_slice(core, size) }.to_vec())
    }
}

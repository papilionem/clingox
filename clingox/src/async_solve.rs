//! Solving in the background: [`Control::solve_async`] and
//! [`AsyncSolveHandle`] (DESIGN S4, S7, S14).

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::fmt;
use std::time::Duration;

use crate::atoms::{ProgramLiteral, literals_from_core};
use crate::control::{Assumption, ControlCore, SolveResult, assumption_literals};
use crate::error::{Error, ErrorKind, Result};
use crate::raw;

/// A search running on clasp's own thread, started by [`Control::solve_async`].
///
/// The caller's thread is free while it runs: [`wait`](Self::wait) polls or
/// waits for it with a time limit, [`cancel`](Self::cancel) stops it, and
/// [`get`](Self::get) waits for its result. The handle gives no access to
/// models; a caller who needs them from a search bounded in time uses
/// [`Control::for_each_model`] with an
/// [`InterruptHandle`](crate::InterruptHandle), or [`Control::solve_with`] with
/// a timeout.
///
/// The handle borrows the control mutably, so the control cannot be used while
/// the search runs (DESIGN S5). Dropping the handle closes the search, which
/// cancels it; a failure to close then poisons the control. Forgetting the
/// handle with [`std::mem::forget`] is safe: the control closes the search at
/// its next call, `&self` calls such as [`Control::statistics`] included, or
/// when it is dropped (S4).
///
/// **A [`SolveEventHandler`](crate::SolveEventHandler)'s own failure is
/// discarded when the search closes by drop, not by
/// [`AsyncSolveHandle::close`]**, for a handle from
/// [`Control::solve_async_with_events`]; see the same note on
/// [`SolveHandle`](crate::SolveHandle) for why.
///
/// # Examples
///
/// ```
/// # if cfg!(all(target_family = "wasm", not(target_feature = "atomics"))) { return Ok(()); }
/// use std::time::Duration;
///
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("a.")?;
/// ctl.ground(&[Part::base()])?;
/// let mut handle = ctl.solve_async(&[])?;
/// while !handle.wait(Duration::from_millis(10)) {
///     // Do something else meanwhile.
/// }
/// assert!(handle.close()?.is_sat());
/// # Ok::<(), clingox::Error>(())
/// ```
#[must_use = "dropping the handle cancels the search at once"]
pub struct AsyncSolveHandle<'c> {
    control: &'c mut ControlCore,
}

impl ScopedControl<'_> {
    /// Starts a search on clasp's own thread and returns at once.
    ///
    /// Assumptions behave as in [`Control::solve`]. No user code runs on the
    /// search's thread, except the logger (DESIGN S10).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control; this
    ///   is checked first, on every target;
    /// - [`ErrorKind::Unsupported`] on a build of clingo without threads (the
    ///   default WebAssembly target). Nothing is started, and the control is
    ///   not poisoned;
    /// - otherwise as [`Control::solve`].
    ///
    /// # Examples
    ///
    /// ```
    /// # if cfg!(all(target_family = "wasm", not(target_feature = "atomics"))) { return Ok(()); }
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a :- not a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_async(&[])?;
    /// assert!(handle.get()?.is_unsat());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_async(&mut self, assumptions: &[Assumption]) -> Result<AsyncSolveHandle<'_>> {
        self.core.guarded(
            || "solving in the background".to_owned(),
            |handle| {
                // clingo would report the async mode as a logic error, which
                // poisons (clasp_facade.cpp:429), so clingox refuses first.
                if !raw::HAS_THREADS {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        "asynchronous solving needs a build of clingo with threads",
                    ));
                }
                let literals = assumption_literals(handle, assumptions)?;
                handle.start_async(&literals)
            },
        )?;
        Ok(AsyncSolveHandle {
            control: &mut self.core,
        })
    }

    /// [`Control::solve_async`], calling `handler` for every solve event.
    ///
    /// `handler` is owned by the returned [`AsyncSolveHandle`] and must be
    /// `Send + 'static`: its callbacks run on clasp's own thread, since the
    /// search itself does (DESIGN S10's table, "solve-event handler
    /// (`solve_async`) | solver threads | `Send + 'static`, owned"), a row this
    /// part gives its first real implementation instead of an empty sink.
    ///
    /// # Errors
    ///
    /// As [`Control::solve_async`]; a handler's own error or panic is returned
    /// or resumed exactly as [`SolveEventHandler`](crate::SolveEventHandler)'s
    /// own documentation describes, and does not poison the control.
    ///
    /// # Examples
    ///
    /// ```
    /// # if cfg!(all(target_family = "wasm", not(target_feature = "atomics"))) { return Ok(()); }
    /// use std::ops::ControlFlow;
    /// use std::sync::atomic::{AtomicU32, Ordering};
    /// use std::sync::Arc;
    ///
    /// use clingox::{Control, ExtendableModel, Part, SolveEventHandler};
    ///
    /// struct CountModels(Arc<AtomicU32>);
    ///
    /// impl SolveEventHandler for CountModels {
    ///     fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> clingox::Result<ControlFlow<()>> {
    ///         self.0.fetch_add(1, Ordering::SeqCst);
    ///         Ok(ControlFlow::Continue(()))
    ///     }
    /// }
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let seen = Arc::new(AtomicU32::new(0));
    /// let mut handle = ctl.solve_async_with_events(&[], CountModels(Arc::clone(&seen)))?;
    /// assert!(handle.get()?.is_sat());
    /// assert_eq!(seen.load(Ordering::SeqCst), 1);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_async_with_events<H>(
        &mut self,
        assumptions: &[Assumption],
        handler: H,
    ) -> Result<AsyncSolveHandle<'_>>
    where
        H: crate::SolveEventHandler + Send + 'static,
    {
        self.core.guarded(
            || "solving in the background".to_owned(),
            |handle| {
                if !raw::HAS_THREADS {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        "asynchronous solving needs a build of clingo with threads",
                    ));
                }
                let literals = assumption_literals(handle, assumptions)?;
                handle.start_async_with_handler(&literals, handler)
            },
        )?;
        Ok(AsyncSolveHandle {
            control: &mut self.core,
        })
    }
}

impl AsyncSolveHandle<'_> {
    /// Waits up to `timeout` for the search to finish and says whether it
    /// has.
    ///
    /// It returns as soon as the search finishes, not after the whole
    /// timeout. `Duration::ZERO` only polls. After the search has finished,
    /// and after [`cancel`](Self::cancel), it returns `true` at once.
    pub fn wait(&mut self, timeout: Duration) -> bool {
        self.control.handle.solve_wait(timeout)
    }

    /// Waits for the search to finish and returns its result. Calling it
    /// again returns the same result.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Runtime`] if the search fails;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::Logic`], [`ErrorKind::BadAlloc`] or
    ///   [`ErrorKind::Unknown`], which poison the control.
    pub fn get(&mut self) -> Result<SolveResult> {
        self.control
            .searching_with_events(
                || "waiting for the search result".to_owned(),
                raw::ControlHandle::solve_get,
            )
            .map(SolveResult)
    }

    /// Stops the search and waits until it has stopped.
    ///
    /// [`get`](Self::get) then reports it interrupted: satisfiable if a model
    /// was found, unknown otherwise. It never leaves an interrupt queued, so
    /// the next solve call is not interrupted.
    ///
    /// # Errors
    ///
    /// As [`AsyncSolveHandle::get`].
    pub fn cancel(&mut self) -> Result<()> {
        self.control.searching_with_events(
            || "cancelling the search".to_owned(),
            raw::ControlHandle::solve_cancel,
        )
    }

    /// The subset of the assumptions given to [`Control::solve_async`] that
    /// made the search unsatisfiable, in the order they were given, sign
    /// included (clingo.h:2596-2606). Same semantics as
    /// [`SolveHandle::core`](crate::SolveHandle::core), which documents them in
    /// full; clingo does not distinguish the yield and async forms here.
    ///
    /// It is empty before the search is known to be unsatisfiable, and for a
    /// satisfiable search.
    ///
    /// **This waits for the search to finish before reading the core**: reading
    /// it while the search is still running on clasp's own thread races clasp's
    /// own write of the step summary (`ClaspFacade::stopStep`), a data race
    /// confirmed under `ThreadSanitizer`. It never moves the search on beyond
    /// that; unlike [`get`](Self::get) it does not close it, so the search
    /// stays open and later calls on this handle still work. It takes `&mut
    /// self`, not `&self`, exactly because it can block on the search
    /// finishing.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// # if cfg!(all(target_family = "wasm", not(target_feature = "atomics"))) { return Ok(()); }
    /// use clingox::{Control, Part, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("{a;b}. :- a, b.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let (a, b) = (Symbol::function("a", &[])?, Symbol::function("b", &[])?);
    /// let mut handle = ctl.solve_async(&[(a, true).into(), (b, true).into()])?;
    /// assert!(handle.get()?.is_unsat());
    /// assert_eq!(handle.core()?.len(), 2);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn core(&mut self) -> Result<Vec<ProgramLiteral>> {
        // Wait for the search to finish before reading the core
        // at all, so `solve_core` below never runs concurrently with it.
        // Without an open search (already closed, or none was ever open on
        // this handle) `solve_wait` returns `true` at once.
        self.control.handle.solve_wait(Duration::MAX);
        let core = self.control.observing_search(
            || "reading the unsat core".to_owned(),
            raw::ControlHandle::solve_core,
        )?;
        Ok(literals_from_core(core))
    }

    /// Waits for the result, as [`get`](Self::get), then closes the search
    /// and returns that result (DESIGN S7).
    ///
    /// # Errors
    ///
    /// As [`AsyncSolveHandle::get`]. The search is closed even when waiting
    /// for the result fails. A failure to close poisons the control. For a
    /// handle from [`Control::solve_async_with_events`], the handler's own
    /// error or panic, if either is set, is reported here, unlike a close
    /// triggered by dropping the handle instead (see the struct documentation).
    pub fn close(self) -> Result<SolveResult> {
        self.control.get_and_close()
    }
}

impl Drop for AsyncSolveHandle<'_> {
    fn drop(&mut self) {
        // Closing cancels the search (S7). A failure poisons the control, which
        // is all `Drop` can do with it. `finish_search` discards a handler's
        // own recorded error or panic along with the handler, rather than
        // promoting it: `Drop` has no caller to report it to.
        drop(self.control.finish_search());
    }
}

impl fmt::Debug for AsyncSolveHandle<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsyncSolveHandle").finish_non_exhaustive()
    }
}

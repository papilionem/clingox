//! Solve events: [`SolveEventHandler`] and [`Control::solve_with_events`]
//! (DESIGN S8, S10, S13).
//!
//! clingo reports four events during a search (`clingo_solve_event_type_e`,
//! clingo.h:2534-2539): a model is found, an optimisation problem is found
//! unsatisfiable (with the lower bound it has proven so far), the statistics
//! can be updated, and the search has finished. [`SolveEventHandler`] gives one
//! method per event, each with a `Continue`-returning default, mirroring
//! [`GroundProgramObserver`](crate::observer::GroundProgramObserver)'s own
//! per-callback shape.
//!
//! **`goon` is not the same channel as an error, and this matters more here
//! than anywhere else in clingox.** Returning `Ok(ControlFlow::Break(()))` from
//! any method stops the search *gracefully*, through clingo's own `goon`
//! out-parameter, and nothing is treated as a failure. **This is not the same
//! as an interrupt**: clingo reserves its own interrupted flag for a signal
//! delivered from outside the search (an
//! [`InterruptHandle`](crate::InterruptHandle), a timeout, or
//! [`Control::for_each_model`]'s own early exit, which cancels the search
//! outright); stopping through `goon` instead leaves the result exactly as
//! clingo reports it: satisfiable if a model was found, and neither exhausted
//! nor interrupted (checked directly against pyclingo 5.8.2). Tell the two
//! apart with [`SolveResult::is_exhausted`]: a graceful `Break` is never
//! exhausted, the same as an interrupt, but never sets
//! [`SolveResult::is_interrupted`] either. Returning `Err(e)` is a genuine
//! error: `e` is returned by the API call that owns the search (never poisoning
//! the control, since it is the handler's own error, not clingo's, exactly as a
//! model closure's own error does not poison it), or a panic resumes on the
//! caller's thread. Conflating `Break` with an error is the single easiest
//! mistake to make with this API.
//!
//! ```
//! use std::ops::ControlFlow;
//!
//! use clingox::{Control, ExtendableModel, Part, SolveEventHandler, SolveOptions};
//!
//! struct StopAfterOne(bool);
//!
//! impl SolveEventHandler for StopAfterOne {
//!     fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> clingox::Result<ControlFlow<()>> {
//!         if std::mem::replace(&mut self.0, true) {
//!             return Ok(ControlFlow::Break(())); // a graceful stop, not an error.
//!         }
//!         Ok(ControlFlow::Continue(()))
//!     }
//! }
//!
//! let mut ctl = Control::with_args(["--models=0"])?;
//! ctl.add_base("{a;b}.")?;
//! ctl.ground(&[Part::base()])?;
//! let result = ctl.solve_with_events(SolveOptions::new(), StopAfterOne(false))?;
//! assert!(result.is_sat() && !result.is_exhausted() && !result.is_interrupted());
//! # Ok::<(), clingox::Error>(())
//! ```

use std::ops::ControlFlow;

#[cfg(doc)]
use crate::control::Control;
use crate::control::{Assumption, ScopedControl, SolveResult, assumption_literals};
use crate::error::Result;
use crate::interrupt::SolveOptions;
use crate::model::ExtendableModel;
use crate::stats::MutableStatistics;

/// Reacts to clingo's four solve events, one method per event, each
/// defaulting to `Ok(ControlFlow::Continue(()))`: implement only the ones a
/// given handler cares about, exactly as
/// [`GroundProgramObserver`](crate::observer::GroundProgramObserver) does
/// for the ground program's own callbacks.
///
/// Install a handler with [`Control::solve_with_events`] (blocking),
/// [`Control::solve_yield_with_events`] or
/// [`Control::solve_async_with_events`]. The trait itself has **no
/// supertrait bound**: only [`Control::solve_with_events`] borrows a
/// handler for the search's own lifetime, so it may borrow the caller's
/// locals, as [`Control::for_each_model`]'s closure already can, because a
/// blocking call always closes its search and drops the handler before
/// returning. [`Control::solve_yield_with_events`] and
/// [`Control::solve_async_with_events`] both need `Send + 'static`: a yield
/// handle can be leaked (`mem::forget` is safe, DESIGN S4), which would
/// otherwise let a borrowed handler's own borrow end while the search is
/// still open, closed only by the next call on the control; an async search
/// has the same handle-leak exposure and its callbacks also run on clasp's
/// own thread. All three entry points additionally require `Send`: a
/// blocking or yielding search still runs on clasp's own thread wherever
/// clasp has threads (the caller's thread only waits for it), and with more
/// than one solver thread clasp can report a model from whichever thread
/// found it, in every mode alike (DESIGN S10).
///
/// **Reentrancy.** None of the four methods is handed anything that can
/// reach back into the `Control` that owns the search:
/// [`ExtendableModel`] and [`MutableStatistics`] are read/write views with
/// no way to call `ground` or `solve` again, and [`SolveResult`] is a plain
/// value. The one indirect path is a captured
/// [`InterruptHandle`](crate::InterruptHandle): calling `interrupt()` from
/// inside a handler method is calling into `clingo_control_interrupt` while
/// this very search is active, which is safe by construction (DESIGN S13),
/// not a new risk this trait introduces.
///
/// **Calls into the handler are serialised** behind an internal mutex
/// (`&mut self` is always exclusive), documented in full on
/// `raw::events::EventData`; briefly, clasp itself already serialises every
/// event this trait can see (model and unsat events under clasp's own
/// `modelM` mutex, `clasp/src/parallel_solve.cpp`; the finish event, right
/// after the statistics event, from whichever single thread completes a
/// step, DESIGN S13), so no two calls into a single handler ever actually
/// overlap, on any target.
pub trait SolveEventHandler {
    /// A model was found. `model` derefs to [`crate::Model`]; use
    /// [`ExtendableModel::extend`] to add symbols to it.
    ///
    /// `Ok(ControlFlow::Break(()))` stops the search gracefully: the result
    /// is satisfiable if a model was found, and neither exhausted nor
    /// interrupted (a graceful stop is not the same as an interrupt; see
    /// the module documentation). This is the only one of the four methods
    /// whose `Err` takes clingo's ordinary, safe error path.
    ///
    /// # Errors
    ///
    /// Whatever should stop the search and be returned by the API call
    /// that owns it, unchanged; it does not poison the control.
    fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
        let _ = model;
        Ok(ControlFlow::Continue(()))
    }

    /// An optimisation problem was found unsatisfiable, with the lower
    /// bound proven so far: one entry per priority level, as
    /// [`crate::Model::priorities`] orders them.
    ///
    /// # Errors
    ///
    /// As [`SolveEventHandler::on_model`]. **An `Err` or a panic here never
    /// reaches clingo as a `false` return** (U25: clingo's own internal
    /// handler calls an unconditional, unwind-free `std::_Exit(1)` if it
    /// ever does); clingox's trampoline stops the search gracefully
    /// instead and reports the stored error afterwards. The distinction is
    /// invisible from this trait's own contract, which stays uniform across
    /// all four methods; it is purely how the trampoline achieves it
    /// safely.
    fn on_unsat(&mut self, lower_bound: &[i64]) -> Result<ControlFlow<()>> {
        let _ = lower_bound;
        Ok(ControlFlow::Continue(()))
    }

    /// The statistics can be updated: `step` is this step's own tree,
    /// `accumulated` the running total across every step so far. Writes
    /// land under `user_step`/`user_accu` in
    /// [`Control::statistics`]'s ordinary, read-only view once the search
    /// finishes.
    ///
    /// This fires unconditionally, once per step, whether or not `--stats`
    /// was given; `--stats=N` only changes which of clasp's *own* entries
    /// are registered, never whether this event itself fires.
    ///
    /// # Errors
    ///
    /// As [`SolveEventHandler::on_unsat`] (the same non-`false` trampoline
    /// rule applies).
    fn on_statistics(
        &mut self,
        step: &mut MutableStatistics<'_>,
        accumulated: &mut MutableStatistics<'_>,
    ) -> Result<ControlFlow<()>> {
        let (_, _) = (step, accumulated);
        Ok(ControlFlow::Continue(()))
    }

    /// The search has completed, with the same [`SolveResult`] the API
    /// call that owns it would itself return.
    ///
    /// A solve that never reaches a real search (cancelled before it
    /// starts, for example) can fire this event with no preceding
    /// statistics event; do not assume one always precedes the other.
    ///
    /// # Errors
    ///
    /// As [`SolveEventHandler::on_unsat`] (the same non-`false` trampoline
    /// rule applies).
    fn on_finish(&mut self, result: SolveResult) -> Result<ControlFlow<()>> {
        let _ = result;
        Ok(ControlFlow::Continue(()))
    }
}

impl ScopedControl<'_> {
    /// Solves like [`Control::solve_with`], calling `handler` for every
    /// solve event.
    ///
    /// # Errors
    ///
    /// - the error `handler` returned, unchanged, or its panic resumed; it
    ///   does not poison the control (see [`SolveEventHandler`]'s own
    ///   documentation on the `goon`-vs-error distinction);
    /// - otherwise as [`Control::solve_with`].
    ///
    /// # Examples
    ///
    /// See the module documentation.
    pub fn solve_with_events<H>(&mut self, options: SolveOptions, handler: H) -> Result<SolveResult>
    where
        H: SolveEventHandler + Send,
    {
        let (assumptions, timeout) = options.into_parts();
        self.timeout_supported(timeout)?;
        self.with_deadline(timeout, |control| {
            solve_with_events_body(control, &assumptions, handler)
        })
    }
}

/// The body of [`Control::solve_with_events`], run inside the deadline
/// wrapper so a timeout interrupts it exactly as it interrupts
/// [`Control::solve_with`].
fn solve_with_events_body<H>(
    control: &mut ScopedControl<'_>,
    assumptions: &[Assumption],
    handler: H,
) -> Result<SolveResult>
where
    H: SolveEventHandler + Send,
{
    control
        .core
        .guarded_with_events(
            || "solving".to_owned(),
            |handle| {
                let literals = assumption_literals(handle, assumptions)?;
                handle.solve_with_handler(&literals, handler)
            },
        )
        .map(SolveResult)
}

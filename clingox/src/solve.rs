//! Solving with models: the solve handle, the model loop and the convenience
//! calls built on it (DESIGN S4 to S7, S10).

use std::ffi::CStr;
use std::fmt;
use std::ops::ControlFlow;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use crate::atoms::{ProgramLiteral, literals_from_core};
#[cfg(doc)]
use crate::control::Control;
use crate::control::{Assumption, ControlCore, ScopedControl, SolveResult, assumption_literals};
use crate::error::{Error, ErrorKind, Result};
use crate::interrupt::SolveOptions;
use crate::model::{Model, OwnedModel};
use crate::raw;

/// The configuration entry that holds the model limit, `--models`.
const MODEL_LIMIT: &CStr = c"solve.models";

/// A search in progress, started by [`Control::solve_yield`].
///
/// The search runs inside the handle's calls, on the caller's thread.
/// [`SolveHandle::next_model`] lends one model at a time; the model is valid
/// until the next call on the handle, which the borrow checker enforces (DESIGN
/// S6). Keep a model with [`Model::snapshot`].
///
/// The handle borrows the control mutably, so the control cannot be grounded or
/// changed while the search runs (S5). Dropping the handle closes the search,
/// which cancels it; [`SolveHandle::close`] also returns the result. Forgetting
/// the handle with [`std::mem::forget`] is safe: the control closes the search
/// at its next call, or when it is dropped (S4).
///
/// **A [`SolveEventHandler`](crate::SolveEventHandler)'s own failure is
/// discarded when the search closes by drop, not by [`SolveHandle::close`]**,
/// for a handle from [`Control::solve_yield_with_events`]. `Drop` cannot report
/// a failure, and reporting it later, from whatever call happens to close the
/// search or read its slots next, would blame a call that has nothing to do
/// with this search: the same reasoning already covers a forgotten handle's
/// search, closed silently by the next entry point on the control instead of by
/// `Drop` (S4). [`SolveHandle::close`] is the one call that does report a
/// handler's own error or panic, because it is the deliberate, caller-initiated
/// close this failure can be attributed to.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::with_args(["--models=0"])?;
/// ctl.add_base("{a;b}.")?;
/// ctl.ground(&[Part::base()])?;
/// let mut handle = ctl.solve_yield(&[])?;
/// let mut count = 0;
/// while let Some(model) = handle.next_model()? {
///     count += 1;
///     println!("{model}");
/// }
/// let result = handle.close()?;
/// assert_eq!(count, 4);
/// assert!(result.is_exhausted());
/// # Ok::<(), clingox::Error>(())
/// ```
#[must_use = "dropping the handle cancels the search at once"]
pub struct SolveHandle<'c> {
    control: &'c mut ControlCore,
    /// Whether the last `next_model` lent a model that the search has not
    /// moved past yet, so the next one must resume first.
    lent: bool,
}

impl SolveHandle<'_> {
    /// Lends the next model, or returns `None` when there are no more.
    ///
    /// The model lent by the previous call is discarded first (the search
    /// resumes), so it cannot be used after this call. After the last model,
    /// every call returns `Ok(None)`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Runtime`] if the search fails;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::Logic`], [`ErrorKind::BadAlloc`] or
    ///   [`ErrorKind::Unknown`], which poison the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let model = handle.next_model()?.expect("`a.` has a model");
    /// assert_eq!(model.number(), 1);
    /// assert!(handle.next_model()?.is_none());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn next_model(&mut self) -> Result<Option<&Model>> {
        let resume = std::mem::replace(&mut self.lent, false);
        let body = |handle: &mut raw::ControlHandle| -> Result<bool> {
            if resume {
                handle.solve_resume()?;
            }
            handle.solve_wait_model()
        };
        self.lent = self
            .control
            .searching_with_events(|| "reading the next model".to_owned(), body)?;
        Ok(self.control.handle.solve_model())
    }

    /// Waits for the result of the search and returns it.
    ///
    /// While a model is lent (the last [`next_model`](Self::next_model)
    /// returned one), this is a partial result: satisfiable, neither exhausted
    /// nor interrupted. Before the first model, it waits for that model.
    /// `get` never discards a model: the next `next_model` returns the model
    /// it waited for. After the last model it returns the final result, as
    /// often as it is called.
    ///
    /// # Errors
    ///
    /// As [`SolveHandle::next_model`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a :- not a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// assert!(handle.get()?.is_unsat());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn get(&mut self) -> Result<SolveResult> {
        self.control
            .searching_with_events(
                || "waiting for the search result".to_owned(),
                raw::ControlHandle::solve_get,
            )
            .map(SolveResult)
    }

    /// Stops the search.
    ///
    /// Later calls to [`next_model`](Self::next_model) return `Ok(None)`, and
    /// [`get`](Self::get) returns an interrupted result: satisfiable if a model
    /// was found before, unknown otherwise.
    ///
    /// # Errors
    ///
    /// As [`SolveHandle::next_model`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.add_base("{a;b}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// assert!(handle.next_model()?.is_some());
    /// handle.cancel()?;
    /// assert!(handle.next_model()?.is_none());
    /// assert!(handle.get()?.is_interrupted());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn cancel(&mut self) -> Result<()> {
        self.lent = false;
        self.control.searching_with_events(
            || "cancelling the search".to_owned(),
            raw::ControlHandle::solve_cancel,
        )
    }

    /// Waits for the result, as [`get`](Self::get), then closes the search
    /// and returns that result (DESIGN S7).
    ///
    /// Closed in the middle of the models, the result is partial:
    /// satisfiable, neither exhausted nor interrupted.
    ///
    /// The [`SolveResult`] is `#[must_use]`: if only the closing matters, write
    /// `let _ = handle.close()?;`.
    ///
    /// # Errors
    ///
    /// As [`SolveHandle::next_model`]. The search is closed even when waiting
    /// for the result fails. A failure to close poisons the control. For a
    /// handle from [`Control::solve_yield_with_events`], the handler's own
    /// error or panic, if either is set, is reported here, unlike a close
    /// triggered by dropping the handle instead (see the struct documentation).
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// while handle.next_model()?.is_some() {}
    /// assert!(handle.close()?.is_sat());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn close(mut self) -> Result<SolveResult> {
        self.lent = false;
        self.control.get_and_close()
    }

    /// The subset of the assumptions given to [`Control::solve_yield`] that
    /// made the search unsatisfiable, in the order they were given, sign
    /// included (clingo.h:2596-2606).
    ///
    /// It is empty before the search is known to be unsatisfiable, and for a
    /// satisfiable search: clingo does not distinguish the two cases here ("if
    /// the program is not unsatisfiable, core is set to NULL and size to
    /// zero"). It is not documented as minimal, and clingox does not claim it
    /// is (checked directly against clingo 5.8.2).
    ///
    /// Unlike [`SolveHandle::next_model`] and the other calls above, this takes
    /// `&self`: it only reports what the search has already decided, never
    /// moves it on, and so cannot invalidate a model still lent.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs out
    /// of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("{a;b}. :- a, b.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let (a, b) = (Symbol::function("a", &[])?, Symbol::function("b", &[])?);
    /// let mut handle = ctl.solve_yield(&[(a, true).into(), (b, true).into()])?;
    /// assert!(handle.get()?.is_unsat());
    /// assert_eq!(handle.core()?.len(), 2);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn core(&self) -> Result<Vec<ProgramLiteral>> {
        let core = self.control.observing_search(
            || "reading the unsat core".to_owned(),
            raw::ControlHandle::solve_core,
        )?;
        Ok(literals_from_core(core))
    }

    /// Lends the last model of a finished search, which is the only model
    /// whose optimality clingo reports as proven (clingo.h:2616).
    fn last_model(&mut self) -> Result<Option<&Model>> {
        self.lent = false;
        let found = self.control.searching(
            || "reading the last model".to_owned(),
            raw::ControlHandle::solve_last_model,
        )?;
        Ok(if found {
            self.control.handle.solve_model()
        } else {
            None
        })
    }
}

impl Drop for SolveHandle<'_> {
    fn drop(&mut self) {
        // Closing cancels the search (S7). A failure poisons the control, which
        // is all `Drop` can do with it. `finish_search` discards a handler's
        // own recorded error or panic along with the handler, rather than
        // promoting it: `Drop` has no caller to report it to.
        drop(self.control.finish_search());
    }
}

impl fmt::Debug for SolveHandle<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SolveHandle").finish_non_exhaustive()
    }
}

/// The result of [`Control::solve_first`] and [`Control::solve_optimal`]:
/// a model, a proof that there is none, or an undecided search.
///
/// It is deliberately not `#[non_exhaustive]`: a `match` covers all three
/// cases without a wildcard, so that "no model" cannot hide "undecided".
///
/// # Examples
///
/// ```
/// use clingox::{Control, Outcome, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("a.")?;
/// ctl.ground(&[Part::base()])?;
/// match ctl.solve_first()? {
///     Outcome::Sat(model, _) => println!("{model}"),
///     Outcome::Unsat => println!("no answer set"),
///     Outcome::Unknown(result) => println!("undecided: {result:?}"),
/// }
/// # Ok::<(), clingox::Error>(())
/// ```
#[must_use]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<T> {
    /// The program is satisfiable: the model, and the result of the whole
    /// call.
    ///
    /// The result is always satisfiable. It tells a finished search
    /// ([`is_exhausted`](SolveResult::is_exhausted)) from one stopped by a
    /// timeout ([`is_interrupted`](SolveResult::is_interrupted)), and from
    /// one closed after its first model, which is neither.
    Sat(T, SolveResult),
    /// The program was proven to have no answer set.
    Unsat,
    /// The search ended without deciding, for example at a search limit such
    /// as `--solve-limit`, or because it was interrupted. The result says
    /// which. An interrupted search without a model is always `Unknown`, even
    /// when clingo reports it unsatisfiable (see [`SolveResult`]).
    Unknown(SolveResult),
}

/// The outcome for a finished search and the model it produced, if any.
fn outcome(result: SolveResult, model: Option<OwnedModel>) -> Result<Outcome<OwnedModel>> {
    match model {
        Some(model) => Ok(Outcome::Sat(model, result)),
        None if result.is_unsat() => Ok(Outcome::Unsat),
        None if result.is_sat() => Err(Error::new(
            ErrorKind::Unknown,
            "clingo reported a satisfiable search but no model",
        )),
        None => Ok(Outcome::Unknown(result)),
    }
}

impl ScopedControl<'_> {
    /// Starts a search that yields its models one at a time.
    ///
    /// The returned [`SolveHandle`] borrows the control mutably until it is
    /// closed or dropped. The search runs inside the handle's calls, on the
    /// caller's thread. Assumptions behave as in [`Control::solve`], including
    /// assumptions on atoms that do not occur in the grounding. The number of
    /// models is limited by the control's `--models` option, which is 1 by
    /// default outside optimisation.
    ///
    /// # Errors
    ///
    /// As [`Control::solve`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, Symbol};
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.add_base("{a;b}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let a = Symbol::function("a", &[])?;
    /// let mut handle = ctl.solve_yield(&[(a, true).into()])?;
    /// while let Some(model) = handle.next_model()? {
    ///     assert!(model.contains(a)?);
    /// }
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_yield(&mut self, assumptions: &[Assumption]) -> Result<SolveHandle<'_>> {
        self.core.guarded(
            || "solving".to_owned(),
            |handle| {
                let literals = assumption_literals(handle, assumptions)?;
                handle.start_yield(&literals)
            },
        )?;
        Ok(SolveHandle {
            control: &mut self.core,
            lent: false,
        })
    }

    /// [`Control::solve_yield`], calling `handler` for every solve event.
    ///
    /// `handler` must be `Send + 'static`, unlike
    /// [`Control::solve_with_events`], which accepts a handler that borrows the
    /// caller's own locals: dropping the returned handle with
    /// [`std::mem::forget`] is safe (S4), and doing so leaves the search, and
    /// the handler with it, open indefinitely, closed only by the next call on
    /// the control; that would let a borrowed handler's own borrow end while
    /// the search could still call into it, which
    /// [`Control::solve_with_events`] cannot happen to, since it always closes
    /// the search, and drops its handler, before it returns. A yielding
    /// search with one solver thread runs on the caller's thread, inside the
    /// handle's `next_model`, `get` and `close`. `Send` is needed regardless,
    /// since parallel solving can report a model from any solver thread
    /// (DESIGN S10).
    ///
    /// The handler is dropped as soon as the search closes: when the returned
    /// handle is closed or dropped, or, for a forgotten handle, when the next
    /// call on the control finishes it (S4).
    ///
    /// [`Control::for_each_model`], [`Control::solve_first`],
    /// [`Control::solve_optimal`] and [`Control::solve_all`] (and their `_with`
    /// siblings) have no event-handler variants: they are already built on a
    /// plain [`Control::solve_yield`] with a model-only closure, and giving
    /// each of them (six, counting the `_with` siblings) a `_with_events` twin
    /// would be exactly the copy of every solve method this design avoids, for
    /// a use case (`on_unsat`, `on_statistics`, `on_finish` alongside a model
    /// loop) none of them asks for.
    ///
    /// # Errors
    ///
    /// As [`Control::solve_yield`]; a handler's own error or panic is returned
    /// or resumed exactly as [`SolveEventHandler`](crate::SolveEventHandler)'s
    /// own documentation describes, and does not poison the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::ops::ControlFlow;
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicU32, Ordering};
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
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.add_base("{a;b}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let counter = Arc::new(AtomicU32::new(0));
    /// let mut handle = ctl.solve_yield_with_events(&[], CountModels(Arc::clone(&counter)))?;
    /// while handle.next_model()?.is_some() {}
    /// handle.close()?;
    /// assert_eq!(counter.load(Ordering::SeqCst), 4);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_yield_with_events<'c, H>(
        &'c mut self,
        assumptions: &[Assumption],
        handler: H,
    ) -> Result<SolveHandle<'c>>
    where
        H: crate::SolveEventHandler + Send + 'static,
    {
        self.core.guarded(
            || "solving".to_owned(),
            |handle| {
                let literals = assumption_literals(handle, assumptions)?;
                handle.start_yield_with_handler(&literals, handler)
            },
        )?;
        Ok(SolveHandle {
            control: &mut self.core,
            lent: false,
        })
    }

    /// Calls `f` on every model, in the order clingo finds them, and returns
    /// the result of the search.
    ///
    /// It is built on [`Control::solve_yield`], never on clingo's model event,
    /// so `f` runs on the calling thread whatever the number of solver threads:
    /// it may borrow locals and need not be `Send` (DESIGN S10). It respects
    /// the control's model limit (`--models`).
    ///
    /// - `Ok(ControlFlow::Continue(()))` asks for the next model.
    /// - `Ok(ControlFlow::Break(()))` stops the search. The result is then
    ///   interrupted, and satisfiable, since a model was seen.
    /// - `Err(e)` stops the search in the same way and returns `e` unchanged.
    ///   It does not poison the control.
    ///
    /// A panic in `f` unwinds through this call; the search is closed on the
    /// way, and the control stays usable. The model passed to `f` is only
    /// valid during that call; keep it with [`Model::snapshot`].
    ///
    /// # Errors
    ///
    /// - the error `f` returned, unchanged;
    /// - otherwise as [`SolveHandle::next_model`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::prelude::*;
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.add_base("{a;b}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut models = Vec::new();
    /// let result = ctl.for_each_model(&[], |model| {
    ///     models.push(model.snapshot()?);
    ///     Ok(ControlFlow::Continue(()))
    /// })?;
    /// assert_eq!(models.len(), 4);
    /// assert!(result.is_exhausted());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn for_each_model<F>(&mut self, assumptions: &[Assumption], mut f: F) -> Result<SolveResult>
    where
        F: FnMut(&Model) -> Result<ControlFlow<()>>,
    {
        let mut handle = self.solve_yield(assumptions)?;
        while let Some(model) = handle.next_model()? {
            match f(model) {
                Ok(ControlFlow::Continue(())) => {}
                Ok(ControlFlow::Break(())) => {
                    // Leaving early is cancel, get, close (S7), so the result
                    // says the search was interrupted.
                    handle.cancel()?;
                    return handle.close();
                }
                Err(err) => {
                    // Dropping the handle closes the search, which cancels it.
                    // The closure's error is returned even if closing fails,
                    // which poisons the control.
                    drop(handle);
                    return Err(err);
                }
            }
        }
        handle.close()
    }

    /// Returns the first model clingo finds, then stops the search.
    ///
    /// The control's model limit is left as it is. On an optimisation problem
    /// this is the first model found, not the best one, and its
    /// [`optimality_proven`](OwnedModel::optimality_proven) is false; use
    /// [`Control::solve_optimal`] for the optimum. The search is closed right
    /// after the first model, so the result in [`Outcome::Sat`] is
    /// satisfiable, and neither exhausted nor interrupted.
    ///
    /// It is [`Control::solve_first_with`] with no assumptions and no
    /// timeout.
    ///
    /// # Errors
    ///
    /// As [`SolveHandle::next_model`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Outcome, Part, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("p(1). p(2).")?;
    /// ctl.ground(&[Part::base()])?;
    /// let Outcome::Sat(model, _) = ctl.solve_first()? else {
    ///     panic!("the program has a model");
    /// };
    /// assert_eq!(model.symbols().len(), 2);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_first(&mut self) -> Result<Outcome<OwnedModel>> {
        self.solve_first_with(SolveOptions::new())
    }

    /// [`Control::solve_first`] under assumptions and a timeout.
    ///
    /// The assumptions hold for this call only, as in [`Control::solve`],
    /// including an assumption on an atom that does not occur in the
    /// grounding (such an atom is false, so assuming it true gives
    /// [`Outcome::Unsat`]).
    ///
    /// A timeout is a budget for the whole call, counted from its start. When
    /// it is spent, the search is interrupted, as by [`Control::solve_with`]:
    /// the outcome is [`Outcome::Unknown`] if no model was found by then. A
    /// search that ends within its budget returns at once with its own
    /// result. A timeout never poisons the control and never reaches a later
    /// call.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Unsupported`] for a timeout on a build of clingo
    ///   without threads, before anything is solved. It does not poison the
    ///   control;
    /// - otherwise as [`Control::solve_first`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Outcome, Part, SolveOptions, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("{a;b}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let a = Symbol::function("a", &[])?;
    /// let options = SolveOptions::new().assumptions(&[(a, true).into()]);
    /// let Outcome::Sat(model, _) = ctl.solve_first_with(options)? else {
    ///     panic!("there is a model with `a`");
    /// };
    /// assert!(model.contains(a));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_first_with(&mut self, options: SolveOptions) -> Result<Outcome<OwnedModel>> {
        let (assumptions, timeout) = options.into_parts();
        let found = self
            .timeout_supported(timeout)
            .and_then(|()| self.with_deadline(timeout, |c| first_model(c, &assumptions)));
        found.map_err(|err| self.core.note(err))
    }

    /// Returns an optimal model, with its optimality proven when the program
    /// has optimisation statements.
    ///
    /// The search runs with clingo's default model limit (`solve.models = -1`:
    /// every model on an optimisation problem, one otherwise), whatever the
    /// control was configured with, and the control's own setting is restored
    /// afterwards. The optimisation mode is the control's own `--opt-mode`.
    ///
    /// When the search has finished, the model is the one clingo returns as
    /// the last model of the search, and the result in [`Outcome::Sat`] is
    /// exhausted. Under the default `--opt-mode=opt`, the last model is the
    /// only one whose [`optimality_proven`](OwnedModel::optimality_proven) is
    /// true: models seen during the search report false, even the optimal one.
    /// Under `--opt-mode=optN`, clingo first searches for the optimum, then
    /// enumerates the optimal models, which it reports as proven and numbers
    /// from 1 again; the model returned is the last optimal model enumerated.
    /// On a program without optimisation statements the result is one model,
    /// with an empty cost and `optimality_proven` false.
    ///
    /// It is [`Control::solve_optimal_with`] with no assumptions and no
    /// timeout.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Runtime`] if the model limit cannot be set or restored;
    ///   a failed restore poisons the control, which then no longer behaves
    ///   as configured;
    /// - otherwise as [`SolveHandle::next_model`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Outcome, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("{ a; b; c }. :~ a. [1] :~ b. [2] :~ not c. [3]")?;
    /// ctl.ground(&[Part::base()])?;
    /// let Outcome::Sat(best, result) = ctl.solve_optimal()? else {
    ///     panic!("the problem is satisfiable");
    /// };
    /// assert_eq!(best.cost(), [0]);
    /// assert!(best.optimality_proven());
    /// assert!(result.is_exhausted());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_optimal(&mut self) -> Result<Outcome<OwnedModel>> {
        self.solve_optimal_with(SolveOptions::new())
    }

    /// [`Control::solve_optimal`] under assumptions and a timeout.
    ///
    /// Assumptions and the timeout behave as in [`Control::solve_first_with`].
    /// When the timeout interrupts the search, the outcome is
    /// [`Outcome::Sat`] with the last model found, which under
    /// `--opt-mode=opt` is the best so far: its
    /// [`optimality_proven`](OwnedModel::optimality_proven) is false, and the
    /// result is interrupted. It is [`Outcome::Unknown`] if no model was found
    /// by then. The control's model limit is restored on every path.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Unsupported`] for a timeout on a build of clingo
    ///   without threads, before anything is solved. It does not poison the
    ///   control;
    /// - otherwise as [`Control::solve_optimal`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Outcome, Part, SolveOptions, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("{ a; b; c }. :~ a. [1] :~ b. [2] :~ not c. [3]")?;
    /// ctl.ground(&[Part::base()])?;
    /// let c = Symbol::function("c", &[])?;
    /// let options = SolveOptions::new().assumptions(&[(c, false).into()]);
    /// let Outcome::Sat(best, _) = ctl.solve_optimal_with(options)? else {
    ///     panic!("the problem is satisfiable without `c`");
    /// };
    /// assert_eq!(best.cost(), [3]);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_optimal_with(&mut self, options: SolveOptions) -> Result<Outcome<OwnedModel>> {
        let (assumptions, timeout) = options.into_parts();
        let found = self.timeout_supported(timeout).and_then(|()| {
            self.with_deadline(timeout, |c| {
                c.with_model_limit(c"-1", |c| optimal_model(c, &assumptions))
            })
        });
        found.map_err(|err| self.core.note(err))
    }

    /// Returns the result of the search and every model clingo reports.
    ///
    /// The search runs with the model limit lifted (`solve.models = 0`),
    /// whatever the control was configured with, and the control's own setting
    /// is restored afterwards. The models are [`OwnedModel`]s, whose symbols
    /// are sorted. The models themselves are sorted by
    /// [`symbols`](OwnedModel::symbols), then by
    /// [`all_atoms`](OwnedModel::all_atoms), in [`Symbol`](crate::Symbol)'s
    /// order, so the result does not depend on the order of the search.
    ///
    /// On an optimisation problem these are all the models clingo reports under
    /// the control's `--opt-mode`, including the models that are not optimal
    /// and were found on the way to the optimum. Use `--opt-mode=optN` to
    /// enumerate the optimal models.
    ///
    /// It is [`Control::solve_all_with`] with no assumptions and no timeout.
    ///
    /// # Errors
    ///
    /// As [`Control::solve_optimal`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("{a;b}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let (result, models) = ctl.solve_all()?;
    /// assert!(result.is_exhausted());
    /// let texts: Vec<String> = models.iter().map(ToString::to_string).collect();
    /// assert_eq!(texts.len(), 4);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_all(&mut self) -> Result<(SolveResult, Vec<OwnedModel>)> {
        self.solve_all_with(SolveOptions::new())
    }

    /// [`Control::solve_all`] under assumptions and a timeout.
    ///
    /// Assumptions and the timeout behave as in [`Control::solve_first_with`].
    /// When the timeout interrupts the search, the models found so far are
    /// returned, sorted, with an interrupted result. The control's model
    /// limit is restored on every path.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Unsupported`] for a timeout on a build of clingo
    ///   without threads, before anything is solved. It does not poison the
    ///   control;
    /// - otherwise as [`Control::solve_all`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, SolveOptions, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("{a;b}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let a = Symbol::function("a", &[])?;
    /// let options = SolveOptions::new().assumptions(&[(a, true).into()]);
    /// let (result, models) = ctl.solve_all_with(options)?;
    /// assert!(result.is_exhausted());
    /// assert_eq!(models.len(), 2);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_all_with(
        &mut self,
        options: SolveOptions,
    ) -> Result<(SolveResult, Vec<OwnedModel>)> {
        let (assumptions, timeout) = options.into_parts();
        let found = self.timeout_supported(timeout).and_then(|()| {
            self.with_deadline(timeout, |c| {
                c.with_model_limit(c"0", |c| all_models(c, &assumptions))
            })
        });
        found.map_err(|err| self.core.note(err))
    }

    /// Refuses a poisoned control, and a timeout on a build of clingo without
    /// threads, before anything is solved (as [`Control::solve_with`]).
    pub(crate) fn timeout_supported(&self, timeout: Option<Duration>) -> Result<()> {
        self.core.refusal()?;
        if timeout.is_some() && !raw::HAS_THREADS {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "a timeout needs a build of clingo with threads",
            )
            .context("solving"));
        }
        Ok(())
    }

    /// Runs `f`, and interrupts the search it runs once `timeout` has passed
    /// since this call started (DESIGN S14).
    ///
    /// The model loops of `f` run on this thread, so the budget is kept by a
    /// second thread that sends an [`InterruptHandle`](crate::InterruptHandle)
    /// interrupt. An interrupt only ever reaches a running search (S13), and
    /// the thread is joined before this returns, so none can reach a later
    /// call.
    pub(crate) fn with_deadline<T>(
        &mut self,
        timeout: Option<Duration>,
        f: impl FnOnce(&mut ScopedControl<'_>) -> Result<T>,
    ) -> Result<T> {
        // How often the timeout thread retries an interrupt that found no
        // search running yet.
        const DEADLINE_RETRY: Duration = Duration::from_millis(1);
        let Some(budget) = timeout else {
            return f(self);
        };
        let interrupt = self.interrupt_handle();
        let (done, finished) = mpsc::channel::<()>();
        thread::scope(|scope| {
            thread::Builder::new()
                .name("clingox-timeout".to_owned())
                .spawn_scoped(scope, move || {
                    // A disconnected channel means `f` has returned (or
                    // unwound), and the search with it.
                    if finished.recv_timeout(budget) == Err(RecvTimeoutError::Timeout) {
                        // An interrupt only reaches a search that is starting
                        // or running (S13), and the deadline can pass before
                        // `f` has started its search: keep trying until one is
                        // accepted or `f` returns. Once `f` returns no search
                        // of this call is left, so none of a later call is hit.
                        while !interrupt.interrupt() {
                            if finished.recv_timeout(DEADLINE_RETRY)
                                != Err(RecvTimeoutError::Timeout)
                            {
                                break;
                            }
                        }
                    }
                })
                .map_err(|e| {
                    Error::new(
                        ErrorKind::Runtime,
                        format!("cannot start the thread that keeps the timeout: {e}"),
                    )
                    .context("solving")
                })?;
            let value = f(self);
            drop(done);
            value
        })
    }

    /// Runs `f` with the model limit set to `limit`, and restores the
    /// control's own limit on every path out, errors and panics included.
    fn with_model_limit<T>(
        &mut self,
        limit: &CStr,
        f: impl FnOnce(&mut ScopedControl<'_>) -> Result<T>,
    ) -> Result<T> {
        self.with_model_limit_restored_by(limit, f, |handle, previous| {
            handle.config_set(MODEL_LIMIT, previous)
        })
    }

    /// [`Control::with_model_limit`], with the restore as a parameter so that
    /// tests can make it fail, which clingo cannot be made to do.
    ///
    /// The restore always runs. A failed restore poisons the control whatever
    /// its kind, since the control no longer behaves as configured:
    ///
    /// | `f` | restore | returned | poisoned |
    /// |---|---|---|---|
    /// | ok | ok | the value | no |
    /// | ok | failed | the restore error | yes |
    /// | failed | ok | `f`'s error | as its kind decides |
    /// | failed | failed | `f`'s error | yes, with the restore error as the cause |
    /// | panicked | any | the panic resumes | yes, if the restore failed |
    fn with_model_limit_restored_by<T>(
        &mut self,
        limit: &CStr,
        f: impl FnOnce(&mut ScopedControl<'_>) -> Result<T>,
        restore: impl FnOnce(&mut raw::ControlHandle, &CStr) -> Result<()>,
    ) -> Result<T> {
        // clingo always assigns the model limit, so `None` does not happen; the
        // default stands in for it.
        let previous = self
            .core
            .guarded(
                || "reading the model limit".to_owned(),
                |handle| handle.config_get(MODEL_LIMIT),
            )?
            .unwrap_or_else(|| "-1".to_owned());
        let context = || "setting the model limit".to_owned();
        let previous = raw::c_str(&previous).map_err(|err| err.context(context()))?;
        self.core
            .guarded(context, |handle| handle.config_set(MODEL_LIMIT, limit))?;
        // User code runs inside `f` (a `log` backend can panic while messages
        // are captured), and the caller's limit must be restored on every path,
        // so a panic is held until the restore has run.
        let value = panic::catch_unwind(AssertUnwindSafe(|| f(self)));
        let restored = self
            .core
            .refusal()
            .and_then(|()| self.core.finish_search())
            .and_then(|()| restore(&mut self.core.handle, &previous))
            .map_err(|err| err.context("restoring the model limit"));
        match value {
            Ok(value) => raw::settle(value, restored).map_err(|err| self.core.note(err)),
            Err(payload) => {
                if let Err(err) = restored {
                    self.core.note(err.poisoning());
                }
                panic::resume_unwind(payload)
            }
        }
    }
}

/// The body of [`Control::solve_first_with`].
fn first_model(
    control: &mut ScopedControl<'_>,
    assumptions: &[Assumption],
) -> Result<Outcome<OwnedModel>> {
    let mut handle = control.solve_yield(assumptions)?;
    let model = handle.next_model()?.map(Model::snapshot).transpose()?;
    let result = handle.close()?;
    outcome(result, model)
}

/// The body of [`Control::solve_optimal_with`], under the default model
/// limit.
fn optimal_model(
    control: &mut ScopedControl<'_>,
    assumptions: &[Assumption],
) -> Result<Outcome<OwnedModel>> {
    let mut handle = control.solve_yield(assumptions)?;
    // The search must run to its end before clingo returns the last model.
    while handle.next_model()?.is_some() {}
    let result = handle.get()?;
    let model = if result.is_sat() {
        handle.last_model()?.map(Model::snapshot).transpose()?
    } else {
        None
    };
    let result = handle.close()?;
    outcome(result, model)
}

/// The body of [`Control::solve_all_with`], with the model limit lifted.
fn all_models(
    control: &mut ScopedControl<'_>,
    assumptions: &[Assumption],
) -> Result<(SolveResult, Vec<OwnedModel>)> {
    let mut handle = control.solve_yield(assumptions)?;
    let mut models = Vec::new();
    while let Some(model) = handle.next_model()? {
        models.push(model.snapshot()?);
    }
    let result = handle.close()?;
    models.sort_by(|a, b| {
        a.symbols()
            .cmp(b.symbols())
            .then_with(|| a.all_atoms().cmp(b.all_atoms()))
    });
    Ok((result, models))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Control;
    use crate::raw::SolveOutcome;

    #[test]
    fn an_interrupted_search_without_a_model_is_unknown_whatever_clingo_says() {
        // Unsatisfiable, exhausted and interrupted: what clingo 5.8.2 reports
        // now and then for a satisfiable program stopped at its start.
        let claimed = SolveResult(SolveOutcome::from_bits(2 | 4 | 8));
        assert_eq!(outcome(claimed, None).unwrap(), Outcome::Unknown(claimed));
        let proven = SolveResult(SolveOutcome::from_bits(2 | 4));
        assert_eq!(outcome(proven, None).unwrap(), Outcome::Unsat);
    }

    fn model_limit(control: &mut ScopedControl<'_>) -> String {
        control
            .core
            .guarded(String::new, |handle| handle.config_get(MODEL_LIMIT))
            .unwrap()
            .unwrap()
    }

    /// A panic the logger raises while the search closes
    /// resumes from `close`, not from `Drop` or the next call.
    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn a_logger_panic_while_closing_resumes_in_close() {
        let mut control = Control::new().unwrap();
        control.add_base("a.").unwrap();
        control.ground(&[crate::Part::base()]).unwrap();
        let mut handle = control.solve_yield(&[]).unwrap();
        assert!(handle.next_model().unwrap().is_some());
        raw::panic_while_closing(Box::new("the logger failed while closing"));
        let unwound = panic::catch_unwind(AssertUnwindSafe(|| handle.close()))
            .expect_err("the panic resumes from close");
        assert_eq!(
            unwound.downcast_ref::<&str>(),
            Some(&"the logger failed while closing")
        );
        // Nothing is left to resume in the next call, which works.
        assert!(control.solve(&[]).unwrap().is_sat());
        assert!(!format!("{control:?}").contains("poisoned"));
    }

    fn injected(text: &str) -> Error {
        Error::new(ErrorKind::Runtime, text)
    }

    fn poisoned_by(control: &ScopedControl<'_>) -> Option<String> {
        let shown = format!("{control:?}");
        shown.contains("poisoned").then_some(shown)
    }

    /// Every row of
    /// the table on `with_model_limit_restored_by`, with the restore failure
    /// injected, since clingo cannot be made to fail it.
    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn a_failed_restore_poisons_even_when_the_search_failed() {
        let failing_restore = |_: &mut raw::ControlHandle, _: &CStr| -> Result<()> {
            Err(injected("the restore failed"))
        };
        let real_restore = |handle: &mut raw::ControlHandle, previous: &CStr| {
            handle.config_set(MODEL_LIMIT, previous)
        };

        // The search succeeds and the restore succeeds: the value.
        let mut control = Control::with_args(["--models=3"]).unwrap();
        let value = control.with_model_limit_restored_by(c"0", |_| Ok(7), real_restore);
        assert_eq!(value.unwrap(), 7);
        assert_eq!(model_limit(&mut control), "3");
        assert!(poisoned_by(&control).is_none());

        // The search succeeds and the restore fails: the restore error, which
        // poisons although it is a runtime error.
        let mut control = Control::new().unwrap();
        let err = control
            .with_model_limit_restored_by(c"0", |_| Ok(7), failing_restore)
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "restoring the model limit: the restore failed"
        );
        assert!(poisoned_by(&control).is_some_and(|s| s.contains("the restore failed")));

        // The search fails and the restore succeeds: the search error, whose
        // kind decides; a runtime error does not poison.
        let mut control = Control::with_args(["--models=3"]).unwrap();
        let err = control
            .with_model_limit_restored_by(
                c"0",
                |_| -> Result<()> { Err(injected("the search failed")) },
                real_restore,
            )
            .unwrap_err();
        assert_eq!(err.to_string(), "the search failed");
        assert!(poisoned_by(&control).is_none());
        assert_eq!(model_limit(&mut control), "3");

        // Both fail: the search error, and the control poisoned with the
        // restore error as the cause.
        let mut control = Control::new().unwrap();
        let err = control
            .with_model_limit_restored_by(
                c"0",
                |_| -> Result<()> { Err(injected("the search failed")) },
                failing_restore,
            )
            .unwrap_err();
        assert_eq!(err.to_string(), "the search failed");
        assert!(poisoned_by(&control).is_some_and(|s| s.contains("the restore failed")));

        // The search panics and the restore fails: the panic resumes, and the
        // control is poisoned.
        let mut control = Control::new().unwrap();
        let unwound = panic::catch_unwind(AssertUnwindSafe(|| {
            let _: Result<()> = control.with_model_limit_restored_by(
                c"0",
                |_| panic!("user code panicked"),
                failing_restore,
            );
        }));
        assert!(unwound.is_err(), "the panic must reach the caller");
        assert!(poisoned_by(&control).is_some_and(|s| s.contains("the restore failed")));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn a_panic_inside_the_model_limit_scope_restores_the_limit() {
        let mut control = Control::with_args(["--models=3"]).unwrap();
        assert_eq!(model_limit(&mut control), "3");
        let unwound = panic::catch_unwind(AssertUnwindSafe(|| {
            let _: Result<()> = control.with_model_limit(c"0", |_| panic!("user code panicked"));
        }));
        assert!(unwound.is_err(), "the panic must reach the caller");
        assert_eq!(model_limit(&mut control), "3");
    }
}

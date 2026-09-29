//! Bounding a search in time: [`InterruptHandle`], and
//! [`Control::solve_with`] with a timeout (DESIGN S13, S14).

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::control::{Assumption, SolveResult, assumption_literals};
use crate::error::{Error, ErrorKind, Result};
use crate::raw::{self, SolveSync};

/// Stops the solve call running on a control, from any thread.
///
/// [`Control::interrupt_handle`] makes one. It does not borrow the control
/// and may outlive it; clones share one state. It is
/// `Send + Sync + Clone + 'static`, so it can go to another thread, into a
/// model closure, a ground callback or the logger.
///
/// **Only the running solve call is stopped.** clingo 5.8.2 queues an
/// interrupt that arrives while no search runs and ends the *next* solve call
/// with it. clingox never lets that happen: [`InterruptHandle::interrupt`]
/// acts only while a solve call is running, and returns `false` and does
/// nothing otherwise (DESIGN S13). Grounding cannot be interrupted.
///
/// # Examples
///
/// ```
/// use std::ops::ControlFlow;
///
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::with_args(["--models=0"])?;
/// ctl.add_base("{a;b;c}.")?;
/// ctl.ground(&[Part::base()])?;
/// let stop = ctl.interrupt_handle();
/// assert!(!stop.interrupt(), "nothing is solving");
/// let result = ctl.for_each_model(&[], |_| {
///     assert!(stop.interrupt());
///     Ok(ControlFlow::Continue(()))
/// })?;
/// assert!(result.is_interrupted());
/// assert!(!ctl.solve(&[])?.is_interrupted(), "the next solve is not affected");
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone)]
pub struct InterruptHandle {
    sync: Arc<SolveSync>,
}

impl InterruptHandle {
    /// Stops the solve call that is running and returns `true`, or returns
    /// `false` and does nothing when no solve call is running.
    ///
    /// Every solve call can be stopped: [`Control::solve`],
    /// [`Control::solve_with`], a [`SolveHandle`](crate::SolveHandle) and the
    /// calls built on it, and an [`AsyncSolveHandle`](crate::AsyncSolveHandle).
    /// The search ends as soon as clasp sees the signal, and its result is
    /// interrupted: satisfiable if a model was found before, unknown
    /// otherwise. In a yield handle with a model current, the next
    /// [`next_model`](crate::SolveHandle::next_model) returns `None`.
    ///
    /// It returns `false` while the control is idle or grounding, after it is
    /// dropped, and once the search of an open handle has finished. An
    /// interrupt that arrives while a solve call is still starting is
    /// delivered as soon as the search runs; if the search ends at its start
    /// without running, there is nothing left to stop. In no case does an
    /// interrupt reach a later solve call.
    ///
    /// On a build of clingo without threads (the default WebAssembly target),
    /// [`Control::solve`] and [`Control::solve_with`] run the whole search
    /// inside one call to clingo on the control's thread. The only code that
    /// could interrupt them is a callback such as the logger, which cannot
    /// tell whether the search has started, so `interrupt` returns `false`
    /// there. Searches through a [`SolveHandle`](crate::SolveHandle) can be
    /// interrupted on every build.
    pub fn interrupt(&self) -> bool {
        self.sync.interrupt()
    }
}

impl fmt::Debug for InterruptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InterruptHandle")
            .field("solving", &self.sync.is_running())
            .finish()
    }
}

impl ScopedControl<'_> {
    /// A handle that stops this control's running solve call from any
    /// thread. It does not call clingo.
    ///
    /// # Examples
    ///
    /// ```
    /// let ctl = clingox::Control::new()?;
    /// let stop = ctl.interrupt_handle();
    /// drop(ctl);
    /// assert!(!stop.interrupt(), "the handle outlives the control");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn interrupt_handle(&self) -> InterruptHandle {
        InterruptHandle {
            sync: self.core.handle.interrupt_state(),
        }
    }

    /// Solves like [`Control::solve`], with the assumptions and the time
    /// budget in `options`.
    ///
    /// A timeout interrupts the search once the budget is spent (DESIGN S14).
    /// The result is then interrupted and not exhausted: satisfiable if a model
    /// was found before, unknown otherwise. A search that ends within its
    /// budget returns as soon as it ends, with its own result. A timeout does
    /// not poison, and never reaches a later solve call.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Unsupported`] for a timeout on a build of clingo
    ///   without threads (the default WebAssembly target): there is no thread
    ///   to stop a blocking search. Nothing is solved, and the control is not
    ///   poisoned. Without a timeout it solves normally there;
    /// - otherwise as [`Control::solve`].
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use clingox::{Control, Part, SolveOptions};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let result = ctl.solve_with(SolveOptions::new())?;
    /// assert!(result.is_sat());
    /// # if !cfg!(all(target_family = "wasm", not(target_feature = "atomics"))) {
    /// let result = ctl.solve_with(SolveOptions::new().timeout(Duration::from_secs(10)))?;
    /// assert!(result.is_sat() && !result.is_interrupted());
    /// # }
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve_with(&mut self, options: SolveOptions) -> Result<SolveResult> {
        let SolveOptions {
            assumptions,
            timeout,
        } = options;
        self.core.guarded(
            || "solving".to_owned(),
            |handle| {
                if timeout.is_some() && !raw::HAS_THREADS {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        "a timeout needs a build of clingo with threads",
                    ));
                }
                let literals = assumption_literals(handle, &assumptions)?;
                match timeout {
                    Some(budget) => handle.solve_timed(&literals, budget),
                    None => handle.solve(&literals),
                }
                .map(SolveResult)
            },
        )
    }
}

/// The options of [`Control::solve_with`], [`Control::solve_first_with`],
/// [`Control::solve_optimal_with`] and [`Control::solve_all_with`]:
/// assumptions and a time budget.
///
/// Both apply to one call only. A timeout needs a build of clingo with
/// threads; without them, the calls return [`ErrorKind::Unsupported`].
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use clingox::{SolveOptions, Symbol};
///
/// let a = Symbol::function("a", &[])?;
/// let options = SolveOptions::new()
///     .assumptions(&[(a, true).into()])
///     .timeout(Duration::from_secs(5));
/// # Ok::<(), clingox::Error>(())
/// ```
#[must_use]
#[derive(Clone, Debug, Default)]
pub struct SolveOptions {
    assumptions: Vec<Assumption>,
    timeout: Option<Duration>,
}

impl SolveOptions {
    /// No assumptions and no timeout.
    pub fn new() -> SolveOptions {
        SolveOptions::default()
    }

    /// Solves under these assumptions, as [`Control::solve`] takes them. An
    /// atom the grounding does not have counts as false there, unlike in
    /// pyclingo; see [`Assumption`] for the difference and a filter.
    pub fn assumptions(mut self, assumptions: &[Assumption]) -> SolveOptions {
        assumptions.clone_into(&mut self.assumptions);
        self
    }

    /// Interrupts the search once `budget` is spent.
    pub fn timeout(mut self, budget: Duration) -> SolveOptions {
        self.timeout = Some(budget);
        self
    }

    /// The assumptions and the timeout, for the calls that take options.
    pub(crate) fn into_parts(self) -> (Vec<Assumption>, Option<Duration>) {
        (self.assumptions, self.timeout)
    }
}

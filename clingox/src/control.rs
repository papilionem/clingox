//! The control object: adding, grounding and solving programs.

use std::cell::RefCell;
use std::ffi::CString;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use crate::atoms::ProgramLiteral;
use crate::error::{Error, ErrorKind, Message, MessageCode, Poison, Result};
use crate::raw::{self, ControlHandle, GroundError, SolveOutcome};
use crate::symbol::Symbol;

/// A clingo control object: it holds a program and grounds and solves it.
///
/// This is the type of every control. [`Control`] is the kind you create
/// yourself, `ScopedControl<'static>`, and the one you will nearly always name.
/// The lifetime `'r` is a **brand**: it marks a control that clingo owns and
/// lends to a callback, such as the one [`Application::main`] passes. Such a
/// control is freed by clingo after the callback returns, so it must not
/// outlive the call, and neither may anything borrowed from it. The brand makes
/// the compiler enforce that: the callback is written for every `'r`, so `'r`
/// ends inside the call, the control cannot be moved out with
/// [`std::mem::replace`] or [`std::mem::swap`] (a `Control` is not a
/// `ScopedControl<'r>` for that `'r`, because the brand is invariant), and its
/// statistics, atoms, models and search handles cannot escape.
///
/// A function that should work on either kind takes `&mut ScopedControl<'_>`.
/// `&mut Control` accepts only the owned kind, which is why a closure passed to
/// [`Application::main`] should leave its parameter unannotated or write `&mut
/// ScopedControl<'_>`.
///
/// ```
/// use clingox::{Part, ScopedControl};
///
/// fn ground_and_count(ctl: &mut ScopedControl<'_>, program: &str) -> clingox::Result<usize> {
///     ctl.add_base(program)?;
///     ctl.ground(&[Part::base()])?;
///     Ok(ctl.solve_all()?.1.len())
/// }
///
/// // An owned control is a `ScopedControl<'static>`.
/// let mut ctl = clingox::Control::new()?;
/// assert_eq!(ground_and_count(&mut ctl, "{a;b}.")?, 4);
/// # Ok::<(), clingox::Error>(())
/// ```
///
/// [`Application::main`]: crate::application::Application::main
///
/// Programs are added with [`Control::add`], grounded with [`Control::ground`]
/// and solved with [`Control::solve`]. Each call can be repeated, which is
/// clingo's multi-shot solving.
///
/// A `Control` is `Send` but not `Sync` (DESIGN S12): it can move to another
/// thread between calls, with its [`SolveHandle`](crate::SolveHandle) or
/// [`AsyncSolveHandle`](crate::AsyncSolveHandle), but two threads never use it
/// at once. A server that needs clingo on several threads runs one `Control`
/// per thread, or moves one to wherever it is needed. The views that borrow it
/// ([`Model`](crate::Model), [`Statistics`](crate::Statistics),
/// [`SymbolicAtoms`](crate::SymbolicAtoms) and
/// [`Configuration`](crate::Configuration)) stay on their thread.
///
/// **Poisoning.** After an error clingo cannot recover from, every later method
/// returns [`ErrorKind::Poisoned`], and the `Control` can only be dropped. A
/// failure to close a search also poisons it. These errors poison it: a
/// [`ErrorKind::Parse`] error from [`Control::add`], because clingo cannot
/// ground after one, and any [`ErrorKind::Logic`], [`ErrorKind::BadAlloc`] or
/// [`ErrorKind::Unknown`] error. Errors that clingox detects before calling
/// clingo, such as [`ErrorKind::Nul`], do not poison it.
///
/// **An error that stops [`Control::ground`] or [`Control::ground_with`]
/// partway also poisons it, whatever its kind**: a failed or panicking ground
/// callback ([`Control::ground_with`]), a failed or panicking
/// [`GroundProgramObserver`](crate::observer::GroundProgramObserver) callback,
/// and [`ErrorKind::GroundingLimit`] all poison. clingo keeps whatever it
/// already ground before the failure and answers from it silently on a later
/// solve; clingox refuses to expose that truncated, silently wrong program, so
/// recovery means building a new `Control`.
///
/// **A registered observer's own failure poisons the same way outside grounding
/// too**: a directive [`Control::with_backend`] adds, and every call that
/// starts a search (`end_step` fires when a solve starts, not only when
/// grounding finishes), can still reach a registered observer's callback.
/// clingo has already passed part of the output on by the time the observer
/// stops it, so this is the same truncated-program hazard, wherever it is
/// triggered from; see
/// [`GroundProgramObserver`](crate::observer::GroundProgramObserver)'s own
/// rustdoc for the exact list of entry points.
///
/// **Messages.** clingo's warnings and errors are captured during each call.
/// When the call fails they are attached to the error ([`Error::messages`]).
/// Every message is also passed to the logger set with
/// [`ControlBuilder::logger`](crate::ControlBuilder::logger), or, without one
/// and with the feature `log` (on by default), to the `log` crate, target
/// `clingox`.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("a :- not b. b :- not a.")?;
/// ctl.ground(&[Part::base()])?;
/// let result = ctl.solve(&[])?;
/// assert!(result.is_sat());
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct ScopedControl<'r> {
    pub(crate) core: ControlCore,
    /// Invariant in `'r`, so a control cannot be turned into one with another
    /// brand, and in particular cannot be swapped for an owned one.
    _brand: PhantomData<fn(&'r ()) -> &'r ()>,
}

/// The control you create yourself: [`Control::new`], [`Control::with_args`]
/// and [`Control::builder`]. It is a [`ScopedControl`] with the brand
/// `'static`, and has every method described there.
pub type Control = ScopedControl<'static>;

/// The state every [`Control`] wraps, and what the views derived from one
/// borrow (`Statistics`, `SymbolicAtoms`, the solve handles and so on). It
/// holds the raw handle, the poison state and the helpers that run one
/// operation (DESIGN S3 to S5); the public methods of [`Control`] forward to
/// it.
pub(crate) struct ControlCore {
    pub(crate) handle: ControlHandle,
    /// The error that poisoned the control, as its one-line text. It is a
    /// `RefCell` because closing a leftover search, which can poison, also
    /// runs from `&self` (DESIGN S4).
    pub(crate) poisoned: RefCell<Option<String>>,
    /// How many program parts `add_facts` has used, so that each call gets a
    /// part of its own.
    pub(crate) fact_parts: u64,
}

impl ScopedControl<'static> {
    /// Creates a control with clingo's default options.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory;
    /// - [`ErrorKind::Version`] if the linked clingo is not 5.8.1 or newer
    ///   within 5.8.
    ///
    /// # Examples
    ///
    /// ```
    /// let ctl = clingox::Control::new()?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn new() -> Result<Control> {
        Control::with_args(std::iter::empty::<&str>())
    }

    /// Creates a control with command-line options, as the `clingo` program
    /// takes them.
    ///
    /// Only grounding and solving options are accepted, not basic options such
    /// as `--help` or `--output` (clingo.h:2979-2980).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Logic`] for an unknown option or a bad value: clingo
    ///   5.8.2 reports these as logic errors, although clingo.h documents a
    ///   runtime error;
    /// - [`ErrorKind::Parse`] for a `-c` (`--const`) definition that is not
    ///   valid, such as `-c a` or `-c a=b c` (clingo itself reports a runtime
    ///   error); [`ErrorKind::Runtime`] for a `--configuration` that is neither
    ///   a preset nor a file clingo can read;
    /// - [`ErrorKind::Nul`] if an argument contains a NUL byte;
    /// - [`ErrorKind::BadAlloc`] and [`ErrorKind::Version`] as for
    ///   [`Control::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, ErrorKind};
    ///
    /// let ctl = Control::with_args(["--models=0", "--opt-mode=optN"])?;
    /// let err = Control::with_args(["--no-such-option"]).unwrap_err();
    /// assert_eq!(err.kind(), ErrorKind::Logic);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn with_args<I, S>(args: I) -> Result<Control>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Control::builder().args(args).build()
    }

    /// Wraps a newly created control.
    pub(crate) fn from_handle(handle: ControlHandle) -> Control {
        ScopedControl::wrap(handle)
    }
}

impl<'r> ScopedControl<'r> {
    fn wrap(handle: ControlHandle) -> ScopedControl<'r> {
        ScopedControl {
            core: ControlCore {
                handle,
                poisoned: RefCell::new(None),
                fact_parts: 0,
            },
            _brand: PhantomData,
        }
    }

    /// Wraps the control clingo owns and hands to an application's `main`.
    ///
    /// Not `unsafe` because `unsafe` is confined to `raw`; the obligations
    /// are the caller's, and the only caller is the `main` trampoline: `handle`
    /// must come from `ControlHandle::borrowed`, the value stays on the
    /// trampoline's own stack and is dropped before clingo frees the control,
    /// and it is lent to user code only as `&mut ScopedControl<'r>` through a
    /// higher-ranked closure bound, so safe code cannot name a second value of
    /// this type or move the borrowed one out. `'r` must never be `'static`.
    pub(crate) fn from_borrowed(handle: ControlHandle) -> ScopedControl<'r> {
        ScopedControl::wrap(handle)
    }
}

impl ControlCore {
    /// The error every method returns once the control is poisoned, if it is.
    pub(crate) fn refusal(&self) -> Result<()> {
        match &*self.poisoned.borrow() {
            None => Ok(()),
            Some(cause) => Err(Error::new(
                ErrorKind::Poisoned,
                format!("the control was poisoned by an earlier error ({cause})"),
            )),
        }
    }

    /// Poisons the control if `err` calls for it (DESIGN S3), and returns it:
    /// by its kind, or whatever its kind when it was marked so, as a failed
    /// close or restore is. The first poisoning cause is the one remembered.
    pub(crate) fn note(&self, err: Error) -> Error {
        match err.poison() {
            Poison::Always => self.poison(&err),
            Poison::Cause(cause) => self.poison_with(cause),
            Poison::Never => {}
            Poison::ByKind => {
                if matches!(
                    err.kind(),
                    ErrorKind::Parse | ErrorKind::Logic | ErrorKind::BadAlloc | ErrorKind::Unknown
                ) {
                    self.poison(&err);
                }
            }
        }
        err
    }

    pub(crate) fn poison(&self, err: &Error) {
        self.poison_with(&err.to_string());
    }

    pub(crate) fn poison_with(&self, cause: &str) {
        self.poisoned
            .borrow_mut()
            .get_or_insert_with(|| cause.to_owned());
    }

    /// Poisons the control after a ground callback or observer callback
    /// panicked during grounding, before the panic resumes
    /// on the caller's thread: the same reason a returned error poisons
    /// unconditionally, since clingo keeps whatever it already ground and
    /// would answer from it silently. Called instead of [`Control::note`]
    /// because a panic has no [`Error`] of its own to poison with.
    pub(crate) fn poison_after_panic(&self, context: &str) {
        self.poison_with(&format!("{context}: a callback panicked during grounding"));
    }

    /// Closes a search or a backend that a forgotten
    /// [`SolveHandle`](crate::SolveHandle) or
    /// [`Backend`](crate::backend::Backend) left open, including one a panic
    /// unwound past (DESIGN S4). A failure to close either poisons the control,
    /// whatever its kind (S3).
    ///
    /// A search closed this way is a leftover, not one this call's own caller
    /// is asking about: an event handler's own recorded error or panic, if
    /// either is set, is discarded along with it, never promoted, the same as
    /// when [`SolveHandle`](crate::SolveHandle)'s or
    /// [`AsyncSolveHandle`](crate::AsyncSolveHandle)'s own `Drop` calls this.
    ///
    /// **Collects the registered observer's own slots right here** with the
    /// same priority over the raw result, and the same panic resumption, that
    /// [`Control::guarded`] and [`Control::with_backend`] already give an
    /// observer failure during grounding: closing a leftover backend can itself
    /// call the observer (`output_atom` and friends, delayed to
    /// `clingo_backend_end`), and every caller of `finish_search` needs that
    /// failure surfaced at the call that actually triggered it, not left to
    /// whichever unrelated later call happens to check the slots next. Every
    /// read-only entry point (through [`Control::observed`]) reaches this too,
    /// not only the mutating ones `guarded`/`guarded_with_events` already
    /// covered on their own before this decision.
    pub(crate) fn finish_search(&self) -> Result<()> {
        let result = self
            .handle
            .close_solve(false)
            .and_then(|()| self.handle.close_backend())
            .and_then(|()| self.handle.close_program_builder());
        if let Some(payload) = self.handle.take_observer_panic() {
            self.poison_after_panic("finishing an earlier operation");
            std::panic::resume_unwind(payload);
        }
        if let Some(err) = self.handle.take_observer_error() {
            return Err(self.note(err.context("finishing an earlier operation").poisoning()));
        }
        result.map_err(|err| self.note(err.context("finishing an earlier operation").poisoning()))
    }

    /// Runs one operation that starts from an idle control: it refuses when
    /// poisoned, closes a leftover search first (S4), and poisons on the errors
    /// that call for it (S3).
    ///
    /// A registered
    /// [`GroundProgramObserver`](crate::observer::GroundProgramObserver)'s own
    /// panic or error, if `f` triggered one, takes priority over whatever `f`
    /// itself returned and poisons unconditionally, exactly as it already does
    /// for [`Control::ground`]/[`Control::ground_with`]. This is every entry
    /// point through which clingo can call the observer outside grounding:
    /// [`Control::solve`], [`Control::solve_yield`],
    /// [`Control::solve_yield_with_events`], [`Control::solve_async`] and
    /// [`Control::solve_async_with_events`] all start a search through this
    /// function, and `end_step` fires when a solve starts, not only when
    /// grounding finishes (see the rustdoc of
    /// [`GroundProgramObserver`](crate::observer::GroundProgramObserver) for
    /// the exact clingo source lines). Every other caller of `guarded` never
    /// reaches a registered observer at all, so this check is a defensive no-op
    /// for them (their slots are always empty).
    pub(crate) fn guarded<T>(
        &mut self,
        context: impl FnOnce() -> String,
        f: impl FnOnce(&mut ControlHandle) -> Result<T>,
    ) -> Result<T> {
        let raw_result = self
            .refusal()
            .and_then(|()| self.finish_search())
            .and_then(|()| f(&mut self.handle));
        // An observer's failure takes priority and always poisons; a
        // propagator's failure is settled after it.
        if let Some(payload) = self.handle.take_observer_panic() {
            self.poison_after_panic(&context());
            std::panic::resume_unwind(payload);
        }
        if let Some(err) = self.handle.take_observer_error() {
            return Err(self.note(err.context(context()).poisoning()));
        }
        self.settle_scripts(raw_result, context)
    }

    /// Reads what a script callback left during the call that just ran:
    /// a panic first, which poisons and resumes; then a returned error, which
    /// takes the place of clingo's own report of the failed callback and
    /// poisons by its kind like any other error (so a user error stays what
    /// the user made it, and never goes through `add`'s `Runtime` to `Parse`
    /// remap). Otherwise the propagators are settled as before.
    pub(crate) fn settle_scripts<T>(
        &self,
        raw_result: Result<T>,
        context: impl FnOnce() -> String,
    ) -> Result<T> {
        if let Some(payload) = self.handle.take_script_panic() {
            self.poison_with(&format!("{}: a script callback panicked", context()));
            std::panic::resume_unwind(payload);
        }
        if let Some(err) = self.handle.take_script_error() {
            return Err(self.note(err.context(context())));
        }
        self.settle_propagators(raw_result, context)
    }

    /// Runs one operation that reads the control through `&self`: as
    /// [`Control::guarded`], for the views that borrow the control shared
    /// (DESIGN S5).
    pub(crate) fn observed<'c, T>(
        &'c self,
        context: impl FnOnce() -> String,
        f: impl FnOnce(&'c ControlHandle) -> Result<T>,
    ) -> Result<T> {
        let raw_result = self
            .refusal()
            .and_then(|()| self.finish_search())
            .and_then(|()| f(&self.handle));
        self.settle_propagators(raw_result, context)
    }

    /// Runs one operation on the open search: as [`Control::guarded`], but
    /// without closing the search first.
    pub(crate) fn searching<T>(
        &mut self,
        context: impl FnOnce() -> String,
        f: impl FnOnce(&mut ControlHandle) -> Result<T>,
    ) -> Result<T> {
        let raw_result = self.refusal().and_then(|()| self.handle.logged_call(f));
        self.settle_propagators(raw_result, context)
    }

    /// As [`Control::guarded`], for a call that might have run a
    /// [`SolveEventHandler`](crate::SolveEventHandler) trampoline: a panic or
    /// an error the handler recorded (peeked through
    /// [`raw::ControlHandle::event_handler_panic`]/[`raw::
    /// ControlHandle::event_handler_error`]) is resumed or returned before
    /// [`Control::note`] ever sees the raw result, so a handler's own error can
    /// never poison the control. No event's failure ever makes the underlying
    /// call return `false` any more (U26, the model event took clingo's own
    /// safe error path before that fix, which left clasp itself inconsistent
    /// after an async parallel search), so this priority mostly matters for a
    /// handler's panic, and for keeping the reported error the handler's own
    /// rather than a less specific one from a later, unrelated read. Only the
    /// raw result the underlying call actually produced, once no handler
    /// failure is on record, goes through the normal by-kind poisoning path. A
    /// plain search records no handler at all, so this behaves exactly as
    /// [`Control::guarded`] for one.
    pub(crate) fn guarded_with_events<T>(
        &mut self,
        context: impl FnOnce() -> String,
        f: impl FnOnce(&mut ControlHandle) -> Result<T>,
    ) -> Result<T> {
        let raw_result = self
            .refusal()
            .and_then(|()| self.finish_search())
            .and_then(|()| f(&mut self.handle));
        self.settle_events(raw_result, context)
    }

    /// As [`Control::searching`], with the same events-aware priority
    /// [`Control::guarded_with_events`] gives a handler's own error or
    /// panic.
    pub(crate) fn searching_with_events<T>(
        &mut self,
        context: impl FnOnce() -> String,
        f: impl FnOnce(&mut ControlHandle) -> Result<T>,
    ) -> Result<T> {
        let raw_result = self.refusal().and_then(|()| self.handle.logged_call(f));
        self.settle_events(raw_result, context)
    }

    /// [`Control::searching_with_events`] for Shared by
    /// [`Control::guarded_with_events`] and [`Control::
    /// searching_with_events`]: a registered observer's own panic or error, if
    /// either is set, is checked first and poisons unconditionally (see
    /// [`Control::guarded`]'s own doc): an observer failure must never be
    /// masked by a handler's own, non-poisoning one. Only once neither is set
    /// does a handler's stored panic or error, if any, take priority over
    /// `raw_result` and bypass [`Control::note`] (poisoning) entirely;
    /// otherwise `raw_result` is poisoned by kind as usual.
    pub(crate) fn settle_events<T>(
        &self,
        raw_result: Result<T>,
        context: impl FnOnce() -> String,
    ) -> Result<T> {
        if let Some(payload) = self.handle.take_observer_panic() {
            self.poison_after_panic(&context());
            std::panic::resume_unwind(payload);
        }
        if let Some(err) = self.handle.take_observer_error() {
            return Err(self.note(err.context(context()).poisoning()));
        }
        if let Some(payload) = self.handle.event_handler_panic() {
            std::panic::resume_unwind(payload);
        }
        if let Some(err) = self.handle.event_handler_error() {
            return Err(err.context(context()));
        }
        self.settle_propagators(raw_result, context)
    }

    /// Combines what clingo (or an earlier check) reported with what a
    /// registered [`Propagator`](crate::propagate::Propagator) recorded, in
    /// priority order: a panic first, since it must resume, never be treated as
    /// a plain error (poisoning first only when the panic came from `init`, a
    /// panic from `propagate`/`undo`/`check`/ `decide` resumes without
    /// poisoning); then a returned callback error (already marked to poison
    /// unconditionally when it came from `init`, or to never poison, whatever
    /// its kind, when it came from `propagate`/`undo`/`check`/`decide`, both in
    /// `raw::propagate::PropagatorSlots::take_error`); otherwise `raw_result`,
    /// poisoned by kind as usual (S3). Mirrors [`Control::settle_events`] and
    /// [`ControlHandle::resolve_ground`]'s own priority order for a ground
    /// callback and an observer.
    ///
    /// Called from every entry point that might have run a propagator callback
    /// ([`Control::guarded`], [`Control::observed`], [`Control::searching`],
    /// and, through [`Control::settle_events`],
    /// [`Control::guarded_with_events`]/[`Control::searching_with_events`]), so
    /// a failure surfaces at the next call regardless of which one that is
    /// (DESIGN S4).
    pub(crate) fn settle_propagators<T>(
        &self,
        raw_result: Result<T>,
        context: impl FnOnce() -> String,
    ) -> Result<T> {
        if let Some((poisons, payload)) = self.handle.take_propagator_panic() {
            if poisons {
                self.poison_with(&format!("{}: a propagator callback panicked", context()));
            }
            std::panic::resume_unwind(payload);
        }
        if let Some(err) = self.handle.take_propagator_error() {
            return Err(self.note(err.context(context())));
        }
        raw_result.map_err(|err| self.note(err.context(context())))
    }

    /// Waits for the open search's result, then closes the search, and
    /// reports both (DESIGN S3, S7): the result if both succeed; the close
    /// error, which poisons, if only the close fails; the error of `get`
    /// otherwise, poisoning with the close error as its cause if the close
    /// failed too. The search is closed on every path.
    ///
    /// A handler's own stored panic or error is resumed or
    /// returned before [`Control::note`] ever sees the result of
    /// `get`/`close`, and never poisons the control. A plain search records
    /// no handler at all, so this behaves exactly as it did before
    /// handlers existed for one.
    pub(crate) fn get_and_close(&mut self) -> Result<SolveResult> {
        let got = self
            .refusal()
            .and_then(|()| self.handle.logged_call(ControlHandle::solve_get))
            .map(SolveResult)
            .map_err(|e| e.context("waiting for the search result"));
        let closed = self
            .handle
            // This call's own search, closed deliberately by `SolveHandle::
            // close`/`AsyncSolveHandle::close`: promote a handler's own
            // error or panic so it reaches the check just below.
            .close_solve(true)
            .map_err(|e| e.context("closing the search"));
        // A panic the logger raised while closing resumes here, not in `Drop`
        // or the next call, and only once the control is poisoned if it must
        // be.
        self.handle.resume_logger_panic();
        if let Some(payload) = self.handle.event_handler_panic() {
            std::panic::resume_unwind(payload);
        }
        if let Some(err) = self.handle.event_handler_error() {
            return Err(err);
        }
        if let Some((poisons, payload)) = self.handle.take_propagator_panic() {
            if poisons {
                self.poison_with("closing the search: a propagator callback panicked");
            }
            std::panic::resume_unwind(payload);
        }
        if let Some(err) = self.handle.take_propagator_error() {
            return Err(self.note(err));
        }
        raw::settle(got, closed).map_err(|e| self.note(e))
    }

    /// Runs one read-only operation on the open search through `&self`, for
    /// calls that only report what has been decided so far and cannot move
    /// the search on or invalidate a lent model: [`SolveHandle::core`] and
    /// [`AsyncSolveHandle::core`].
    ///
    /// Unlike [`Control::searching`] this does not go through
    /// [`ControlHandle::logged_call`], since none of the calls it runs are
    /// documented to log anything; a failure still poisons or not by its
    /// kind, as every other call does (S3).
    ///
    /// [`SolveHandle::core`]: crate::SolveHandle::core
    /// [`AsyncSolveHandle::core`]: crate::AsyncSolveHandle::core
    pub(crate) fn observing_search<T>(
        &self,
        context: impl FnOnce() -> String,
        f: impl FnOnce(&ControlHandle) -> Result<T>,
    ) -> Result<T> {
        let raw_result = self.refusal().and_then(|()| f(&self.handle));
        self.settle_propagators(raw_result, context)
    }
}

impl ScopedControl<'_> {
    /// Adds a program block, as `#program name(parameters).` followed by
    /// `program`.
    ///
    /// The block is parsed now and grounded later by [`Control::ground`] with a
    /// [`Part`] of the same name. A `#script (name) ... #end.` block runs the
    /// registered [`Script::execute`](crate::script::Script::execute) while the
    /// program is parsed; an error from it comes back unchanged in kind and
    /// poisons the control (see [`script`](crate::script)). Program text
    /// without a `#program` directive belongs to `base`, which
    /// [`Control::add_base`] adds to.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Parse`] for a syntax error, with clingo's messages and
    ///   their positions in [`Error::messages`]. It poisons the control;
    /// - [`ErrorKind::Nul`] if any string contains a NUL byte;
    /// - [`ErrorKind::InvalidInput`] if `name` starts with `__clingox_facts_`,
    ///   which is reserved for the parts of [`Control::add_facts`]. It does not
    ///   poison the control;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add("step", &["t"], "p(t).")?;
    /// ctl.ground(&[Part::new("step", &[Symbol::number(1)])?])?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add(&mut self, name: &str, parameters: &[&str], program: &str) -> Result<()> {
        // Refused before clingo sees anything, so it does not poison.
        unreserved(name).map_err(|e| e.context(format!("parsing program `{name}`")))?;
        self.add_block(name, parameters, program)
    }

    /// [`Control::add`] without the check of the reserved prefix, for
    /// [`Control::add_facts`].
    pub(crate) fn add_block(
        &mut self,
        name: &str,
        parameters: &[&str],
        program: &str,
    ) -> Result<()> {
        let context = || format!("parsing program `{name}`");
        let strings = raw::c_str(name).and_then(|name| {
            let parameters = parameters
                .iter()
                .map(|p| raw::c_str(p))
                .collect::<Result<Vec<_>>>()?;
            Ok((name, parameters, raw::c_str(program)?))
        });
        // A NUL byte is caught before clingo sees anything, so it does not
        // poison.
        let (name_c, parameters, program) = strings.map_err(|e| e.context(context()))?;
        self.core.guarded(context, |handle| {
            handle
                .add(&name_c, &parameters, &program)
                // clingo_control_add raises a runtime error only when parsing
                // or checking the program fails (clingo.h:3040-3044).
                .map_err(|e| match e.kind() {
                    ErrorKind::Runtime => e.with_kind(ErrorKind::Parse),
                    _ => e,
                })
        })
    }

    /// Adds program text to the `base` block, as `add("base", &[], program)`.
    ///
    /// # Errors
    ///
    /// As [`Control::add`].
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// ctl.add_base("a. b :- a.")?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_base(&mut self, program: &str) -> Result<()> {
        self.add("base", &[], program)
    }

    /// Grounds program parts, replacing their parameters with the given values.
    ///
    /// A registered
    /// [`GroundProgramObserver`](crate::observer::GroundProgramObserver) or
    /// [`LimitedObserver`](crate::observer::LimitedObserver) can also stop this
    /// call: an error it returns, or a panic inside it, is reported the same
    /// way a failed grounding is, and poisons the control (see below).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Runtime`] if grounding fails, for example on an unsafe
    ///   variable, with clingo's messages in [`Error::messages`]. This does not
    ///   poison the control;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::Logic`], [`ErrorKind::BadAlloc`] or
    ///   [`ErrorKind::Unknown`], which poison the control;
    /// - whatever an observer returned, unchanged, if it stopped grounding.
    ///   This always poisons the control, whatever its kind: clingo keeps
    ///   whatever it already ground and would answer from it silently, which is
    ///   never safe to hand back (a truncated program, wrong on its own terms).
    ///   Recovery means building a new [`Control`]. This includes
    ///   [`ErrorKind::GroundingLimit`] from
    ///   [`GroundingLimit`](crate::observer::GroundingLimit).
    ///
    /// A panic inside an observer callback is caught, poisons the control for
    /// the same reason, and resumes on the calling thread once this call
    /// returns.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn ground(&mut self, parts: &[Part]) -> Result<()> {
        let context = || crate::ground::grounding(parts);
        let raw_parts = crate::ground::raw_parts(parts);
        self.core
            .refusal()
            .and_then(|()| self.core.finish_search())
            .map_err(|err| self.core.note(err.context(context())))?;
        match self.core.handle.ground(&raw_parts) {
            Ok(()) => Ok(()),
            // An observer's own error poisons unconditionally, unlike a
            // callback error and unlike `InvalidInput`'s
            // usual non-poisoning treatment (S3), because clingo keeps the
            // truncated program it already ground and would answer from it
            // silently.
            Err(GroundError::Callback(err)) => {
                Err(self.core.note(err.context(context()).poisoning()))
            }
            Err(GroundError::Clingo(err)) => Err(self.core.note(err.context(context()))),
            Err(GroundError::Panic(payload)) => {
                self.core.poison_after_panic(&context());
                std::panic::resume_unwind(payload)
            }
        }
    }

    /// Solves the grounded program and returns whether it has an answer set.
    ///
    /// The call blocks until the search is finished (DESIGN S7). Each
    /// assumption fixes an atom to true or false for this call only.
    ///
    /// An assumption on an atom that does not occur in the grounding follows
    /// clingo's semantics, in which such an atom is false: assuming it true
    /// makes the program unsatisfiable, and assuming it false changes nothing.
    /// This is what clingo's C++ API does, and it differs from pyclingo, which
    /// drops the assumptions on unknown atoms. [`Assumption`] shows the filter
    /// that gives pyclingo's behaviour.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Runtime`] if the search cannot start or fails;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::Logic`], [`ErrorKind::BadAlloc`] or
    ///   [`ErrorKind::Unknown`], which poison the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a :- not b. b :- not a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let a = Symbol::function("a", &[])?;
    /// assert!(ctl.solve(&[(a, true).into()])?.is_sat());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn solve(&mut self, assumptions: &[Assumption]) -> Result<SolveResult> {
        self.core.guarded(
            || "solving".to_owned(),
            |handle| {
                let literals = assumption_literals(handle, assumptions)?;
                handle.solve(&literals).map(SolveResult)
            },
        )
    }

    /// Reads a file as ordinary program text, exactly as [`Control::add`] reads
    /// a string.
    ///
    /// Exactly the path `-` is not a file: as in clingo, it reads standard
    /// input, and clingox does not open it first. A file literally named `-`
    /// therefore cannot be loaded by that name; use `./-`. Other spellings such
    /// as `-/` are ordinary paths. A failure while reading standard input
    /// poisons the control like any other failure of the read.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Parse`] for a syntax error inside the file, with clingo's
    ///   messages and their positions in [`Error::messages`]. It poisons the
    ///   control, exactly as a parse error from [`Control::add`] does;
    /// - [`ErrorKind::Runtime`] if the file cannot be opened; it does not
    ///   poison the control (a missing file and a file that opens but fails to
    ///   parse are told apart; clingo reports both as the same underlying
    ///   failure);
    /// - any other failure of reading the file, whatever its kind, for example
    ///   a `#script` block of a language that is not registered
    ///   ([`ErrorKind::Runtime`]) or an error a [script](crate::script)
    ///   returned. It poisons the control: clingo keeps the rest of the failed
    ///   file queued and the next `add` or `load` would resume it (U44);
    /// - [`ErrorKind::InvalidInput`] if `file` is not valid UTF-8;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    ///
    /// A file removed between clingox's own check and clingo's read still
    /// leaves the control unable to parse anything again (UPSTREAM-ISSUES U23).
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let file = std::env::temp_dir().join(format!("clingox_doctest_load_{}.lp", std::process::id()));
    /// std::fs::write(&file, "a. b :- a.").unwrap();
    /// let mut ctl = Control::new()?;
    /// ctl.load(&file)?;
    /// ctl.ground(&[Part::base()])?;
    /// assert!(ctl.solve(&[])?.is_sat());
    /// std::fs::remove_file(&file).ok();
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn load(&mut self, file: impl AsRef<Path>) -> Result<()> {
        let path = file.as_ref();
        let context = || format!("loading program file `{}`", path.display());
        // Checked before touching the filesystem, so a poisoned control
        // reports `Poisoned` for a missing file too, rather than the
        // `Runtime` error the open check below would otherwise report first.
        self.core.refusal().map_err(|e| e.context(context()))?;
        let file_c = path_c_string(path).map_err(|e| e.context(context()))?;
        // clingo 5.8.2's own `clingo_control_load` leaves its internal logger
        // permanently in an error state once a file cannot be opened, so
        // every later `add`/`load` on the same control fails too (checked
        // directly against clingo 5.8.2, including through pyclingo).
        // Opening the file from Rust first keeps
        // clingo from ever seeing the failure for this one case, which is
        // exactly the case the acceptance test needs to leave the control
        // usable afterward.
        // Exactly `-` is standard input, clingo's convention, and is not a file
        // to open from here.
        if path.as_os_str() != "-"
            && let Err(io_err) = std::fs::File::open(path)
        {
            return Err(Error::new(
                ErrorKind::Runtime,
                format!("{} could not be opened: {io_err}", path.display()),
            )
            .context(context()));
        }
        self.core.guarded(context, |handle| {
            // Once the file is confirmed open, any failure of the call itself
            // poisons, whatever its kind (as `load_aspif`):
            // clingo keeps the rest of the failed file queued in its parser
            // and the next `add` or `load` would resume it (U44).
            handle
                .load(&file_c)
                .map_err(|e| classify_load_error(e).poisoning())
        })
    }

    /// Loads ground programs written in aspif format.
    ///
    /// This should be called on a control that has not added or grounded
    /// anything yet (clingo.h:3018-3019); clingox does not enforce this itself,
    /// and clingo reports a violation as [`ErrorKind::Runtime`]. If more than
    /// one file is given, they are merged into one file, and only the first
    /// should carry an aspif preamble.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Runtime`] if a file cannot be opened; checked by clingox
    ///   itself before any file reaches clingo, so it does not poison the
    ///   control;
    /// - [`ErrorKind::InvalidInput`] if a path is not valid UTF-8; also checked
    ///   before any file reaches clingo, so it does not poison;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - otherwise, **any failure of the underlying call, once every file has
    ///   been confirmed open, poisons the control, whatever its kind**: clingo
    ///   may have already loaded part of the program before the failure, and
    ///   answers from that truncated program silently otherwise. The kind
    ///   reported is clingo's own: [`ErrorKind::Parse`] for a malformed aspif
    ///   file, with clingo's message in the error text, or
    ///   [`ErrorKind::Logic`], [`ErrorKind::BadAlloc`] or
    ///   [`ErrorKind::Unknown`] for whatever else clingo itself reports.
    ///
    /// A file removed between clingox's own check and clingo's read still
    /// leaves the control unable to parse anything again (UPSTREAM-ISSUES U23).
    ///
    /// **An aspif file naming an atom or literal in range but far beyond the
    /// program's real atoms still costs memory in proportion to its magnitude**
    /// (`docs/dev/UPSTREAM-ISSUES.md` U22, also documented at
    /// [`ProgramLiteral::from_raw`]): unlike a literal built through that
    /// constructor and checked against
    /// [`ProgramLiteral::MAX_MAGNITUDE`](crate::ProgramLiteral::MAX_MAGNITUDE),
    /// `load_aspif` takes a raw file, so this is a risk from the file's own
    /// contents, which clingox never gets a chance to validate first.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let file = std::env::temp_dir()
    ///     .join(format!("clingox_doctest_load_aspif_{}.aspif", std::process::id()));
    /// std::fs::write(&file, "asp 1 0 0\n1 0 1 1 0 0\n4 1 a 1 1\n0\n").unwrap();
    /// let mut ctl = Control::new()?;
    /// ctl.load_aspif([&file])?;
    /// ctl.ground(&[Part::base()])?;
    /// assert!(ctl.solve(&[])?.is_sat());
    /// std::fs::remove_file(&file).ok();
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn load_aspif<I, P>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let context = || "loading an aspif program".to_owned();
        // Checked before touching the filesystem, as in `load`, so a
        // poisoned control reports `Poisoned` for a missing file too.
        self.core.refusal().map_err(|e| e.context(context()))?;
        let paths: Vec<PathBuf> = files.into_iter().map(|p| p.as_ref().to_owned()).collect();
        let files: Vec<CString> = paths
            .iter()
            .map(|p| path_c_string(p))
            .collect::<Result<_>>()
            .map_err(|e| e.context(context()))?;
        // As in `load`: a file clingo cannot open breaks the control for good
        // (UPSTREAM-ISSUES U23), so each one is opened from Rust first.
        for path in &paths {
            if let Err(io_err) = std::fs::File::open(path) {
                return Err(Error::new(
                    ErrorKind::Runtime,
                    format!("{} could not be opened: {io_err}", path.display()),
                )
                .context(context()));
            }
        }
        self.core.guarded(context, |handle| {
            handle.load_aspif(&files).map_err(|err| {
                // Once the files are confirmed open (just above), a failure of
                // the call itself is a genuine clasp-side problem, of whatever
                // kind clingo reports it as -- never only the `Parse` kind
                // `classify_load_aspif_error` recognises. Left unpoisoned,
                // clingo keeps whatever it already loaded before the failure
                // and answers from it silently (a truncated program that solves
                // and drops a fact), the same reasoning `Control::load`'s own
                // parse failures and every observer/ground-callback failure
                // already poison by.
                classify_load_aspif_error(err).poisoning()
            })
        })
    }

    /// Cleans up the grounding using the solver's current assignment: atoms
    /// known false are removed from it and atoms known true become facts,
    /// which can make later grounding steps of multi-shot solving smaller.
    ///
    /// Automatic cleanup after each solve is enabled by default
    /// ([`Control::enable_cleanup`]), so a manual call is only useful with it
    /// disabled, or right before grounding a further part.
    ///
    /// # Errors
    ///
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
    /// ctl.add_base("{a}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.set_enable_cleanup(false)?;
    /// assert!(ctl.solve(&[])?.is_sat());
    /// ctl.cleanup()?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn cleanup(&mut self) -> Result<()> {
        self.core.guarded(
            || "cleaning up the grounding".to_owned(),
            ControlHandle::cleanup,
        )
    }

    /// Enables or disables automatic cleanup after each solve call.
    ///
    /// Cleanup is enabled by default.
    ///
    /// # Errors
    ///
    /// As [`Control::cleanup`].
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// ctl.set_enable_cleanup(false)?;
    /// assert!(!ctl.enable_cleanup());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn set_enable_cleanup(&mut self, enable: bool) -> Result<()> {
        self.core.guarded(
            || "setting whether cleanup is automatic".to_owned(),
            |handle| handle.set_enable_cleanup(enable),
        )
    }

    /// Whether automatic cleanup after each solve call is enabled.
    ///
    /// clingo cannot fail this query, so it stays available even after the
    /// control is poisoned. A leftover search is finished first (S4, for
    /// uniformity with [`Control::is_conflicting`]); a failure to finish it
    /// poisons the control but does not stop this from answering.
    ///
    /// # Examples
    ///
    /// ```
    /// let ctl = clingox::Control::new()?;
    /// assert!(ctl.enable_cleanup());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn enable_cleanup(&self) -> bool {
        let _ = self.core.finish_search();
        self.core.handle.enable_cleanup()
    }

    /// Removes every minimize (`:~`) constraint from the program.
    ///
    /// # Errors
    ///
    /// As [`Control::cleanup`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Outcome, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("{a}. :~ a. [1]")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.remove_minimize()?;
    /// let Outcome::Sat(model, _) = ctl.solve_optimal()? else {
    ///     panic!("the program has a model");
    /// };
    /// assert!(model.cost().is_empty());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn remove_minimize(&mut self) -> Result<()> {
        self.core.guarded(
            || "removing the minimize constraints".to_owned(),
            ControlHandle::remove_minimize,
        )
    }

    /// Replaces or extends the set of projection atoms used with clingo's
    /// `--project` option.
    ///
    /// `append: false` discards any projection atoms set earlier and uses
    /// `atoms` as the new set; `append: true` adds to the existing set. A
    /// symbol that is not a current atom of the grounding is silently
    /// skipped, as [`Control::assign_external`] skips a symbol that is not an
    /// external.
    ///
    /// # Errors
    ///
    /// As [`Control::cleanup`].
    ///
    /// # Panics
    ///
    /// Never on caller input: a symbolic atom's [`ProgramLiteral`] is always
    /// positive, so converting it to the unsigned atom id
    /// `clingo_control_update_project` takes cannot fail.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Symbol};
    ///
    /// let mut ctl = Control::with_args(["--project"])?;
    /// ctl.add_base("{a}. {b}.")?;
    /// ctl.ground(&[clingox::Part::base()])?;
    /// ctl.update_project([Symbol::function("a", &[])?], false)?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn update_project<I>(&mut self, atoms: I, append: bool) -> Result<()>
    where
        I: IntoIterator<Item = Symbol>,
    {
        self.core.guarded(
            || "updating the projection atoms".to_owned(),
            |handle| {
                let mut ids = Vec::new();
                for symbol in atoms {
                    if let Some(literal) = handle.atom_literal(symbol.raw())? {
                        // Literals of symbolic atoms are always positive, so
                        // this conversion cannot fail.
                        let atom =
                            u32::try_from(literal).expect("a symbolic atom's literal is positive");
                        ids.push(atom);
                    }
                }
                handle.update_project(&ids, append)
            },
        )
    }

    /// Whether the program's internal representation is already known to be
    /// conflicting.
    ///
    /// If this is `true`, solving returns unsatisfiable immediately without
    /// searching. Conflicts first have to be detected, though (by unit
    /// propagation during grounding, or during an earlier search), so `false`
    /// never proves the program satisfiable.
    ///
    /// clingo cannot fail this query, so it stays available even after the
    /// control is poisoned. A leftover search is finished first (S4): otherwise
    /// this reads clasp's internal state while a forgotten async search is
    /// still running on another thread, a data race under `ThreadSanitizer`
    /// (`SharedContext::ok`). A failure to finish the leftover search poisons
    /// the control but does not stop this from answering.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a. :- a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// assert!(ctl.is_conflicting());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn is_conflicting(&self) -> bool {
        let _ = self.core.finish_search();
        self.core.handle.is_conflicting()
    }

    /// Enables or disables the enumeration assumption: whether information
    /// learnt from enumeration (cautious or brave consequences, projected
    /// enumeration, optimization, or clauses added during enumeration) is
    /// cleared after each solve call.
    ///
    /// Enabled by default. Disabling it can save a little time in
    /// single-shot solving, or just before the last solve call of a
    /// multi-shot program.
    ///
    /// # Errors
    ///
    /// As [`Control::cleanup`].
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// ctl.set_enable_enumeration_assumption(false)?;
    /// assert!(!ctl.enable_enumeration_assumption());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn set_enable_enumeration_assumption(&mut self, enable: bool) -> Result<()> {
        self.core.guarded(
            || "setting the enumeration assumption".to_owned(),
            |handle| handle.set_enable_enumeration_assumption(enable),
        )
    }

    /// Whether the enumeration assumption is enabled.
    ///
    /// clingo cannot fail this query, so it stays available even after the
    /// control is poisoned. A leftover search is finished first (S4, for
    /// uniformity with [`Control::is_conflicting`]); a failure to finish it
    /// poisons the control but does not stop this from answering.
    ///
    /// # Examples
    ///
    /// ```
    /// let ctl = clingox::Control::new()?;
    /// assert!(ctl.enable_enumeration_assumption());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn enable_enumeration_assumption(&self) -> bool {
        let _ = self.core.finish_search();
        self.core.handle.enable_enumeration_assumption()
    }

    /// The symbol of a `#const name = symbol.` definition, or `None` if no
    /// such constant is defined.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Nul`] if `name` contains a NUL byte;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#const x = 1.")?;
    /// ctl.ground(&[Part::base()])?;
    /// assert_eq!(ctl.get_const("x")?, Some(Symbol::number(1)));
    /// assert_eq!(ctl.get_const("undefined")?, None);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn get_const(&self, name: &str) -> Result<Option<Symbol>> {
        self.core.observed(
            || format!("reading the constant `{name}`"),
            |handle| {
                let name = raw::c_str(name)?;
                if !handle.has_const(&name)? {
                    return Ok(None);
                }
                handle
                    .get_const(&name)
                    .map(|symbol| Some(Symbol::from_clingo(symbol)))
            },
        )
    }
}

/// Where [`SymbolicAtoms`](crate::SymbolicAtoms),
/// [`TheoryAtoms`](crate::TheoryAtoms) and their iterators and items report an
/// error: a [`Control`], with the ordinary poisoning-aware
/// [`Control::refusal`]/[`Control::note`] machinery, or a propagator's `init`
/// call, which needs none of it (a
/// [`PropagateInit`](crate::propagate::PropagateInit) has no `Control` to
/// poison through; the trampoline dispatch poisons the whole `Control` once
/// `init` as a whole returns an error, `raw::propagate`'s own
/// `PropagatorContext`, not once per atom read). The same types are reused for
/// `PropagateInit::symbolic_atoms`/ `theory_atoms`, with a narrower lifetime;
/// this enum is what lets the same field serve both sources without duplicating
/// either type.
#[derive(Clone, Copy)]
pub(crate) enum ErrorSink<'c> {
    Control(&'c ControlCore),
    None,
}

impl ErrorSink<'_> {
    /// As [`Control::refusal`], or `Ok(())` for [`ErrorSink::None`].
    pub(crate) fn refusal(self) -> Result<()> {
        match self {
            ErrorSink::Control(control) => control.refusal(),
            ErrorSink::None => Ok(()),
        }
    }

    /// As [`Control::note`], or `err` unchanged for [`ErrorSink::None`].
    pub(crate) fn note(self, err: Error) -> Error {
        match self {
            ErrorSink::Control(control) => control.note(err),
            ErrorSink::None => err,
        }
    }

    /// As [`Control::observed`], or `f()` run directly for [`ErrorSink::None`].
    pub(crate) fn observed<T>(
        self,
        context: impl FnOnce() -> String,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        match self {
            ErrorSink::Control(control) => control.observed(context, |_| f()),
            ErrorSink::None => f(),
        }
    }
}

/// Converts a file path to a NUL-terminated C string: clingo reads a path as
/// a plain C string, not necessarily UTF-8, but clingox only accepts paths it
/// can also report in an error message, so a non-UTF-8 path is rejected here
/// rather than passed through lossily.
fn path_c_string(path: &Path) -> Result<CString> {
    let text = path.to_str().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("the file path {} is not valid UTF-8", path.display()),
        )
    })?;
    raw::c_str(text)
}

/// `clingo_control_load` reports both a missing file and a genuine parse
/// failure inside an opened file as `clingo_error_runtime` with one logged
/// message; the two are told apart by whether the message carries a
/// location, which only a parse or check failure has.
fn classify_load_error(err: Error) -> Error {
    let is_parse =
        err.kind() == ErrorKind::Runtime && err.messages().iter().any(|m| m.location().is_some());
    if is_parse {
        err.with_kind(ErrorKind::Parse)
    } else {
        err
    }
}

/// `clingo_control_load_aspif` reports a malformed aspif file the other way
/// around from `clingo_control_load`: nothing is logged during the call (so
/// `err.messages()` is empty), and the raw error text itself carries the
/// location (`<file>:<line>:<col>-...: error: aspif error, ...`), checked
/// directly against clingo 5.8.2 through the Python module's C API
/// (`clingo._internal`, 2026-09-27). `Control::load_aspif` already opens
/// every file itself before calling clingo (U23), so by the time clingo's
/// own call fails, a `Runtime` error here is a genuine aspif-format problem
/// in an opened file, not a missing one: `Message::from_clingo` is reused
/// only to reach `Location::parse_prefix`'s parsing of that raw text, the
/// same rule `classify_load_error` applies to a logged message.
fn classify_load_aspif_error(err: Error) -> Error {
    let is_parse = err.kind() == ErrorKind::Runtime
        && Message::from_clingo(MessageCode::Other, err.raw_message())
            .location()
            .is_some();
    if is_parse {
        err.with_kind(ErrorKind::Parse)
    } else {
        err
    }
}

/// The prefix of the program parts [`Control::add_facts`] creates. Users may
/// not name a part with it, so that no user part collides with one of them and
/// none of them is grounded twice.
pub(crate) const FACTS_PREFIX: &str = "__clingox_facts_";

/// Refuses a part name with the prefix reserved for `add_facts`.
fn unreserved(name: &str) -> Result<()> {
    if name.starts_with(FACTS_PREFIX) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "the part name `{name}` starts with `{FACTS_PREFIX}`, which is reserved for \
                 the parts of `Control::add_facts`"
            ),
        ));
    }
    Ok(())
}

/// The program literals for assumptions on atoms, as clingo's C++ API makes
/// them (clingo.hh:4271-4286).
pub(crate) fn assumption_literals(
    handle: &mut ControlHandle,
    assumptions: &[Assumption],
) -> Result<Vec<i32>> {
    let mut literals = Vec::with_capacity(assumptions.len());
    for assumption in assumptions {
        match assumption.0 {
            Inner::Symbol(symbol, value) => match (handle.atom_literal(symbol.raw())?, value) {
                (Some(literal), true) => literals.push(literal),
                (Some(literal), false) => literals.push(-literal),
                // The atom does not exist, so it is false. Assuming it true is
                // a contradiction, expressed as a literal and its negation.
                (None, true) => literals.extend([1, -1]),
                (None, false) => {}
            },
            // The literal's own sign is the truth value:
            // clingo takes assumptions directly as signed literals
            // (clingo.h:3092), so no lookup is needed.
            Inner::Literal(literal) => literals.push(literal.get()),
        }
    }
    Ok(literals)
}

impl fmt::Debug for ScopedControl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Control");
        match &*self.core.poisoned.borrow() {
            Some(cause) => s
                .field("state", &format_args!("poisoned"))
                .field("cause", cause),
            // Only a forgotten handle leaves a search open while the control
            // itself can be reached; the next call closes it (S4).
            None if self.core.handle.solve_is_active() => {
                s.field("state", &format_args!("solving"))
            }
            None => s.field("state", &format_args!("idle")),
        };
        s.finish_non_exhaustive()
    }
}

/// A program part to ground: a block name and values for its parameters.
///
/// # Examples
///
/// ```
/// use clingox::{Part, Symbol};
///
/// let base = Part::base();
/// let step = Part::new("step", &[Symbol::number(3)])?;
/// assert_eq!(step.to_string(), "step(3)");
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Part {
    name: CString,
    parameters: Vec<Symbol>,
}

impl Part {
    /// A part with the given name and parameter values.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Nul`] if `name` contains a NUL byte;
    /// - [`ErrorKind::InvalidInput`] if `name` starts with `__clingox_facts_`,
    ///   which is reserved for the parts of [`Control::add_facts`]. Since no
    ///   `Part` can have such a name, [`Control::ground`] never grounds one of
    ///   those parts again.
    pub fn new(name: &str, parameters: &[Symbol]) -> Result<Part> {
        unreserved(name).map_err(|e| e.context("creating a program part"))?;
        Part::reserved(name, parameters)
    }

    /// [`Part::new`] without the check of the reserved prefix, for
    /// [`Control::add_facts`].
    pub(crate) fn reserved(name: &str, parameters: &[Symbol]) -> Result<Part> {
        Ok(Part {
            name: raw::c_str(name).map_err(|e| e.context("creating a program part"))?,
            parameters: parameters.to_vec(),
        })
    }

    /// The `base` part without parameters, where program text without a
    /// `#program` directive goes.
    #[must_use]
    pub fn base() -> Part {
        Part {
            name: c"base".to_owned(),
            parameters: Vec::new(),
        }
    }
}

impl Part {
    /// The name and parameters as clingo takes them.
    pub(crate) fn as_raw(&self) -> (&std::ffi::CStr, &[Symbol]) {
        (self.name.as_c_str(), self.parameters.as_slice())
    }
}

impl fmt::Display for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name.to_string_lossy())?;
        if let Some((first, rest)) = self.parameters.split_first() {
            write!(f, "({first}")?;
            for parameter in rest {
                write!(f, ",{parameter}")?;
            }
            f.write_str(")")?;
        }
        Ok(())
    }
}

/// An assumption for [`Control::solve`]: an atom or a program literal that
/// must be true or false for one solve call.
///
/// It is made either from a `(Symbol, bool)` pair (`(atom, true)` requires
/// the atom to be true, `(atom, false)` requires it to be false) or from a
/// [`ProgramLiteral`]: the literal's own sign is the truth value, so `lit` and
/// `lit.negate()` (or `-lit`) require the opposite truth values of the same
/// atom, matching clingo's own convention.
///
/// # An atom the grounding does not have
///
/// clingo's C API, and so clingox, treats an atom that does not occur in the
/// grounding as false: assuming it **true** makes the solve unsatisfiable, and
/// assuming it false changes nothing. pyclingo behaves differently: its
/// `solve(assumptions=...)` drops the assumptions on unknown atoms, so the same
/// call there is satisfiable. A port from pyclingo that relies on that has to
/// filter, as below, using [`SymbolicAtoms::find`](crate::SymbolicAtoms::find).
///
/// # Examples
///
/// ```
/// use clingox::{Assumption, Symbol};
///
/// let a = Symbol::function("a", &[])?;
/// let assumption = Assumption::from((a, false));
/// # Ok::<(), clingox::Error>(())
/// ```
///
/// Assuming an unknown atom, and pyclingo's behaviour by a filter:
///
/// ```
/// use clingox::{Assumption, Control, Part, Symbol};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("{a}.")?;
/// ctl.ground(&[Part::base()])?;
/// let a = Symbol::function("a", &[])?;
/// let unknown = Symbol::function("nosuch", &[])?;
///
/// // clingo: `nosuch` is false, so assuming it true has no model.
/// assert!(!ctl.solve(&[(a, true).into(), (unknown, true).into()])?.is_sat());
///
/// // pyclingo drops what the grounding does not know:
/// let atoms = ctl.symbolic_atoms()?;
/// let mut known = Vec::new();
/// for pair in [(a, true), (unknown, true)] {
///     if atoms.find(pair.0)?.is_some() {
///         known.push(Assumption::from(pair));
///     }
/// }
/// drop(atoms);
/// assert!(ctl.solve(&known)?.is_sat());
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Assumption(Inner);

/// The representation is private so that program literals could be added as a
/// second form later without a breaking change; a program-literal form now
/// exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Inner {
    Symbol(Symbol, bool),
    Literal(ProgramLiteral),
}

impl From<(Symbol, bool)> for Assumption {
    fn from((symbol, value): (Symbol, bool)) -> Assumption {
        Assumption(Inner::Symbol(symbol, value))
    }
}

impl From<ProgramLiteral> for Assumption {
    /// The literal's own sign is the truth value: a negative literal requires
    /// its atom false. clingo takes assumptions directly as signed literals
    /// (clingo.h:3092), so this needs no lookup, unlike the `(Symbol, bool)`
    /// form.
    ///
    /// The literal should name an atom of the program. clingo checks little
    /// here: a condition id from
    /// [`TheoryElement::condition_id`](crate::TheoryElement::condition_id),
    /// which is not an atom's literal, is accepted without an error as a
    /// positive assumption and refused as a negative one.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Assumption, Control, Part, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let a = Symbol::function("a", &[])?;
    /// let literal = ctl.symbolic_atoms()?.find(a)?.expect("a is an atom").literal();
    /// let assumption: Assumption = literal.into();
    /// assert!(ctl.solve(&[assumption])?.is_sat());
    /// assert!(ctl.solve(&[(-literal).into()])?.is_unsat());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    fn from(literal: ProgramLiteral) -> Assumption {
        Assumption(Inner::Literal(literal))
    }
}

/// The result of a search.
///
/// A finished search is satisfiable, unsatisfiable or unknown (when it was
/// interrupted before it could tell). [`Display`](fmt::Display) prints it as
/// clingo does: `SATISFIABLE`, `UNSATISFIABLE` or `UNKNOWN`.
///
/// **An interrupted result is never conclusive.** clingo 5.8.2 can report an
/// interrupted search as unsatisfiable and exhausted even when the program
/// has answer sets: clasp computes the result from how far the search got,
/// and a search stopped at its very start looks complete to it (for
/// `{a;b}.`, about 1 in 1,600 early cancels, and 1 in 14 interrupts from
/// another thread, reproduced with the Python module). So once
/// [`is_interrupted`](Self::is_interrupted) is true,
/// [`is_unsat`](Self::is_unsat) and [`is_exhausted`](Self::is_exhausted)
/// return `false` and [`is_unknown`](Self::is_unknown) returns `true`, unless a
/// model was found: [`is_sat`](Self::is_sat) stays reliable, because clasp
/// only sets it for a model it has seen. clingo's own flags remain available
/// through [`clingo_flags`](Self::clingo_flags).
///
/// The type is `#[must_use]`, because a solve whose answer is dropped is
/// usually a mistake, and the `?` on a `Result` does not silence the warning
/// for the value inside. When the call is made for its effect, such as
/// [`SolveHandle::close`](crate::SolveHandle::close) after the models have been
/// read, say so with `let _ = handle.close()?;`.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("a :- not a.")?;
/// ctl.ground(&[Part::base()])?;
/// let result = ctl.solve(&[])?;
/// assert!(result.is_unsat());
/// assert_eq!(result.to_string(), "UNSATISFIABLE");
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct SolveResult(pub(crate) SolveOutcome);

impl SolveResult {
    /// Whether an answer set was found. This holds for an interrupted search
    /// too: clasp sets it only for a model it found.
    pub fn is_sat(&self) -> bool {
        self.0.satisfiable
    }

    /// Whether the program was proven to have no answer set.
    ///
    /// Always `false` for an interrupted search, whatever clingo reported,
    /// because clingo 5.8.2 can report an interrupted search of a satisfiable
    /// program as unsatisfiable (see [`SolveResult`]).
    pub fn is_unsat(&self) -> bool {
        self.0.unsatisfiable && !self.0.interrupted
    }

    /// Whether the search ended without deciding either: neither a model was
    /// found nor unsatisfiability proven. It is `true` for every interrupted
    /// search that found no model.
    pub fn is_unknown(&self) -> bool {
        !self.is_sat() && !self.is_unsat()
    }

    /// Whether the search space was fully explored.
    ///
    /// Always `false` for an interrupted search, for the reason given for
    /// [`is_unsat`](Self::is_unsat).
    pub fn is_exhausted(&self) -> bool {
        self.0.exhausted && !self.0.interrupted
    }

    /// Whether the search was interrupted: by an
    /// [`InterruptHandle`](crate::InterruptHandle), a timeout, a cancel, or
    /// by leaving a model loop early.
    pub fn is_interrupted(&self) -> bool {
        self.0.interrupted
    }

    /// clingo's own result bitset (`clingo_solve_result_bitset_t`), unchanged:
    /// `1` satisfiable, `2` unsatisfiable, `4` exhausted, `8` interrupted.
    ///
    /// After an interrupt its unsatisfiable and exhausted bits are not
    /// trustworthy (see [`SolveResult`]); the other methods correct for that,
    /// and this one is only for comparing with clingo itself.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a :- not a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// // Unsatisfiable and exhausted.
    /// assert_eq!(ctl.solve(&[])?.clingo_flags(), 2 | 4);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn clingo_flags(&self) -> u32 {
        self.0.to_bits()
    }
}

impl fmt::Display for SolveResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.is_sat() {
            "SATISFIABLE"
        } else if self.is_unsat() {
            "UNSATISFIABLE"
        } else {
            "UNKNOWN"
        })
    }
}

impl fmt::Debug for SolveResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SolveResult({self}")?;
        if self.is_exhausted() {
            f.write_str(", exhausted")?;
        }
        if self.is_interrupted() {
            f.write_str(", interrupted")?;
            if self.0.unsatisfiable || self.0.exhausted {
                // clingo's own claim, which the methods do not trust here.
                write!(f, ", clingo flags {}", self.clingo_flags())?;
            }
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(bits: u32) -> SolveResult {
        SolveResult(SolveOutcome::from_bits(bits))
    }

    #[test]
    fn an_interrupted_result_is_never_unsat_or_exhausted() {
        // What clingo 5.8.2 reports now and then for `{a;b}.` interrupted at
        // its start: unsatisfiable, exhausted and interrupted.
        let claimed = result(2 | 4 | 8);
        assert!(claimed.is_interrupted());
        assert!(!claimed.is_unsat());
        assert!(!claimed.is_exhausted());
        assert!(claimed.is_unknown());
        assert!(!claimed.is_sat());
        assert_eq!(claimed.to_string(), "UNKNOWN");
        assert_eq!(claimed.clingo_flags(), 2 | 4 | 8, "clingo's bits are kept");
        assert!(format!("{claimed:?}").contains("clingo flags 14"));
    }

    /// clingo's flags show when either untrusted flag is set.
    #[test]
    fn debug_shows_clingos_flags_for_either_contradiction() {
        let unsat = format!("{:?}", result(2 | 8));
        assert!(unsat.contains("clingo flags 10"), "{unsat}");
        let exhausted = format!("{:?}", result(4 | 8));
        assert!(exhausted.contains("clingo flags 12"), "{exhausted}");
        let plain = format!("{:?}", result(8));
        assert!(!plain.contains("clingo flags"), "{plain}");
    }

    fn is_poisoned_by(control: &Control, cause: &str) -> bool {
        let shown = format!("{control:?}");
        shown.contains("poisoned") && shown.contains(cause)
    }

    /// How the result of waiting for a
    /// search and the result of closing it combine, with the close failure
    /// injected, since clingo cannot be made to fail a close.
    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn a_failed_close_poisons_even_when_get_failed() {
        let get = || Error::new(ErrorKind::Runtime, "get failed");
        let close = || Error::new(ErrorKind::Runtime, "close failed");

        // Both fail: `get`'s error, and the close error as the cause.
        let control = Control::new().unwrap();
        let err = control
            .core
            .note(raw::settle::<()>(Err(get()), Err(close())).unwrap_err());
        assert_eq!(err.to_string(), "get failed");
        assert!(is_poisoned_by(&control, "close failed"), "{control:?}");

        // Only the close fails: its error, which poisons whatever its kind.
        let control = Control::new().unwrap();
        let err = control
            .core
            .note(raw::settle(Ok(()), Err(close())).unwrap_err());
        assert_eq!(err.to_string(), "close failed");
        assert!(is_poisoned_by(&control, "close failed"), "{control:?}");

        // Only `get` fails: its kind decides, and a runtime error does not
        // poison.
        let control = Control::new().unwrap();
        let err = control
            .core
            .note(raw::settle::<()>(Err(get()), Ok(())).unwrap_err());
        assert_eq!(err.to_string(), "get failed");
        assert!(!format!("{control:?}").contains("poisoned"));

        assert_eq!(raw::settle(Ok(7), Ok(())).unwrap(), 7);
    }

    /// `note`'s dispatch on `Poison::Never` (a
    /// `propagate`/`undo`/`check`/`decide` error, tagged by `raw::propagate::
    /// PropagatorSlots::take_error`) never poisons, whatever the error's own
    /// kind, unlike the `ByKind` default it overrides for exactly the four
    /// kinds that call for it otherwise.
    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn note_never_poisons_an_excused_error_whatever_its_kind() {
        for kind in [
            ErrorKind::Parse,
            ErrorKind::Logic,
            ErrorKind::BadAlloc,
            ErrorKind::Unknown,
        ] {
            let control = Control::new().unwrap();
            let err = control.core.note(Error::new(kind, "m").excused());
            assert_eq!(err.kind(), kind, "the kind is unchanged");
            assert!(
                !format!("{control:?}").contains("poisoned"),
                "an excused {kind:?} error must not poison ({control:?})"
            );
        }
    }

    #[test]
    fn an_interrupted_result_with_a_model_stays_satisfiable() {
        let found = result(1 | 8);
        assert!(found.is_sat() && !found.is_unknown() && !found.is_exhausted());
        assert_eq!(
            format!("{found:?}"),
            "SolveResult(SATISFIABLE, interrupted)"
        );
    }

    #[test]
    fn an_uninterrupted_result_is_read_as_clingo_reports_it() {
        for bits in [1, 1 | 4, 2 | 4, 0] {
            let plain = result(bits);
            assert_eq!(plain.is_sat(), bits & 1 != 0, "{bits}");
            assert_eq!(plain.is_unsat(), bits & 2 != 0, "{bits}");
            assert_eq!(plain.is_exhausted(), bits & 4 != 0, "{bits}");
            assert_eq!(plain.clingo_flags(), bits);
        }
    }
}

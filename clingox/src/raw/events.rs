//! The composed `clingo_solve_event_callback_t` trampoline for a search that
//! installs a user [`SolveEventHandler`].
//!
//! `clingo_control_solve` accepts exactly one `notify`/`data` pair
//! (clingo.h:3089-3092); every search clingox starts already installs one,
//! `solve_event::<SolveSync>` (`raw::interrupt`), whose only job is to end
//! the interrupt phase on the finish event (S13). A search that also wants a
//! user handler cannot register a second one, so [`solve_event_with_handler`]
//! is one trampoline that does both jobs, in a fixed order: the interrupt
//! bookkeeping always runs first, unconditionally, whatever the user handler
//! does or will do.
//!
//! **`goon` is not the same channel as an error**, and **the trampoline never
//! returns `false` to clingo, for any event, including the model event.**
//! clingo's own internal event handler (`control.cc:1988-2020`, see
//! `docs/dev/UPSTREAM-ISSUES.md` U25) calls
//! `clingo_terminate` -- an unconditional, unwind-free `std::_Exit(1)` -- if
//! the trampoline ever returns `false` for the unsat, statistics or finish
//! event, so those three never could. The model event technically has a
//! safe `false` return (clingo's own `throw ClingoError()`, an ordinary
//! exception, `control.cc:1993-1997`), but taking it leaves clasp itself
//! inconsistent when the search is an async parallel one (also the
//! blocking `solve_with_events`, which runs in async mode): the next update
//! after such a search reads out of bounds inside clasp (U26).
//! So every event, including the model event, is handled the same way: an
//! `Err` or a panic from `on_model`, `on_unsat`, `on_statistics` or
//! `on_finish` is stored in the same first-writer-wins slot every other
//! trampoline uses (S8), `*goon` is set to `false` (the same bit a user
//! `Break` sets), and the trampoline always returns `true`. The API method
//! that owns the call checks the slot afterwards and reports the stored
//! error or resumes the stored panic instead of whatever the underlying C
//! call returned, since a `*goon = false` stop looks like an ordinary
//! interrupt to clingo itself.

use std::any::Any;
use std::ffi::c_void;
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};

use clingox_sys as ffi;

use super::control::SolveOutcome;
use super::interrupt::SolveSync;
use super::raw_slice;
use super::trampoline::{PanicSlot, Slot, guard};
use crate::error::Error;
use crate::model::ExtendableModel;
use crate::solve_events::SolveEventHandler;
use crate::stats::MutableStatistics;

/// What the composed trampoline's data pointer points to for a search of type
/// `H`: the existing interrupt-phase sink (`sync`) plus the user's handler,
/// kept alive from the moment the search starts until it is closed on every
/// path ("handler storage", the yield and async handles, and the body of
/// `Control::solve_with_events`, own it).
///
/// **Decision J: calls into the handler are serialised**, through the `Mutex`,
/// exactly as `Capture` serialises calls into the user's logger. clasp in fact
/// already serialises every event this trampoline can see on its own: model and
/// unsat events run under `shared_->modelM` (`clasp/src/parallel_solve.cpp`,
/// `ParallelSolve::commitModel`/ `commitUnsat`, "models must be processed
/// sequentially"), and the finish event (with the statistics event immediately
/// before it) fires once per step from whichever single thread completes it
/// (S13). The mutex is therefore not load-bearing against genuine concurrent
/// calls from clasp -- there are none to guard against -- but it is the one,
/// simple, audited reason `&mut H` is sound to hand the user's methods, rather
/// than a conclusion that depends on tracing every path through clasp's event
/// delivery and hoping a future clasp version keeps them serialised. A poisoned
/// mutex is recovered with `into_inner`: the lock is only ever held across the
/// one handler call that could panic, and a panic from it is caught by
/// [`guard`] before the trampoline returns, so the poison, if any, reflects
/// exactly the panic already recorded in `panic`.
pub(crate) struct EventData<H> {
    sync: Arc<SolveSync>,
    handler: Mutex<H>,
    error: Slot<Error>,
    panic: PanicSlot,
}

impl<H: SolveEventHandler + Send> EventData<H> {
    pub(crate) fn new(sync: Arc<SolveSync>, handler: H) -> Self {
        EventData {
            sync,
            handler: Mutex::new(handler),
            error: Slot::default(),
            panic: PanicSlot::default(),
        }
    }
}

/// Type-erased access to a registered event handler's slots, so the safe
/// layer can check for a stored error or panic without naming the concrete
/// `H` (mirrors `raw::observer::ObserverSlots`).
pub(crate) trait EventHandlerSlots {
    /// A copy of the recorded callback error, if any (first writer wins, S8): a
    /// *peek*, not a take: the handler's own error is reported on every later
    /// call on the same search, not only the first one to read this, so the
    /// slot itself is never emptied by reading it -- only
    /// [`Error::repeatable_copy`] leaves it, and the same stored error is still
    /// there, unchanged, for the next caller. The handler itself is never
    /// re-entered once this is set, independently of whether it has been read
    /// (`raw::events::conclude`'s own `error.is_set()` check).
    fn peek_error(&self) -> Option<Error>;
    /// Takes the recorded panic payload, if any. Unlike
    /// [`EventHandlerSlots::peek_error`], this *is* a take: a caught panic
    /// is resumed at most once, since [`std::panic::resume_unwind`] itself
    /// unwinds the caller's own thread from that point on, so there is no
    /// "later call" on the same search left to report it to again.
    fn take_panic(&self) -> Option<Box<dyn Any + Send>>;
}

impl<H> EventHandlerSlots for EventData<H> {
    fn peek_error(&self) -> Option<Error> {
        self.error.peek_with(Error::repeatable_copy)
    }

    fn take_panic(&self) -> Option<Box<dyn Any + Send>> {
        self.panic.take()
    }
}

/// The unsat event's payload: `Potassco::Span<int64_t>`
/// (`clasp/libpotassco/potassco/basic_types.h:108-116`), a pointer to the
/// first element and a size, verified directly against the vendored source
/// (`control.cc:1998-2003`: `bool on_unsat(Potassco::Span<int64_t>
/// optimization) override { ... cb_(clingo_solve_event_type_unsat,
/// &optimization, data_, &goon); ... }`).
#[repr(C)]
struct RawSpan {
    first: *const i64,
    size: usize,
}

/// The outcome of one dispatched event, translated from
/// [`ControlFlow`] for the trampoline's own `goon`/return-value logic below.
enum EventOutcome {
    Continue,
    Break,
}

/// Runs the user's handler for one event, behind the mutex, and
/// translates the payload for the matching [`SolveEventHandler`] method.
///
/// # Safety
///
/// `event` must be the pointer clingo passed for this call, of the shape
/// its `kind` implies (clingo.h:2549-2550; control.cc:1988-2020): a live,
/// non-const `clingo_model_t *` for the model kind; a live `RawSpan` for the
/// unsat kind; a live `[clingo_statistics_t *; 2]` (step, then accumulated,
/// both non-const) for the statistics kind; a live
/// `clingo_solve_result_bitset_t` for the finish kind.
unsafe fn dispatch<H: SolveEventHandler + Send>(
    context: &EventData<H>,
    kind: ffi::clingo_solve_event_type_t,
    event: *mut c_void,
) -> Result<EventOutcome, Error> {
    let mut handler = context
        .handler
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let flow = if kind == ffi::clingo_solve_event_type_model {
        // SAFETY: the caller's contract above: `event` is a live, non-const
        // `clingo_model_t *` for the duration of this call.
        let model = unsafe { super::model::extendable_model(event.cast()) };
        let mut model = ExtendableModel::from_raw(model);
        handler.on_model(&mut model)?
    } else if kind == ffi::clingo_solve_event_type_unsat {
        // SAFETY: the caller's contract above: `event` points to a live
        // `RawSpan` for this call.
        let span = unsafe { &*event.cast::<RawSpan>() };
        // SAFETY: `span.first` is null (with `span.size == 0`) or points to
        // `span.size` valid `i64`s for this call (the caller's contract).
        let bounds = unsafe { raw_slice(span.first, span.size) };
        handler.on_unsat(bounds)?
    } else if kind == ffi::clingo_solve_event_type_statistics {
        // SAFETY: the caller's contract above: `event` points to a live
        // `[clingo_statistics_t *; 2]` for this call, index 0 the per-step
        // tree and index 1 the accumulated tree (control.cc:2009-2010).
        let trees = unsafe { *event.cast::<[*mut ffi::clingo_statistics_t; 2]>() };
        // SAFETY: both pointers are live, non-const statistics objects for
        // this call (the caller's contract).
        let step_raw = unsafe { super::stats::MutableStats::new(trees[0]) }?;
        // SAFETY: as above.
        let accu_raw = unsafe { super::stats::MutableStats::new(trees[1]) }?;
        let mut step = MutableStatistics::from_raw(step_raw);
        let mut accumulated = MutableStatistics::from_raw(accu_raw);
        handler.on_statistics(&mut step, &mut accumulated)?
    } else if kind == ffi::clingo_solve_event_type_finish {
        // SAFETY: the caller's contract above: `event` points to a live
        // `clingo_solve_result_bitset_t` for this call
        // (`Gringo::SolveResult`'s only member is this same bitset,
        // `control.hh:69-79`; control.cc:2013-2015).
        let bits = unsafe { *event.cast::<ffi::clingo_solve_result_bitset_t>() };
        let result = crate::control::SolveResult(SolveOutcome::from_bits(bits));
        handler.on_finish(result)?
    } else {
        // A solve event kind this version of clingo.h does not document.
        // Nothing to dispatch, and nothing wrong: keep going.
        ControlFlow::Continue(())
    };
    Ok(match flow {
        ControlFlow::Continue(()) => EventOutcome::Continue,
        ControlFlow::Break(()) => EventOutcome::Break,
    })
}

/// Ends the call for an event that failed (an `Err`, a panic, or an earlier
/// event on the same search that already recorded one): `*goon = false`, return
/// `true`, for every event, including the model event (U26). No event may ever
/// see a `false` return: the other three because clingo's `clingo_terminate`s
/// the process for it (U25), the model event because that path leaves clasp
/// itself inconsistent for an async parallel search (U26).
fn stop(goon: *mut bool) -> bool {
    // SAFETY: `goon` is a valid out-pointer for this call (the trampoline's
    // own caller's contract, clingo.h:2559).
    unsafe { *goon = false };
    true
}

/// The outer control-flow every event follows: an earlier failure on this
/// same search stops at once, without re-entering the (possibly poisoned)
/// handler mutex (S8); otherwise `run` (dispatch to the user's handler)
/// executes inside [`guard`], and its outcome, an error it returned, or a
/// caught panic is translated to clingo's `goon`/return-value protocol via
/// [`stop`].
///
/// Split out from [`solve_event_with_handler`] so the unit tests below can
/// pin this mapping directly for every event kind and outcome, including the
/// statistics kind, whose own dispatch calls into clingo
/// (`MutableStats::new`) to build its two views and so cannot run under
/// Miri; the tests instead build those views with
/// [`super::stats::MutableStats::for_test`] and call `handler.on_statistics`
/// through the same `conclude` this function itself uses for every other
/// kind. No event dispatched through `conclude` ever calls into clingo's
/// error state (S8's report/resume happens later, on the caller's thread,
/// through the context's own slots), so unlike the observer and ground
/// trampolines this one needs no `ErrorState` generic to run under Miri.
fn conclude(
    error: &Slot<Error>,
    panic: &PanicSlot,
    goon: *mut bool,
    run: impl FnOnce() -> Result<EventOutcome, Error>,
) -> bool {
    if panic.is_set() || error.is_set() {
        return stop(goon);
    }
    match guard(panic, run) {
        Some(Ok(EventOutcome::Continue)) => true,
        Some(Ok(EventOutcome::Break)) => {
            // SAFETY: `goon` is a valid out-pointer for this call.
            unsafe { *goon = false };
            true
        }
        Some(Err(err)) => {
            error.store(err);
            stop(goon)
        }
        // A panic, already recorded in `panic` by `guard`.
        None => stop(goon),
    }
}

/// The composed `clingo_solve_event_callback_t` for a search that installs
/// both the internal interrupt-phase sink and a user [`SolveEventHandler`]
/// of type `H`. See the module documentation for the full contract.
///
/// # Safety
///
/// `data` must point to a live [`EventData<H>`] for the duration of the
/// call (the caller's contract, upheld by every function in `raw::solve`
/// that installs this trampoline).
pub(crate) unsafe extern "C" fn solve_event_with_handler<H: SolveEventHandler + Send>(
    kind: ffi::clingo_solve_event_type_t,
    event: *mut c_void,
    data: *mut c_void,
    goon: *mut bool,
) -> bool {
    // SAFETY: the caller's contract above.
    let context = unsafe { &*data.cast::<EventData<H>>().cast_const() };

    // S13: the interrupt-phase-ending step runs first and unconditionally,
    // before anything below, whatever the user handler does or will do (the
    // single most important ordering rule in this module). Plain searches
    // without a user handler keep using `solve_event::<SolveSync>` unchanged;
    // this is the only place that composes the two.
    if kind == ffi::clingo_solve_event_type_finish {
        context.sync.finished();
    }

    conclude(&context.error, &context.panic, goon, || {
        // SAFETY: the caller's contract above: `event` is valid for the
        // duration of this call, of the shape `kind` implies.
        unsafe { dispatch(context, kind, event) }
    })
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;

    use super::super::stats::MutableStats;
    use super::*;
    use crate::error::{ErrorKind, Result};

    /// What a handler does for one event, configured per test.
    #[derive(Clone, Copy, Default)]
    enum Reaction {
        #[default]
        Continue,
        Break,
        Fail,
        Panic,
    }

    impl Reaction {
        fn flow(self) -> Result<ControlFlow<()>> {
            match self {
                Reaction::Continue => Ok(ControlFlow::Continue(())),
                Reaction::Break => Ok(ControlFlow::Break(())),
                Reaction::Fail => Err(Error::new(
                    ErrorKind::Runtime,
                    "the handler fails on purpose",
                )),
                Reaction::Panic => panic!("the handler panics on purpose"),
            }
        }
    }

    /// A handler whose reaction to each event is set independently, and
    /// which counts how many events actually reached it (S8: a later event
    /// on a search that already failed must never re-enter it).
    #[derive(Default)]
    struct Reacts {
        model: Reaction,
        unsat: Reaction,
        statistics: Reaction,
        finish: Reaction,
        calls: u32,
    }

    impl SolveEventHandler for Reacts {
        fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
            self.calls += 1;
            self.model.flow()
        }

        fn on_unsat(&mut self, _lower_bound: &[i64]) -> Result<ControlFlow<()>> {
            self.calls += 1;
            self.unsat.flow()
        }

        fn on_statistics(
            &mut self,
            _step: &mut MutableStatistics<'_>,
            _accumulated: &mut MutableStatistics<'_>,
        ) -> Result<ControlFlow<()>> {
            self.calls += 1;
            self.statistics.flow()
        }

        fn on_finish(&mut self, _result: crate::control::SolveResult) -> Result<ControlFlow<()>> {
            self.calls += 1;
            self.finish.flow()
        }
    }

    /// A `SolveSync` with nothing behind its control pointer: every method
    /// these tests call (`starting`, `started`, `finished`, `is_running`)
    /// only touches the phase machine, never the pointer (only `interrupt`
    /// while `Running` does, through `deliver`, which none of these tests
    /// reach).
    fn sync() -> Arc<SolveSync> {
        Arc::new(SolveSync::new(NonNull::dangling()))
    }

    /// A `sync` already in its `Running` phase, so `finished()`'s effect
    /// (S13) is observable through `is_running()`.
    fn running_sync() -> Arc<SolveSync> {
        let sync = sync();
        sync.starting(false);
        sync.started();
        sync
    }

    /// `Model` is zero-sized with alignment 1 (`raw::model`'s own static
    /// assert), so a dangling, well-aligned, non-null pointer is a live
    /// model for every purpose these tests exercise: none of them reads
    /// through it.
    fn model_event() -> *mut c_void {
        NonNull::<ffi::clingo_model_t>::dangling().as_ptr().cast()
    }

    /// An empty unsat payload (`RawSpan { first: null, size: 0 }`), valid
    /// the same way an empty slice from a null pointer always is here
    /// (`raw_slice`'s own contract).
    fn unsat_event(span: &mut RawSpan) -> *mut c_void {
        std::ptr::from_mut(span).cast()
    }

    /// A finish event with an arbitrary result bitset: none of these tests
    /// reads it back.
    fn finish_event(bits: &mut ffi::clingo_solve_result_bitset_t) -> *mut c_void {
        std::ptr::from_mut(bits).cast()
    }

    /// What one call to [`solve_event_with_handler`] produced: its return
    /// value, the final `*goon`, and whatever it left in the context's
    /// slots.
    struct Outcome {
        ok: bool,
        goon: bool,
        error: Option<ErrorKind>,
        panicked: bool,
    }

    /// Builds a fresh context around `handler`, sends it one event of `kind`
    /// with payload `event`, and reports what happened.
    fn run_event(
        handler: Reacts,
        sync: Arc<SolveSync>,
        kind: ffi::clingo_solve_event_type_t,
        event: *mut c_void,
    ) -> Outcome {
        let context = EventData::new(sync, handler);
        let data = std::ptr::from_ref(&context).cast_mut().cast::<c_void>();
        let mut goon = true;
        // SAFETY: `data` points to the live `context` above, which outlives
        // this call; `event` is one of this module's own well-typed fakes
        // for `kind`, valid for the call's duration.
        let ok = unsafe { solve_event_with_handler::<Reacts>(kind, event, data, &raw mut goon) };
        Outcome {
            ok,
            goon,
            error: context.error.take().map(|e| e.kind()),
            panicked: context.panic.is_set(),
        }
    }

    #[test]
    fn the_model_event_continuing_returns_true_and_leaves_goon() {
        let handler = Reacts::default();
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_model,
            model_event(),
        );
        assert!(outcome.ok);
        assert!(outcome.goon);
        assert!(outcome.error.is_none());
        assert!(!outcome.panicked);
    }

    #[test]
    fn the_model_event_breaking_returns_true_and_clears_goon() {
        let handler = Reacts {
            model: Reaction::Break,
            ..Reacts::default()
        };
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_model,
            model_event(),
        );
        assert!(outcome.ok, "a graceful stop is not clingo's error path");
        assert!(!outcome.goon);
    }

    #[test]
    fn the_model_event_erring_never_returns_false_and_records_the_error() {
        let handler = Reacts {
            model: Reaction::Fail,
            ..Reacts::default()
        };
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_model,
            model_event(),
        );
        assert!(outcome.ok, "U26: never false for the model event either");
        assert!(!outcome.goon, "the stop goes through goon instead");
        assert_eq!(outcome.error, Some(ErrorKind::Runtime));
    }

    #[test]
    fn the_model_event_panicking_never_returns_false_and_records_the_panic() {
        let handler = Reacts {
            model: Reaction::Panic,
            ..Reacts::default()
        };
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_model,
            model_event(),
        );
        assert!(outcome.ok, "U26: never false for the model event either");
        assert!(!outcome.goon);
        assert!(outcome.panicked);
    }

    #[test]
    fn the_unsat_event_continuing_returns_true_and_leaves_goon() {
        let handler = Reacts::default();
        let mut span = RawSpan {
            first: std::ptr::null(),
            size: 0,
        };
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_unsat,
            unsat_event(&mut span),
        );
        assert!(outcome.ok);
        assert!(outcome.goon);
    }

    #[test]
    fn the_unsat_event_breaking_returns_true_and_clears_goon() {
        let handler = Reacts {
            unsat: Reaction::Break,
            ..Reacts::default()
        };
        let mut span = RawSpan {
            first: std::ptr::null(),
            size: 0,
        };
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_unsat,
            unsat_event(&mut span),
        );
        assert!(outcome.ok);
        assert!(!outcome.goon);
    }

    #[test]
    fn the_unsat_event_erring_never_returns_false_and_records_the_error() {
        let handler = Reacts {
            unsat: Reaction::Fail,
            ..Reacts::default()
        };
        let mut span = RawSpan {
            first: std::ptr::null(),
            size: 0,
        };
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_unsat,
            unsat_event(&mut span),
        );
        assert!(outcome.ok, "U25: never false for the unsat event");
        assert!(!outcome.goon, "the stop goes through goon instead");
        assert_eq!(outcome.error, Some(ErrorKind::Runtime));
    }

    #[test]
    fn the_unsat_event_panicking_never_returns_false_and_records_the_panic() {
        let handler = Reacts {
            unsat: Reaction::Panic,
            ..Reacts::default()
        };
        let mut span = RawSpan {
            first: std::ptr::null(),
            size: 0,
        };
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_unsat,
            unsat_event(&mut span),
        );
        assert!(outcome.ok, "U25: never false for the unsat event");
        assert!(!outcome.goon);
        assert!(outcome.panicked);
    }

    #[test]
    fn the_finish_event_continuing_returns_true_and_leaves_goon() {
        let handler = Reacts::default();
        let mut bits = 0;
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_finish,
            finish_event(&mut bits),
        );
        assert!(outcome.ok);
        assert!(outcome.goon);
    }

    #[test]
    fn the_finish_event_breaking_returns_true_and_clears_goon() {
        let handler = Reacts {
            finish: Reaction::Break,
            ..Reacts::default()
        };
        let mut bits = 0;
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_finish,
            finish_event(&mut bits),
        );
        assert!(outcome.ok);
        assert!(!outcome.goon);
    }

    #[test]
    fn the_finish_event_erring_never_returns_false_and_records_the_error() {
        let handler = Reacts {
            finish: Reaction::Fail,
            ..Reacts::default()
        };
        let mut bits = 0;
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_finish,
            finish_event(&mut bits),
        );
        assert!(outcome.ok, "U25: never false for the finish event");
        assert!(!outcome.goon);
        assert_eq!(outcome.error, Some(ErrorKind::Runtime));
    }

    #[test]
    fn the_finish_event_panicking_never_returns_false_and_records_the_panic() {
        let handler = Reacts {
            finish: Reaction::Panic,
            ..Reacts::default()
        };
        let mut bits = 0;
        let outcome = run_event(
            handler,
            sync(),
            ffi::clingo_solve_event_type_finish,
            finish_event(&mut bits),
        );
        assert!(outcome.ok, "U25: never false for the finish event");
        assert!(!outcome.goon);
        assert!(outcome.panicked);
    }

    /// The statistics event's own dispatch calls into clingo
    /// (`MutableStats::new`, which calls `clingo_statistics_root`) to build
    /// its two views, so it cannot run under Miri; these tests instead build
    /// the views with [`MutableStats::for_test`] (skipping only that FFI
    /// lookup) and drive [`conclude`] directly, the same function
    /// [`solve_event_with_handler`] itself uses to turn the handler's
    /// outcome into a return value and `*goon` for every other kind.
    fn run_statistics_event(handler: Reaction) -> Outcome {
        let error = Slot::default();
        let panic = PanicSlot::default();
        let mut goon = true;
        let mut reacts = Reacts {
            statistics: handler,
            ..Reacts::default()
        };
        let ok = conclude(&error, &panic, &raw mut goon, || {
            let mut step = MutableStatistics::from_raw(MutableStats::for_test(
                NonNull::<ffi::clingo_statistics_t>::dangling().as_ptr(),
                0,
            ));
            let mut accumulated = MutableStatistics::from_raw(MutableStats::for_test(
                NonNull::<ffi::clingo_statistics_t>::dangling().as_ptr(),
                0,
            ));
            let flow = reacts.on_statistics(&mut step, &mut accumulated)?;
            Ok(match flow {
                ControlFlow::Continue(()) => EventOutcome::Continue,
                ControlFlow::Break(()) => EventOutcome::Break,
            })
        });
        Outcome {
            ok,
            goon,
            error: error.take().map(|e| e.kind()),
            panicked: panic.is_set(),
        }
    }

    #[test]
    fn the_statistics_event_continuing_returns_true_and_leaves_goon() {
        let outcome = run_statistics_event(Reaction::Continue);
        assert!(outcome.ok);
        assert!(outcome.goon);
    }

    #[test]
    fn the_statistics_event_breaking_returns_true_and_clears_goon() {
        let outcome = run_statistics_event(Reaction::Break);
        assert!(outcome.ok);
        assert!(!outcome.goon);
    }

    #[test]
    fn the_statistics_event_erring_never_returns_false_and_records_the_error() {
        let outcome = run_statistics_event(Reaction::Fail);
        assert!(outcome.ok, "U25: never false for the statistics event");
        assert!(!outcome.goon);
        assert_eq!(outcome.error, Some(ErrorKind::Runtime));
    }

    #[test]
    fn the_statistics_event_panicking_never_returns_false_and_records_the_panic() {
        let outcome = run_statistics_event(Reaction::Panic);
        assert!(outcome.ok, "U25: never false for the statistics event");
        assert!(!outcome.goon);
        assert!(outcome.panicked);
    }

    #[test]
    fn an_earlier_failure_stops_a_later_event_without_re_entering_the_handler() {
        let handler = Reacts {
            unsat: Reaction::Fail,
            ..Reacts::default()
        };
        let context = EventData::new(sync(), handler);
        let data = std::ptr::from_ref(&context).cast_mut().cast::<c_void>();

        let mut span = RawSpan {
            first: std::ptr::null(),
            size: 0,
        };
        let mut goon = true;
        // SAFETY: `data` points to the live `context` above, which outlives
        // both calls; `span` outlives the first.
        let ok = unsafe {
            solve_event_with_handler::<Reacts>(
                ffi::clingo_solve_event_type_unsat,
                std::ptr::from_mut(&mut span).cast(),
                data,
                &raw mut goon,
            )
        };
        assert!(ok);
        assert!(!goon);
        assert!(context.error.is_set());

        // A later event on the same search, even one the handler would
        // otherwise continue on, stops at once (S8) without calling it
        // again: the model event, whose `calls` would be visible above.
        goon = true;
        // SAFETY: `data` as above; a null pointer with a zero length is a
        // valid empty slice, and the model event never reads through the
        // pointer this test passes.
        let ok = unsafe {
            solve_event_with_handler::<Reacts>(
                ffi::clingo_solve_event_type_model,
                model_event(),
                data,
                &raw mut goon,
            )
        };
        assert!(ok, "U26: the model event never takes clingo's error path");
        assert!(!goon, "the earlier failure's stop still goes through goon");
        assert_eq!(
            context.handler.lock().unwrap().calls,
            1,
            "the second event never reached the handler"
        );
    }

    #[test]
    fn the_finish_step_runs_before_on_finish_even_when_it_errs() {
        let handler = Reacts {
            finish: Reaction::Fail,
            ..Reacts::default()
        };
        let sync = running_sync();
        assert!(sync.is_running());
        let mut bits = 0;
        let outcome = run_event(
            handler,
            sync.clone(),
            ffi::clingo_solve_event_type_finish,
            finish_event(&mut bits),
        );
        assert_eq!(outcome.error, Some(ErrorKind::Runtime));
        assert!(
            !sync.is_running(),
            "the S13 step runs first, unconditionally, before on_finish's own error"
        );
    }

    #[test]
    fn the_finish_step_runs_before_on_finish_even_when_it_panics() {
        let handler = Reacts {
            finish: Reaction::Panic,
            ..Reacts::default()
        };
        let sync = running_sync();
        assert!(sync.is_running());
        let mut bits = 0;
        let outcome = run_event(
            handler,
            sync.clone(),
            ffi::clingo_solve_event_type_finish,
            finish_event(&mut bits),
        );
        assert!(outcome.panicked);
        assert!(
            !sync.is_running(),
            "the S13 step runs first, unconditionally, before on_finish's own panic"
        );
    }
}

//! Interrupts that reach the running search and nothing else (DESIGN S13).
//!
//! # The problem
//!
//! `clingo_control_interrupt` ends in `ClaspFacade::SolveData::interrupt`
//! (`clasp_facade.cpp:474-478`):
//!
//! ```text
//! if (solving()) { return active->interrupt(sig); }
//! if (!qSig && sig != SIGCANCEL) { qSig = sig; }
//! ```
//!
//! `solving()` is true while the search strategy is in its `run` or `model`
//! state. An interrupt that finds it false is queued in `qSig`, and the next
//! solve call takes it at its start (`facade_->interrupt(0)`,
//! `clasp_facade.cpp:322`) and ends at once, interrupted. Nothing else drains
//! the queue. So an interrupt must never reach clingo while the strategy is
//! not running: while the control is idle or grounding, before a search has
//! started, and after it has finished while its handle is still open. The
//! check and the queueing are not atomic in clasp, so an interrupt that
//! races with the end of a search can be queued too.
//!
//! The call is also not safe at every moment: starting a search replaces the
//! strategy (`solve_->active`) and a later program update deletes it, without
//! synchronisation, and `clingo_control_free` frees the control.
//!
//! # The design
//!
//! [`SolveSync`] is shared by the control and every `InterruptHandle`. Its
//! lock guards a phase and the control pointer, and `clingo_control_interrupt`
//! is only ever called with the lock held and the phase `Running`. The
//! invariant is:
//!
//! > While the lock is held and the phase is `Running`, clasp's strategy for
//! > this control is running, and it keeps running until the lock is
//! > released.
//!
//! It holds because of where the phase changes:
//!
//! - **Start.** For a search in yield or async mode, the control sets
//!   `Starting` before `clingo_control_solve` and
//!   `Running` only after it has returned, when the strategy exists and has
//!   attached (clasp sets `run` inside the start, `clasp_facade.cpp:320`, and
//!   in async mode `doStart` waits for it, `clasp_facade.cpp:376-382`). While
//!   the phase is `Starting` no interrupt is sent: it is recorded as pending,
//!   and the control delivers it itself when it sets `Running`.
//! - **End.** clasp leaves the running states in exactly one place,
//!   `detachAlgo` (`clasp_facade.cpp:344-372`). Before it marks the strategy
//!   done (`doNotify(event_detach)`, line 355) it calls `stopStep` (line 350),
//!   which reports `StepReady` to the context's event handler
//!   (`clasp_facade.cpp:915-937`). clingo's handler turns that into the finish
//!   event of the solve-event callback (`ClingoLib::onEvent`, `onFinish`,
//!   `clingocontrol.cc:349-354, 1025-1029`). Every search clingox starts
//!   registers [`solve_event`] with this state as its data, and the finish
//!   event sets the phase to `Idle` under the lock. So the strategy cannot
//!   become done while another thread holds the lock with the phase
//!   `Running`: its finish event waits for the lock first. After the finish
//!   event, no interrupt reaches clingo. This covers a search that ends by
//!   itself, by an interrupt, by `cancel`, and by `close`, which cancels.
//!   `stopStep` runs once per solve call: its `solved` flag is cleared by the
//!   program update at the start of each one.
//! - **Drop.** The control closes any open search, then clears the pointer
//!   under the lock before `clingo_control_free`, so an interrupt that holds
//!   the lock finishes first and no later one sees the pointer.
//!
//! The strategy pointer that `clingo_control_interrupt` reads is only
//! written while starting a search and by the next program update, both on
//! the control's thread and both outside `Running`; the lock orders those
//! writes before any read.
//!
//! An interrupt from the control's own thread (a model closure, a ground
//! callback, the logger) takes the lock like any other; the control never
//! holds it across a call that runs user code or ends a search, so this
//! cannot deadlock. `clingo_control_interrupt` with a signal other than
//! cancel only sets clasp's stop flags and resets a timer
//! (`clasp_facade.cpp:216-220`, `parallel_solve.cpp:215-228, 495-500`): it
//! never waits for another thread and never delivers the finish event. Two
//! of those writes race with the solver threads inside clasp; see the
//! `SAFETY` comment on `ControlPtr`.
//!
//! # What is left
//!
//! - **A blocking search on a build without threads is not interrupted from
//!   inside it.** Where clasp has threads, `Control::solve` runs in async mode
//!   and waits for the result, which is as fast as clingo's own blocking solve
//!   and keeps the phases above. Without threads (the default WebAssembly
//!   build) there is no async mode, so it uses mode 0, in which the whole
//!   search, from clingo's preparation of the program to the finish event,
//!   runs inside one `clingo_control_solve` call on the control's thread. The
//!   only code that can call `interrupt` meanwhile is a callback on that
//!   thread, such as the logger. It cannot tell whether clasp's strategy has
//!   attached yet: the logger also runs during the preparation, before the
//!   strategy exists, and an interrupt then would be queued for the next
//!   solve call. So in this phase (`Inside`) `interrupt` returns `false` and
//!   does nothing. No other thread exists to call it.
//! - An interrupt during `Starting` returns `true` and is delivered as soon
//!   as the search runs. If the search ends at its start without running
//!   (the program is already inconsistent, or clasp stops it for another
//!   reason), there is nothing left to stop: the interrupt is dropped, and the
//!   result is the search's own. clingo does the same with an interrupt that
//!   arrives in the last moment of a search.
//! - An interrupt is not delivered once the finish event has run, although
//!   the handle may still be open; `interrupt` returns `false` then. The
//!   search has already ended, so there is nothing to stop.
//! - If clasp throws inside `stopStep` before it reports `StepReady` (only an
//!   allocation failure in its statistics update can), `detachAlgo` marks the
//!   strategy done without the finish event. Until the handle is closed,
//!   which sets `Idle`, an interrupt could then be queued for the next solve
//!   call. The search itself fails with that error. clingox cannot observe
//!   the end of a search any earlier than clasp reports it.

use std::ffi::{c_uint, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::{Mutex, MutexGuard};

use clingox_sys as ffi;

use super::trampoline::PanicSlot;

/// The state an `InterruptHandle` shares with its control.
#[derive(Debug)]
pub(crate) struct SolveSync {
    state: Mutex<SyncState>,
    /// A panic in the finish event, which only sets the phase and cannot
    /// panic in practice; kept because every trampoline catches (S8).
    panic: PanicSlot,
}

#[derive(Debug)]
struct SyncState {
    /// The control to interrupt; `None` once it is freed.
    control: Option<ControlPtr>,
    phase: Phase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// No search is running: the control is idle, grounding, or its search
    /// has finished.
    Idle,
    /// A search is being started; `pending` records an interrupt that arrived
    /// meanwhile.
    Starting { pending: bool },
    /// clasp's strategy is running (see the module invariant).
    Running,
    /// A blocking search runs inside `clingo_control_solve` on the control's
    /// thread (mode 0, on a build without threads). No interrupt is sent: see
    /// "What is left" in the module documentation.
    Inside,
}

/// The control pointer, moved into the shared state.
#[derive(Clone, Copy, Debug)]
struct ControlPtr(NonNull<ffi::clingo_control_t>);

// SAFETY: the pointer is only passed to `clingo_control_interrupt`, one call
// at a time, under the lock with the phase `Running`, which the module
// documentation shows means the control is alive, and its search strategy
// exists and runs, until the lock is released.
//
// What that call touches from the interrupting thread, while solver threads
// run (`clingocontrol.cc:456`, `clasp_facade.cpp:216-220, 474-478, 959-961`):
// - `solve_` and `active`, plain pointers written only on the control's
//   thread outside `Running`; the lock orders those writes before this read;
// - the strategy's atomic `state_` and `signal_`. The compare-and-swap on
//   `signal_` succeeds once per search, so the rest runs at most once per
//   search;
// - with one solver thread, `SequentialSolve::term_`, a `volatile int` that
//   this call increments and the solver thread reads in its propagation loop
//   (`solve_algorithms.cpp:417-436`). It is not atomic;
// - with several, `ParallelSolve::SharedData`: the atomic `control` flags,
//   and the `syncT` timer, a plain struct of doubles this call resets and
//   restarts while solver threads read and lap it (`parallel_solve.cpp:215-228,
//   588-589`).
//
// The last two are data races, which ThreadSanitizer reports. They are in
// clasp, not in clingox: every binding reaches them the same way, since this
// is the only way to stop a running search, and clingo documents it for this
// use ("thread-safe and can be called from a signal handler",
// libpyclingo/clingo/control.py:577; clingo's own application calls it from
// a signal handler). clingox relies on that upstream contract as it relies on
// clingo.h for every other call. Neither race touches a pointer, a container
// or anything freed: `term_` is one aligned integer written once per search,
// whose only reader acts on it being non-zero, and `syncT` only feeds the
// timings clasp reports for synchronisation. Moving the pointer between
// threads adds nothing beyond that call, so clingox considers the handle
// sound to send. The races are recorded to be reported upstream.
unsafe impl Send for ControlPtr {}

impl SolveSync {
    pub(crate) fn new(control: NonNull<ffi::clingo_control_t>) -> Self {
        SolveSync {
            state: Mutex::new(SyncState {
                control: Some(ControlPtr(control)),
                phase: Phase::Idle,
            }),
            panic: PanicSlot::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, SyncState> {
        // A poisoned lock only means a thread panicked while holding it; no
        // code under this lock panics, and the state is valid after every
        // assignment, so it is used as it is.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Interrupts the running search, or records the interrupt for the search
    /// being started. Returns `false`, and does nothing, when no search is
    /// running or the control is gone.
    pub(crate) fn interrupt(&self) -> bool {
        let mut state = self.lock();
        match (state.phase, state.control) {
            (Phase::Running, Some(control)) => {
                deliver(control);
                true
            }
            (Phase::Starting { .. }, Some(_)) => {
                state.phase = Phase::Starting { pending: true };
                true
            }
            _ => false,
        }
    }

    /// Whether an interrupt would act now.
    pub(crate) fn is_running(&self) -> bool {
        let state = self.lock();
        state.control.is_some() && state.phase != Phase::Idle
    }

    /// The control is about to call `clingo_control_solve`. `inside` says the
    /// whole search runs inside that call (mode 0).
    pub(crate) fn starting(&self, inside: bool) {
        self.lock().phase = if inside {
            Phase::Inside
        } else {
            Phase::Starting { pending: false }
        };
    }

    /// `clingo_control_solve` has returned a handle: the strategy has attached
    /// and runs, unless its finish event has already set `Idle`. A pending
    /// interrupt is delivered now.
    pub(crate) fn started(&self) {
        let mut state = self.lock();
        if let Phase::Starting { pending } = state.phase {
            state.phase = Phase::Running;
            if let (true, Some(control)) = (pending, state.control) {
                deliver(control);
            }
        }
    }

    /// The search has finished, or never started: no interrupt may reach
    /// clingo until the next start.
    pub(crate) fn finished(&self) {
        self.lock().phase = Phase::Idle;
    }

    /// The control is about to be freed.
    pub(crate) fn detach(&self) {
        let mut state = self.lock();
        state.control = None;
        state.phase = Phase::Idle;
    }

    /// Resumes a panic caught in the finish event.
    pub(crate) fn resume_panic(&self) {
        self.panic.resume();
    }
}

/// Sends clingo's interrupt signal. Only called with the lock held and the
/// phase `Running`.
fn deliver(control: ControlPtr) {
    // SAFETY: the caller holds the lock with the phase `Running` (or sets it
    // under the same lock), so by the module invariant the control is alive
    // and its strategy is running and stays so until the lock is released.
    // `clingo_control_interrupt` then reaches the running search, never the
    // queue, and never touches a freed strategy (clasp_facade.cpp:474-478).
    // The races inside clasp that the call takes part in are discussed on
    // `ControlPtr`.
    unsafe { ffi::clingo_control_interrupt(control.0.as_ptr()) };
}

/// The receiver of solve events (clingo.h:2541-2559).
pub(crate) trait SolveEventSink: Sync {
    /// The search has finished. clasp calls this before it marks the search
    /// done (`clasp_facade.cpp:344-372`).
    fn finish(&self);
    /// Where a panic in [`SolveEventSink::finish`] is recorded.
    fn panic_slot(&self) -> &PanicSlot;
}

impl SolveEventSink for SolveSync {
    fn finish(&self) {
        self.finished();
    }

    fn panic_slot(&self) -> &PanicSlot {
        &self.panic
    }
}

/// The `clingo_solve_event_callback_t` trampoline for a sink of type `S`.
///
/// Only the finish event matters; model, unsat and statistics events are
/// accepted as they come. It always returns `true`: clingo terminates the
/// process when the handler fails on any event but a model
/// (`control.cc:1995-2020`), and nothing here can fail. `goon` keeps clingo's
/// initial `true`. The finish work runs even after a caught panic, because
/// the interrupt invariant depends on it.
///
/// # Safety
///
/// `data` must point to an `S` that is valid for the duration of the call.
pub(crate) unsafe extern "C" fn solve_event<S: SolveEventSink>(
    kind: c_uint,
    _event: *mut c_void,
    data: *mut c_void,
    _goon: *mut bool,
) -> bool {
    // SAFETY: the caller passes the sink pointer registered with
    // clingo_control_solve, which points to a live `S`. Only shared access is
    // taken, and `S: Sync`, since async and parallel searches call this from
    // clasp's threads.
    let sink = unsafe { &*data.cast::<S>().cast_const() };
    if kind == ffi::clingo_solve_event_type_finish
        && let Err(payload) = catch_unwind(AssertUnwindSafe(|| sink.finish()))
    {
        sink.panic_slot().store(payload);
    }
    true
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[derive(Default)]
    struct Counter {
        finished: AtomicU32,
        panic: PanicSlot,
    }

    impl SolveEventSink for Counter {
        fn finish(&self) {
            let earlier = self.finished.fetch_add(1, Ordering::SeqCst);
            assert!(earlier != 1, "the second finish panics on purpose");
        }
        fn panic_slot(&self) -> &PanicSlot {
            &self.panic
        }
    }

    fn send(counter: &Counter, kind: c_uint) -> bool {
        let mut goon = true;
        let data = std::ptr::from_ref(counter).cast_mut().cast::<c_void>();
        // SAFETY: `data` points to a live Counter that outlives the call.
        unsafe { solve_event::<Counter>(kind, std::ptr::null_mut(), data, &raw mut goon) }
    }

    #[test]
    fn only_the_finish_event_reaches_the_sink_and_every_event_succeeds() {
        let counter = Counter::default();
        for kind in [
            ffi::clingo_solve_event_type_model,
            ffi::clingo_solve_event_type_unsat,
            ffi::clingo_solve_event_type_statistics,
        ] {
            assert!(send(&counter, kind));
        }
        assert_eq!(counter.finished.load(Ordering::SeqCst), 0);
        assert!(send(&counter, ffi::clingo_solve_event_type_finish));
        assert_eq!(counter.finished.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_panic_in_the_finish_event_is_caught_and_later_events_still_run() {
        let counter = Counter::default();
        assert!(send(&counter, ffi::clingo_solve_event_type_finish));
        assert!(
            send(&counter, ffi::clingo_solve_event_type_finish),
            "never false"
        );
        assert!(counter.panic.is_set());
        assert!(send(&counter, ffi::clingo_solve_event_type_finish));
        assert_eq!(
            counter.finished.load(Ordering::SeqCst),
            3,
            "finish always runs"
        );
    }

    /// A panic caught in the finish event, which clasp may run
    /// on its own thread, resumes on the caller's thread.
    #[test]
    fn a_panic_in_the_finish_event_resumes() {
        let mut dummy = 0_u8;
        let sync = SolveSync::new(NonNull::from(&mut dummy).cast());
        // What `solve_event` does when the sink's `finish` panics.
        sync.panic_slot()
            .store(Box::new("the finish event failed on purpose"));
        let resumed =
            catch_unwind(AssertUnwindSafe(|| sync.resume_panic())).expect_err("the panic resumes");
        assert_eq!(
            resumed.downcast_ref::<&str>(),
            Some(&"the finish event failed on purpose")
        );
        sync.resume_panic();
    }

    /// The phase machine without clingo: only `Running` would deliver, so
    /// these transitions never reach `deliver`.
    #[test]
    fn interrupts_outside_a_running_search_do_nothing() {
        let mut dummy = 0_u8;
        let sync = SolveSync::new(NonNull::from(&mut dummy).cast());
        assert!(!sync.interrupt(), "idle");
        sync.starting(false);
        assert!(sync.interrupt(), "recorded while starting");
        sync.finished();
        sync.started();
        assert!(!sync.is_running(), "a finish during the start wins");
        assert!(!sync.interrupt(), "finished");
        sync.starting(true);
        assert!(!sync.interrupt(), "a mode-0 search is not interrupted");
        sync.finished();
        sync.starting(false);
        sync.detach();
        assert!(!sync.interrupt(), "detached");
    }
}

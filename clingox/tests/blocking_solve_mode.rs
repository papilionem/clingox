//! Which thread runs a blocking solve: clingo's mode 0, in which the
//! search runs inside `clingo_control_solve` on the calling thread, against
//! async mode, in which clasp starts a thread of its own.
//!
//! `Control::solve`, `Control::solve_with` and `Control::solve_with_events`
//! solve in mode 0 when nothing can interrupt the search: no `InterruptHandle`
//! is alive and the call has no timeout. Otherwise they solve in async mode, so
//! that an interrupt can reach the search once it runs (DESIGN S13).
//!
//! The mode is observed through public API only: at `-t 1` clasp has a single
//! solver thread, so a callback of the search runs either on the thread that
//! called `solve` (mode 0) or on the one thread clasp started (async). Two
//! callbacks are used. A `SolveEventHandler` sees the thread of `on_model`
//! for `solve_with_events`. The logger sees the thread that clasp writes its
//! `#models not 0` warning for every entry point, including the ones
//! that take no handler. The Python module `clingo` 5.8.2 confirms both
//! placements for the program below: `ctl.solve()` logs the warning on the
//! main thread, and `ctl.solve(async_=True).get()` logs it on a foreign thread.
//!
//! Every test needs threads and returns early without them: the default
//! WebAssembly build already solves in mode 0, so there is nothing to tell
//! apart.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stderr,
    reason = "a test that needs threads says visibly that it was skipped"
)]

use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::Duration;

use clingox::{
    Assumption, Control, ExtendableModel, InterruptHandle, MessageCode, Part, Result,
    SolveEventHandler, SolveOptions, SolveResult, Symbol,
};

/// Returns from the calling test when the build has no threads.
macro_rules! require_threads {
    () => {
        if !clingox_sys::HAS_THREADS {
            eprintln!("SKIPPED: this build of clingo has no threads");
            return;
        }
    };
}

/// A program with an objective. With `--models=1 --opt-mode=opt` clasp stops
/// after the first model and warns that optimality is not guaranteed.
const OPTIMISING: &str = "{a;b}. #minimize{1:a; 1:b}.";

/// The warning clasp prints for [`OPTIMISING`], as the Python module `clingo`
/// 5.8.2 reports it: code `Other` (6), this text.
const OPTIMALITY_WARNING: &str = "#models not 0: optimality of last model not guaranteed.";

/// What a logger saw: the thread, the code and the text of each message.
type Messages = Arc<Mutex<Vec<(ThreadId, MessageCode, String)>>>;

/// Records the thread of every model.
struct ModelThreads(Arc<Mutex<Vec<ThreadId>>>);

impl SolveEventHandler for ModelThreads {
    fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
        self.0
            .lock()
            .expect("the lock is not poisoned")
            .push(std::thread::current().id());
        Ok(ControlFlow::Continue(()))
    }
}

/// A control on [`OPTIMISING`] at `-t 1` that records where its model events
/// and its logged messages run.
struct Observed {
    ctl: Control,
    models: Arc<Mutex<Vec<ThreadId>>>,
    messages: Messages,
}

impl Observed {
    fn new() -> Observed {
        let messages = Messages::default();
        let sink = Arc::clone(&messages);
        let mut ctl = Control::builder()
            .args(["-t", "1", "--models=1", "--opt-mode=opt"])
            .logger(move |code, text| {
                sink.lock().expect("the lock is not poisoned").push((
                    std::thread::current().id(),
                    code,
                    text.to_owned(),
                ));
            })
            .build()
            .expect("the arguments are valid");
        ctl.add_base(OPTIMISING).expect("the program parses");
        ctl.ground(&[Part::base()]).expect("the program grounds");
        // Grounding may log; the tests look at what the search logs.
        sink_clear(&messages);
        Observed {
            ctl,
            models: Arc::default(),
            messages,
        }
    }

    /// The threads that ran `on_model` since the last call.
    fn take_models(&self) -> Vec<ThreadId> {
        std::mem::take(&mut *self.models.lock().expect("the lock is not poisoned"))
    }

    /// The messages logged since the last call.
    fn take_messages(&self) -> Vec<(ThreadId, MessageCode, String)> {
        sink_take(&self.messages)
    }

    /// The thread of the one optimality warning logged since the last call.
    fn take_warning_thread(&self) -> ThreadId {
        let messages = self.take_messages();
        assert_eq!(messages.len(), 1, "one warning per search: {messages:?}");
        let (thread, code, text) = &messages[0];
        assert_eq!(*code, MessageCode::Other);
        assert_eq!(text, OPTIMALITY_WARNING);
        *thread
    }

    /// Solves with a handler and returns the thread of its model event and
    /// the thread of the warning.
    fn solve_with_events(&mut self, options: SolveOptions) -> (ThreadId, ThreadId) {
        let result = self
            .ctl
            .solve_with_events(options, ModelThreads(Arc::clone(&self.models)))
            .expect("the program solves");
        assert!(result.is_sat() && !result.is_interrupted(), "{result:?}");
        let models = self.take_models();
        assert_eq!(models.len(), 1, "--models=1 reports one model");
        (models[0], self.take_warning_thread())
    }

    /// Solves with `Control::solve` and returns the thread of the warning.
    fn solve(&mut self) -> ThreadId {
        let result = self.ctl.solve(&[]).expect("the program solves");
        assert!(result.is_sat() && !result.is_interrupted(), "{result:?}");
        self.take_warning_thread()
    }

    /// Solves with `Control::solve_with` and returns the thread of the
    /// warning.
    fn solve_with(&mut self, options: SolveOptions) -> ThreadId {
        let result = self.ctl.solve_with(options).expect("the program solves");
        assert!(result.is_sat() && !result.is_interrupted(), "{result:?}");
        self.take_warning_thread()
    }
}

fn sink_clear(messages: &Messages) {
    messages.lock().expect("the lock is not poisoned").clear();
}

fn sink_take(messages: &Messages) -> Vec<(ThreadId, MessageCode, String)> {
    std::mem::take(&mut *messages.lock().expect("the lock is not poisoned"))
}

/// Every entry point that can solve in mode 0 runs the search on the calling
/// thread.
fn assert_all_on_the_calling_thread(observed: &mut Observed, when: &str) {
    let me = std::thread::current().id();
    let (model, warning) = observed.solve_with_events(SolveOptions::new());
    assert_eq!(model, me, "{when}: solve_with_events, model event");
    assert_eq!(warning, me, "{when}: solve_with_events, logger");
    assert_eq!(observed.solve(), me, "{when}: solve, logger");
    assert_eq!(
        observed.solve_with(SolveOptions::new()),
        me,
        "{when}: solve_with, logger"
    );
}

/// Every entry point that must keep async mode runs the search on another
/// thread.
fn assert_all_off_the_calling_thread(observed: &mut Observed, when: &str) {
    let me = std::thread::current().id();
    let (model, warning) = observed.solve_with_events(SolveOptions::new());
    assert_ne!(model, me, "{when}: solve_with_events, model event");
    assert_ne!(warning, me, "{when}: solve_with_events, logger");
    assert_ne!(observed.solve(), me, "{when}: solve, logger");
    assert_ne!(
        observed.solve_with(SolveOptions::new()),
        me,
        "{when}: solve_with, logger"
    );
}

#[test]
fn a_blocking_solve_without_a_handle_runs_on_the_calling_thread() {
    require_threads!();
    let mut observed = Observed::new();
    assert_all_on_the_calling_thread(&mut observed, "no handle was ever made");

    // The same holds after a handle came and went: it was the only holder.
    drop(observed.ctl.interrupt_handle());
    assert_all_on_the_calling_thread(&mut observed, "after a dropped handle");

    // And after solves with a timeout: nothing of the timeout is left holding
    // the control once they have returned. The budget is long, so the search
    // ends long before it.
    let budget = || SolveOptions::new().timeout(Duration::from_secs(60));
    observed.solve_with_events(budget());
    observed.solve_with(budget());
    assert_all_on_the_calling_thread(&mut observed, "after solves with a timeout");
}

#[test]
fn a_live_interrupt_handle_keeps_the_search_off_the_calling_thread() {
    require_threads!();
    let mut observed = Observed::new();
    let handle = observed.ctl.interrupt_handle();
    assert_all_off_the_calling_thread(&mut observed, "a handle is alive");

    // A clone is a second holder: the original can go and the search stays
    // off the calling thread.
    let clone: InterruptHandle = handle.clone();
    drop(handle);
    assert_all_off_the_calling_thread(&mut observed, "only a clone is alive");

    // The last holder is gone: the calling thread runs the search again.
    drop(clone);
    assert_all_on_the_calling_thread(&mut observed, "after the last handle was dropped");
}

#[test]
fn a_timeout_keeps_the_search_off_the_calling_thread() {
    require_threads!();
    let mut observed = Observed::new();
    let me = std::thread::current().id();
    // The budget is long: the search ends long before it, and what is checked
    // is only where the search ran.
    let budget = || SolveOptions::new().timeout(Duration::from_secs(60));

    let (model, warning) = observed.solve_with_events(budget());
    assert_ne!(model, me, "solve_with_events with a timeout, model event");
    assert_ne!(warning, me, "solve_with_events with a timeout, logger");
    assert_ne!(
        observed.solve_with(budget()),
        me,
        "solve_with with a timeout, logger"
    );
}

/// Atoms `p(1)` to `p(22)`: 2^22 models, about a second to enumerate, so a
/// search that nothing interrupts runs long enough to be noticed.
const MANY_MODELS: &str = "{p(1..22)}.";

/// Rounds of [`mode_zero_and_interrupted_solves_alternate_on_one_control`];
/// each is a thread start and a short search, well under a millisecond in a Linux debug build.
const ROUNDS: usize = 500;

/// Assumptions that fix `p(3)` to `p(22)` false, leaving four models.
fn all_but_two_false() -> Vec<Assumption> {
    (3..=22)
        .map(|i| {
            let atom = Symbol::function("p", &[Symbol::number(i)]).expect("the name is valid");
            (atom, false).into()
        })
        .collect()
}

#[test]
fn mode_zero_and_interrupted_solves_alternate_on_one_control() {
    require_threads!();
    let mut ctl = Control::with_args(["-t", "1", "--models=0"]).expect("the arguments are valid");
    ctl.add_base(MANY_MODELS).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    let fixed = all_but_two_false();

    for round in 0..ROUNDS {
        // A search that must run to its end on the calling thread, with no
        // interrupt left over from the previous round.
        let models = Arc::new(Mutex::new(Vec::new()));
        let result = ctl
            .solve_with_events(
                SolveOptions::new().assumptions(&fixed),
                ModelThreads(Arc::clone(&models)),
            )
            .expect("the program solves");
        assert!(
            result.is_sat() && result.is_exhausted() && !result.is_interrupted(),
            "round {round}: {result:?}"
        );
        let models = models.lock().expect("the lock is not poisoned");
        assert_eq!(models.len(), 4, "round {round}: p(1) and p(2) are free");

        // A long search that another thread interrupts through a fresh
        // handle. The other thread tries until an interrupt is accepted,
        // which happens once the search runs, or gives up when it has
        // returned.
        let stop = ctl.interrupt_handle();
        let done = Arc::new(AtomicBool::new(false));
        let spinner = {
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                loop {
                    if stop.interrupt() {
                        return true;
                    }
                    if done.load(Ordering::SeqCst) {
                        return false;
                    }
                    std::thread::yield_now();
                }
            })
        };
        let result = ctl.solve(&[]).expect("the program solves");
        done.store(true, Ordering::SeqCst);
        let delivered = spinner.join().expect("the spinner does not panic");
        assert!(delivered, "round {round}: interrupt() was accepted");
        assert_interrupted(result, round);
    }
}

fn assert_interrupted(result: SolveResult, round: usize) {
    assert!(result.is_interrupted(), "round {round}: {result:?}");
    assert!(!result.is_exhausted(), "round {round}: {result:?}");
}

#[test]
fn the_optimality_warning_is_reported_in_both_modes() {
    require_threads!();
    let expected = vec![(MessageCode::Other, OPTIMALITY_WARNING.to_owned())];
    let codes_and_texts = |observed: &Observed| -> Vec<(MessageCode, String)> {
        observed
            .take_messages()
            .into_iter()
            .map(|(_, code, text)| (code, text))
            .collect()
    };

    let mut observed = Observed::new();
    let check = |observed: &mut Observed, when: &str| {
        let result = observed.ctl.solve(&[]).expect("the program solves");
        assert!(result.is_sat(), "{when}: {result:?}");
        assert_eq!(codes_and_texts(observed), expected, "{when}: solve");

        let result = observed
            .ctl
            .solve_with_events(
                SolveOptions::new(),
                ModelThreads(Arc::clone(&observed.models)),
            )
            .expect("the program solves");
        assert!(result.is_sat(), "{when}: {result:?}");
        assert_eq!(
            codes_and_texts(observed),
            expected,
            "{when}: solve_with_events"
        );
    };

    // Mode 0: no handle.
    check(&mut observed, "without a handle");

    // Async mode: a live handle.
    let handle = observed.ctl.interrupt_handle();
    check(&mut observed, "with a live handle");

    // Mode 0 again.
    drop(handle);
    check(&mut observed, "after the handle was dropped");
}

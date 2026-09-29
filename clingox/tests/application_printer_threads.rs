//! The model printer on solver threads, run under `ThreadSanitizer` and
//! `AddressSanitizer` by `cargo xtask sanitize` (this file is in
//! `THREAD_TESTS`).
//!
//! Expected values come from pyclingo 5.8.2 (clingo 5.8.2, clasp 3.4.1)
//! calling `clingo_main` at the C level with a `printer` callback:
//!
//! - with a blocking solve in `main` and `-t4`, about one call in twelve runs
//!   on the calling thread (solver thread 0), so only an asynchronous solve
//!   guarantees that every call is on another thread;
//! - a failing printer never returns false to clingo (U25, U26): the error is
//!   stored, the closure is skipped for every later model and the search is
//!   interrupted; `run` returns the stored error.
//!
//! Without threads (WebAssembly) every test here is skipped, visibly.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test helpers fail loudly; skips and the watchdog explain themselves"
)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::thread::{self, ThreadId};
use std::time::Duration;

use clingox::application::{Application, DefaultPrinter};
use clingox::{Error, ErrorKind, Model, Part, Result, ScopedControl, Symbol};

const LIMIT: Duration = Duration::from_secs(30);
/// 512 models.
const NINE: &str = "{x(1..9)}.";
const ALL: [&str; 2] = ["--verbose=0", "0"];
const ALL_T4: [&str; 3] = ["--verbose=0", "0", "-t4"];

static SERIAL: Mutex<()> = Mutex::new(());

struct Watchdog(Option<mpsc::Sender<()>>);

impl Watchdog {
    fn start() -> Watchdog {
        let (done, wait) = mpsc::channel::<()>();
        thread::spawn(move || {
            if wait.recv_timeout(LIMIT) == Err(mpsc::RecvTimeoutError::Timeout) {
                eprintln!("the test hung for {LIMIT:?}: run did not return; aborting the process");
                std::process::abort();
            }
        });
        Watchdog(Some(done))
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Some(done) = self.0.take() {
            let _ = done.send(());
        }
    }
}

/// The lock and the watchdog, or `None` (after saying so) without threads.
fn begin(name: &str) -> Option<(MutexGuard<'static, ()>, Watchdog)> {
    if !clingox_sys::HAS_THREADS {
        eprintln!("SKIPPED: {name}: this build has no threads");
        return None;
    }
    let lock = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Some((lock, Watchdog::start()))
}

fn async_solve(ctl: &mut ScopedControl<'_>, program: &str) -> Result<()> {
    ctl.add_base(program)?;
    ctl.ground(&[Part::base()])?;
    let mut handle = ctl.solve_async(&[])?;
    let _ = handle.get()?;
    let _ = handle.close()?;
    Ok(())
}

fn blocking_solve(ctl: &mut ScopedControl<'_>, program: &str) -> Result<()> {
    ctl.add_base(program)?;
    ctl.ground(&[Part::base()])?;
    let _ = ctl.solve(&[])?;
    Ok(())
}

/// What the printer recorded over a whole run.
#[derive(Default)]
struct Log {
    numbers: Vec<u64>,
    threads: Vec<ThreadId>,
    thread_ids: Vec<u32>,
}

/// A printer that records every call and detects two calls at once.
fn recording<'a>(
    log: &'a Mutex<Log>,
    inside: &'a AtomicBool,
    overlaps: &'a AtomicUsize,
) -> impl for<'p> Fn(&Model, &mut DefaultPrinter<'p>) -> Result<()> + Send + Sync + 'a {
    move |model, _printer| {
        if inside.swap(true, Ordering::SeqCst) {
            overlaps.fetch_add(1, Ordering::SeqCst);
        }
        {
            let mut log = log.lock().unwrap();
            log.numbers.push(model.number());
            log.threads.push(thread::current().id());
            log.thread_ids.push(model.thread_id());
        }
        // Long enough for a second thread to arrive if the guarantee failed.
        thread::sleep(Duration::from_micros(200));
        inside.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn t1_an_async_solve_puts_the_printer_on_another_thread() {
    let Some(_guard) = begin("t1") else { return };
    let caller = thread::current().id();
    let log = Mutex::new(Log::default());
    let (inside, overlaps) = (AtomicBool::new(false), AtomicUsize::new(0));
    let code = Application::new()
        .main(|ctl, _files| async_solve(ctl, "{x(1..2)}. y. #show x/1."))
        .print_model(recording(&log, &inside, &overlaps))
        .run(ALL)
        .unwrap();
    assert_eq!(code, 30);
    let log = log.into_inner().unwrap();
    let mut numbers = log.numbers.clone();
    numbers.sort_unstable();
    assert_eq!(numbers, [1, 2, 3, 4]);
    // pyclingo 5.8.2: 96 of 96 calls off the calling thread, thread id 0.
    assert!(log.threads.iter().all(|t| *t != caller));
    assert!(log.thread_ids.iter().all(|id| *id == 0));
}

#[test]
fn t2_the_printer_runs_while_main_keeps_working() {
    let Some(_guard) = begin("t2") else { return };
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let started_tx = Mutex::new(started_tx);
    let release_rx = Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    let overlap = Mutex::new(None);
    let code = Application::new()
        .main(|ctl, _files| {
            ctl.add_base("{x(1..2)}.")?;
            ctl.ground(&[Part::base()])?;
            let mut handle = ctl.solve_async(&[])?;
            // The first model's printer call is blocked until `main` says so,
            // so the search cannot have finished while `main` is here.
            let running = started_rx.recv_timeout(Duration::from_secs(10)).is_ok();
            let finished = handle.wait(Duration::from_millis(50));
            *overlap.lock().unwrap() = Some((running, finished));
            release_tx.send(()).unwrap();
            let _ = handle.get()?;
            let _ = handle.close()?;
            Ok(())
        })
        .print_model(|_model, _printer| {
            if first.swap(false, Ordering::SeqCst) {
                started_tx.lock().unwrap().send(()).unwrap();
                let _ = release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10));
            }
            Ok(())
        })
        .run(ALL)
        .unwrap();
    assert_eq!(code, 30);
    assert_eq!(
        overlap.into_inner().unwrap(),
        Some((true, false)),
        "main saw the printer running and the search unfinished"
    );
}

fn full_run(solve: fn(&mut ScopedControl<'_>, &str) -> Result<()>) -> (Log, usize) {
    let log = Mutex::new(Log::default());
    let (inside, overlaps) = (AtomicBool::new(false), AtomicUsize::new(0));
    let code = Application::new()
        .main(move |ctl, _files| solve(ctl, NINE))
        .print_model(recording(&log, &inside, &overlaps))
        .run(ALL_T4)
        .unwrap();
    assert_eq!(code, 30);
    (log.into_inner().unwrap(), overlaps.load(Ordering::SeqCst))
}

fn check_complete(log: &Log, overlaps: usize) {
    let mut numbers = log.numbers.clone();
    numbers.sort_unstable();
    assert_eq!(numbers, (1..=512).collect::<Vec<u64>>(), "each model once");
    assert!(log.thread_ids.iter().all(|id| *id < 4), "four threads");
    assert_eq!(overlaps, 0, "the closure never runs twice at once");
}

#[test]
fn t3_four_threads_async_every_call_is_off_the_calling_thread() {
    let Some(_guard) = begin("t3") else { return };
    let caller = thread::current().id();
    let (log, overlaps) = full_run(async_solve);
    check_complete(&log, overlaps);
    assert!(log.threads.iter().all(|t| *t != caller));
}

#[test]
fn t4_four_threads_blocking_complete_and_never_overlapping() {
    let Some(_guard) = begin("t4") else { return };
    let (log, overlaps) = full_run(blocking_solve);
    // No claim about which threads: about one call in twelve is on the
    // calling thread (solver thread 0) with a blocking solve.
    check_complete(&log, overlaps);
}

/// A failing printer at `-t4`, an async search, then updates of the control:
/// U26 corrupted clasp when a model callback failed this way.
fn failing_at_third(fail: fn() -> Fail) -> (Result<i32>, usize, usize, bool) {
    let calls = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let late = AtomicUsize::new(0);
    let updated = AtomicBool::new(false);
    let e = Symbol::function("e", &[]).unwrap();
    let result = Application::new()
        .main(|ctl, _files| {
            ctl.add_base("#external e. {x(1..7)}. y.")?;
            ctl.ground(&[Part::base()])?;
            let mut handle = ctl.solve_async(&[])?;
            let _ = handle.get();
            let _ = handle.close();
            // Updates after the failed search: these read clasp's state.
            let released = ctl.release_external(e).is_ok();
            let cleaned = ctl.cleanup().is_ok();
            updated.store(released && cleaned, Ordering::SeqCst);
            Ok(())
        })
        .print_model(|_model, _printer| {
            if failed.load(Ordering::SeqCst) {
                late.fetch_add(1, Ordering::SeqCst);
            }
            if calls.fetch_add(1, Ordering::SeqCst) + 1 == 3 {
                failed.store(true, Ordering::SeqCst);
                return match fail() {
                    Fail::Error(error) => Err(error),
                    Fail::Panic => panic!("printer panic"),
                };
            }
            Ok(())
        })
        .run(ALL_T4);
    (
        result,
        calls.load(Ordering::SeqCst),
        late.load(Ordering::SeqCst),
        updated.load(Ordering::SeqCst),
    )
}

enum Fail {
    Error(Error),
    Panic,
}

#[test]
fn t5_a_failing_printer_stops_the_search_and_leaves_the_control_usable() {
    let Some(_guard) = begin("t5") else { return };
    for _ in 0..3 {
        let (result, calls, late, updated) =
            failing_at_third(|| Fail::Error(Error::new(ErrorKind::Runtime, "printer failed")));
        let error = result.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Runtime);
        assert!(error.to_string().contains("printer failed"), "{error}");
        // `late` counts closure entries after the failure was recorded: models
        // still in flight on other threads must be skipped by the wrapper.
        assert_eq!(late, 0, "the closure ran again after the failure");
        assert!(calls >= 3, "{calls}");
        assert!(
            updated,
            "release_external and cleanup work after the failure"
        );
    }
}

#[test]
fn t5b_a_panicking_printer_is_resumed_and_the_control_stays_usable() {
    let Some(_guard) = begin("t5b") else { return };
    let outcome = catch_unwind(AssertUnwindSafe(|| failing_at_third(|| Fail::Panic)));
    let payload = outcome.expect_err("the panic reaches the caller of run");
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    assert!(text.contains("printer panic"), "{text}");
}

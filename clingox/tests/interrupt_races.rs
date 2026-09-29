//! Interrupts racing with the start and the end of searches (DESIGN S13).
//!
//! clingo queues an interrupt that finds no running search and ends the next
//! solve call with it. clingox must never let one through outside a running
//! search, including when it arrives just as a search starts or finishes on
//! another thread. Each test below lets another thread call `interrupt` in a
//! tight loop while the control starts and finishes many short searches,
//! then stops that thread and checks that the next solve call is not
//! interrupted. An interrupt that lands on one of the short searches is
//! legitimate; only one that survives into the check is a failure.
//!
//! The races are probabilistic by nature, so these tests cannot prove the
//! absence of a window; they exercise the windows many times on every
//! target with threads, and the deterministic cases are in `api_interrupt.rs`
//! and `api_async.rs`.

#![forbid(unsafe_code)]
#![cfg(not(all(target_family = "wasm", not(target_feature = "atomics"))))]

use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use clingox::{Control, InterruptHandle, Part, SolveOptions};

/// Four models, found in microseconds: many search starts and ends per
/// second.
const SHORT: &str = "{a;b}.";

const ROUNDS: usize = 100;
const SOLVES_PER_ROUND: usize = 50;

fn grounded(ctl: &mut Control) {
    ctl.add_base(SHORT).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
}

/// A thread that interrupts until told to stop, and counts the interrupts
/// that were accepted.
struct Spinner {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<usize>,
}

impl Spinner {
    /// Starts the thread and returns once it is interrupting, so the searches
    /// that follow run while it does.
    fn start(handle: InterruptHandle) -> Spinner {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let running = Arc::new(Barrier::new(2));
        let spinning = Arc::clone(&running);
        let thread = std::thread::spawn(move || {
            spinning.wait();
            let mut accepted = 0;
            while !flag.load(Ordering::SeqCst) {
                accepted += usize::from(handle.interrupt());
            }
            accepted
        });
        running.wait();
        Spinner { stop, thread }
    }

    fn finish(self) -> usize {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.join().expect("the spinner does not panic")
    }
}

/// The next solve call after the spinner has stopped runs to its end.
fn assert_next_solve_is_clean(ctl: &mut Control, round: usize) {
    let result = ctl.solve(&[]).expect("the program solves");
    assert!(
        !result.is_interrupted(),
        "round {round}: a stale interrupt reached the next solve"
    );
    assert!(result.is_exhausted(), "round {round}: {result:?}");
}

fn race(mut ctl: Control, mut solve: impl FnMut(&mut Control)) {
    grounded(&mut ctl);
    for round in 0..ROUNDS {
        let spinner = Spinner::start(ctl.interrupt_handle());
        for _ in 0..SOLVES_PER_ROUND {
            solve(&mut ctl);
        }
        // How many interrupts landed depends on scheduling, so it is not
        // checked; what matters is that none outlives the round.
        spinner.finish();
        assert_next_solve_is_clean(&mut ctl, round);
    }
}

#[test]
fn interrupts_racing_with_blocking_solves_never_reach_a_later_one() {
    let ctl = Control::with_args(["--models=0"]).unwrap();
    race(ctl, |ctl| {
        let _result = ctl.solve(&[]).unwrap();
    });
}

#[test]
fn interrupts_racing_with_yield_searches_never_reach_a_later_one() {
    let ctl = Control::with_args(["--models=0"]).unwrap();
    race(ctl, |ctl| {
        let _result = ctl
            .for_each_model(&[], |_| Ok(ControlFlow::Continue(())))
            .unwrap();
    });
}

#[test]
fn interrupts_racing_with_open_finished_handles_never_reach_a_later_one() {
    // The search finishes inside `next_model`, and the handle stays open
    // until `close`: the window in which clingo would queue.
    let ctl = Control::with_args(["--models=0"]).unwrap();
    race(ctl, |ctl| {
        let mut handle = ctl.solve_yield(&[]).unwrap();
        while handle.next_model().unwrap().is_some() {}
        let _result = handle.close().unwrap();
    });
}

#[test]
fn interrupts_racing_with_async_searches_never_reach_a_later_one() {
    let ctl = Control::with_args(["--models=0"]).unwrap();
    race(ctl, |ctl| {
        let mut handle = ctl.solve_async(&[]).unwrap();
        handle.wait(Duration::from_secs(60));
        let _result = handle.close().unwrap();
    });
}

#[test]
fn interrupts_racing_with_timeouts_never_reach_a_later_one() {
    let ctl = Control::with_args(["--models=0"]).unwrap();
    race(ctl, |ctl| {
        let _result = ctl
            .solve_with(SolveOptions::new().timeout(Duration::from_secs(60)))
            .unwrap();
    });
}

#[test]
fn interrupts_racing_with_parallel_searches_never_reach_a_later_one() {
    let ctl = Control::builder()
        .threads(4)
        .args(["--models=0"])
        .build()
        .unwrap();
    race(ctl, |ctl| {
        let _result = ctl.solve(&[]).unwrap();
    });
}

#[test]
fn interrupts_racing_with_grounding_never_reach_a_later_solve() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    grounded(&mut ctl);
    for round in 0..ROUNDS {
        ctl.add("step", &[], "c.").unwrap();
        let spinner = Spinner::start(ctl.interrupt_handle());
        ctl.ground(&[Part::new("step", &[]).unwrap()]).unwrap();
        assert_eq!(spinner.finish(), 0, "grounding cannot be interrupted");
        assert_next_solve_is_clean(&mut ctl, round);
    }
}

#[test]
fn interrupts_racing_with_drop_are_safe() {
    // The control is freed while another thread interrupts: the lock in the
    // shared state makes the interrupt finish first or see the control gone.
    for _ in 0..ROUNDS {
        let mut ctl = Control::with_args(["--models=0"]).unwrap();
        grounded(&mut ctl);
        let handle = ctl.interrupt_handle();
        let spinner = Spinner::start(handle.clone());
        let _result = ctl.solve(&[]).unwrap();
        drop(ctl);
        spinner.finish();
        assert!(!handle.interrupt(), "the control is gone");
    }
}

#[test]
fn an_interrupt_from_the_logger_does_not_deadlock_or_linger() {
    // The logger runs on the thread that makes the call; interrupting from it
    // takes the same lock as any other interrupt. Grounding logs here, so the
    // interrupt finds no search.
    let slot: Arc<Mutex<Option<InterruptHandle>>> = Arc::default();
    let seen = Arc::new(AtomicUsize::new(0));
    let (logger_slot, logger_seen) = (Arc::clone(&slot), Arc::clone(&seen));
    let mut ctl = Control::builder()
        .args(["--models=0"])
        .logger(move |_, _| {
            if let Some(handle) = &*logger_slot.lock().unwrap() {
                logger_seen.fetch_add(1, Ordering::SeqCst);
                assert!(!handle.interrupt(), "no search runs while grounding");
            }
        })
        .build()
        .unwrap();
    *slot.lock().unwrap() = Some(ctl.interrupt_handle());
    ctl.add_base("{a;b}. c :- d.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 1, "the logger ran once");
    assert_next_solve_is_clean(&mut ctl, 0);
}

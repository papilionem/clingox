//! Two ThreadSanitizer-oriented races (DESIGN S13):
//!
//! - the composition test: an interrupt racing a solve that also has a
//!   user `SolveEventHandler` installed, at the scale
//!   `interrupt_races.rs` already uses for the handler-less case. This is
//!   the single most important test here: a bug that runs the
//!   internal interrupt-phase-ending step only when the user handler is
//!   also installed, or only when it returns `Continue`, would silently
//!   reopen the exact race S13's own ~400,000-run verification closed.
//! - `AsyncSolveHandle::core` called from another thread while the search
//!   it belongs to is still running .
//!
//! Like `interrupt_races.rs`, these races are probabilistic: they exercise
//! the windows many times on every target with threads rather than proving
//! their absence, and the real evidence is a clean `cargo xtask sanitize`
//! run, not these functional assertions alone.

#![forbid(unsafe_code)]
#![cfg(not(all(target_family = "wasm", not(target_feature = "atomics"))))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::thread::JoinHandle;
use std::time::Duration;

use clingox::{Control, InterruptHandle, Part, SolveEventHandler};

const ROUNDS: usize = 100;
const SOLVES_PER_ROUND: usize = 50;

/// Four models, found in microseconds, matching `interrupt_races.rs`'s own
/// `SHORT` program.
const SHORT: &str = "{a;b}.";

fn grounded(program: &str) -> Control {
    let mut ctl = Control::with_args(["--models=0"]).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// A thread that interrupts until told to stop (verbatim copy of
/// `interrupt_races.rs`'s own `Spinner`, kept file-local since integration
/// test files do not share code except through `tests/conformance/`).
struct Spinner {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<usize>,
}

impl Spinner {
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

fn assert_next_solve_is_clean(ctl: &mut Control, round: usize) {
    let result = ctl.solve(&[]).expect("the program solves");
    assert!(
        !result.is_interrupted(),
        "round {round}: a stale interrupt reached the next solve"
    );
    assert!(result.is_exhausted(), "round {round}: {result:?}");
}

struct NoOp;
impl SolveEventHandler for NoOp {}

#[test]
fn interrupts_racing_with_a_solve_that_has_a_user_handler_installed_never_reach_a_later_one() {
    let mut ctl = grounded(SHORT);
    for round in 0..ROUNDS {
        let spinner = Spinner::start(ctl.interrupt_handle());
        for _ in 0..SOLVES_PER_ROUND {
            let _result = ctl
                .solve_with_events(clingox::SolveOptions::new(), NoOp)
                .unwrap();
        }
        // How many interrupts landed depends on scheduling; what matters is
        // that none outlives the round (checked below).
        spinner.finish();
        assert_next_solve_is_clean(&mut ctl, round);
    }
}

#[test]
fn interrupts_racing_with_a_yield_search_that_has_a_user_handler_never_reach_a_later_one() {
    let mut ctl = grounded(SHORT);
    for round in 0..ROUNDS {
        let spinner = Spinner::start(ctl.interrupt_handle());
        for _ in 0..SOLVES_PER_ROUND {
            let mut handle = ctl.solve_yield_with_events(&[], NoOp).unwrap();
            while handle.next_model().unwrap().is_some() {}
            let _result = handle.close().unwrap();
        }
        spinner.finish();
        assert_next_solve_is_clean(&mut ctl, round);
    }
}

/// The composition test proper: a handler that both counts every event it
/// sees and installs itself for an async search, while another thread
/// interrupts in a tight loop. The counts are not asserted precisely (the
/// point is that the internal finish handling, which sets the phase back
/// to `Idle`, always still runs, checked indirectly through
/// `assert_next_solve_is_clean`); what would fail here is exactly the bug
/// the safety analysis calls out: the internal step running only when
/// the user handler also fires or only when it returns `Continue`.
#[test]
fn an_interrupt_racing_an_async_search_with_a_user_handler_never_reaches_a_later_solve() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let mut ctl = grounded(SHORT);
    for round in 0..ROUNDS {
        let spinner = Spinner::start(ctl.interrupt_handle());
        for _ in 0..SOLVES_PER_ROUND {
            let mut handle = ctl.solve_async_with_events(&[], NoOp).unwrap();
            handle.wait(Duration::from_secs(5));
            let _result = handle.close().unwrap();
        }
        spinner.finish();
        assert_next_solve_is_clean(&mut ctl, round);
    }
}

// ---------------------------------------------------------------------------
// `AsyncSolveHandle::core` called while the search is still running.
// ---------------------------------------------------------------------------

/// A pigeonhole program that takes a long time to prove unsatisfiable (`n+1`
/// pigeons in `n` holes), so a search on it stays "running" long enough for
/// another thread to call `core()` before it finishes.
fn pigeonhole(holes: u32) -> String {
    let pigeons = holes + 1;
    let mut program = format!("hole(1..{holes}). pigeon(1..{pigeons}).\n");
    program.push_str("1 { at(P,H) : hole(H) } 1 :- pigeon(P).\n");
    program.push_str(":- at(P1,H), at(P2,H), P1 < P2, hole(H), pigeon(P1), pigeon(P2).\n");
    program
}

#[test]
fn async_solve_handle_core_can_be_called_while_the_search_is_still_running() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    // Nine holes take about a second, against about 30 ms for seven: a runner that
    // stalls this thread for longer than that between starting the search and the
    // first read would otherwise find the search over and read nothing.
    let program = pigeonhole(9);
    let mut ctl = Control::with_args(["-t", "1"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    // No assumptions: the program is unsatisfiable by the pigeonhole
    // principle alone, so `core()` is expected to stay empty throughout
    // (`core_is_empty_when_unsat_does_not_come_from_assumptions`'s own
    // finding, reused here); its content is not the point of this test.
    // What matters is that calling it while the search is still active on
    // clasp's own thread neither crashes nor deadlocks, with or without
    // ThreadSanitizer. The caller's thread is the reader: `Control` is not
    // `Sync` (DESIGN S12), so the handle cannot be shared with a second Rust
    // thread, and clasp's thread is the concurrent party.
    let mut handle = ctl.solve_async(&[]).unwrap();
    let mut reads_while_running = 0_u32;
    while !handle.wait(Duration::ZERO) {
        assert!(
            handle.core().unwrap().is_empty(),
            "no core before the search is known to be unsatisfiable"
        );
        reads_while_running += 1;
    }
    assert!(
        reads_while_running > 0,
        "the search ran long enough to read the core while it was running"
    );

    assert!(
        handle.get().unwrap().is_unsat(),
        "pigeonhole is unsatisfiable"
    );
    assert!(
        handle.core().unwrap().is_empty(),
        "no assumptions were given"
    );
}

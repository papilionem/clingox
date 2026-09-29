//! Bounding a search in time: `InterruptHandle`, and `solve_with` with a
//! timeout.
//!
//! clingo 5.8.2 queues an interrupt that arrives while clasp is not searching
//! and kills the next solve with it (checked with the Python module: after
//! `ctl.interrupt()` on an idle control, during grounding, or on a yield or
//! async handle whose search had finished, the next `ctl.solve()` came back
//! interrupted). clingox must never let an interrupt reach a later solve
//! (DESIGN S13); most tests below end by checking exactly that.
//!
//! No test sleeps for correctness. `MEDIUM` and `MANY_MODELS` are long enough
//! that an interrupt or timeout arrives while they still run; if it is lost,
//! the search ends by itself and an assertion fails, so no test hangs. The
//! running times are in the contract.

#![forbid(unsafe_code)]

use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clingox::{
    Assumption, Control, ErrorKind, FunctionCall, InterruptHandle, Part, SolveOptions, SolveResult,
    Symbol,
};

/// 11 pigeons in 10 holes: unsatisfiable, and clingo 5.8.2 needs about 17 s
/// natively to prove it.
const MEDIUM: &str =
    "p(1..11). h(1..10). 1 { at(P,H) : h(H) } 1 :- p(P). :- at(P,H), at(Q,H), P < Q.";

/// 2^25 models; enumerating them all takes clingo 5.8.2 about 40 s natively.
const MANY_MODELS: &str = "{p(1..25)}.";

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn at(pigeon: i32, hole: i32) -> Symbol {
    Symbol::function("at", &[Symbol::number(pigeon), Symbol::number(hole)])
        .expect("the name has no NUL byte")
}

/// Two pigeons in one hole: `MEDIUM` under these assumptions is unsatisfiable
/// by propagation alone, so the solve ends at once.
fn conflicting() -> [Assumption; 2] {
    [(at(1, 1), true).into(), (at(2, 1), true).into()]
}

/// Solves `MEDIUM` under `conflicting()` and checks that no interrupt from
/// before reached this solve.
fn assert_next_solve_not_interrupted(ctl: &mut Control) {
    let result = ctl
        .solve(&conflicting())
        .expect("the solve under assumptions runs");
    assert!(
        !result.is_interrupted(),
        "a stale interrupt reached this solve"
    );
    assert!(result.is_unsat());
}

fn assert_interrupted_without_model(result: SolveResult) {
    assert!(result.is_interrupted(), "{result:?}");
    assert!(result.is_unknown(), "{result:?}");
    assert!(!result.is_exhausted(), "{result:?}");
}

// ---------------------------------------------------------------------------
// InterruptHandle

#[test]
fn interrupt_handles_can_go_anywhere() {
    fn handle_traits<T: Send + Sync + Clone + std::fmt::Debug + 'static>() {}
    handle_traits::<InterruptHandle>();
    let ctl = Control::new().unwrap();
    let text = format!("{:?}", ctl.interrupt_handle());
    assert!(text.contains("InterruptHandle"), "{text}");
}

#[test]
fn an_interrupt_while_idle_returns_false_and_does_nothing() {
    let mut ctl = Control::new().unwrap();
    let stop = ctl.interrupt_handle();
    assert!(!stop.interrupt(), "nothing is solving before grounding");
    ctl.add_base(MEDIUM).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(!stop.interrupt(), "nothing is solving after grounding");
    assert!(!stop.clone().interrupt());
    assert_next_solve_not_interrupted(&mut ctl);
}

#[test]
fn an_interrupt_after_the_control_is_dropped_returns_false() {
    let ctl = grounded(&[], "a.");
    let stop = ctl.interrupt_handle();
    drop(ctl);
    assert!(!stop.interrupt());
}

#[test]
fn an_interrupt_during_grounding_does_not_reach_the_next_solve() {
    // Grounding cannot be interrupted (S13). The handle is used from inside a
    // ground callback, so this runs on one thread.
    let mut ctl = Control::new().unwrap();
    let stop = ctl.interrupt_handle();
    ctl.add_base(&format!("{MEDIUM} q(@f()).")).unwrap();
    let mut returned = Vec::new();
    ctl.ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
        returned.push(stop.interrupt());
        call.push(Symbol::number(1))
    })
    .unwrap();
    assert_eq!(returned, vec![false]);
    assert_next_solve_not_interrupted(&mut ctl);
}

#[test]
fn an_interrupt_from_the_model_closure_stops_the_search() {
    let mut ctl = grounded(&["--models=0"], "{a;b;c}.");
    let stop = ctl.interrupt_handle();
    let mut returned = Vec::new();
    let result = ctl
        .for_each_model(&[], |_| {
            // A clone shares the handle's state.
            returned.push(stop.clone().interrupt());
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(returned, vec![true], "one model, then the search stopped");
    assert!(result.is_sat());
    assert!(result.is_interrupted());
    assert!(!result.is_exhausted());

    let result = ctl.solve(&[]).unwrap();
    assert!(!result.is_interrupted());
    assert!(result.is_exhausted());
}

#[test]
fn an_interrupt_with_a_model_current_ends_a_yield_search() {
    let mut ctl = grounded(&["--models=0"], "{a;b;c}.");
    let stop = ctl.interrupt_handle();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    assert!(stop.interrupt());
    assert!(handle.next_model().unwrap().is_none());
    let result = handle.close().unwrap();
    assert!(result.is_sat());
    assert!(result.is_interrupted());
    assert!(!result.is_exhausted());

    let result = ctl.solve(&[]).unwrap();
    assert!(!result.is_interrupted());
    assert!(result.is_exhausted());
}

#[test]
fn an_interrupt_after_the_search_finished_does_not_reach_the_next_solve() {
    // The handle is still open, but clasp has finished: clingo would queue
    // this interrupt for the next solve.
    let mut ctl = grounded(&["--models=0"], "{a}.");
    let stop = ctl.interrupt_handle();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut models = 0;
    while handle.next_model().unwrap().is_some() {
        models += 1;
        assert!(models <= 2, "{{a}} has two models");
    }
    assert!(handle.get().unwrap().is_exhausted());
    // Either answer is allowed here; only the effect on the next solve counts.
    let _ = stop.interrupt();
    let result = handle.close().unwrap();
    assert!(result.is_exhausted());

    let result = ctl.solve(&[]).unwrap();
    assert!(
        !result.is_interrupted(),
        "a stale interrupt reached this solve"
    );
    assert!(result.is_exhausted());
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn an_interrupt_from_another_thread_stops_a_blocking_solve() {
    let mut ctl = grounded(&[], MEDIUM);
    let stop = ctl.interrupt_handle();
    let done = Arc::new(AtomicBool::new(false));
    // The other thread tries until an interrupt is accepted, which happens
    // once the solve below is running, or gives up when it has returned.
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
    let result = ctl.solve(&[]).unwrap();
    done.store(true, Ordering::SeqCst);
    let delivered = spinner.join().expect("the other thread does not panic");

    assert!(delivered, "interrupt() returned true while the solve ran");
    assert_interrupted_without_model(result);
    assert_next_solve_not_interrupted(&mut ctl);
}

// ---------------------------------------------------------------------------
// SolveOptions and timeouts

#[test]
fn solve_with_default_options_solves_like_solve() {
    let mut ctl = grounded(&[], "a :- not b. b :- not a.");
    let result = ctl.solve_with(SolveOptions::new()).unwrap();
    assert!(result.is_sat());
    assert!(!result.is_interrupted());
    let result = ctl.solve_with(SolveOptions::default()).unwrap();
    assert!(result.is_sat());
}

#[test]
fn solve_with_applies_its_assumptions() {
    let mut ctl = grounded(&[], "a :- not b. b :- not a.");
    let a = Symbol::function("a", &[]).unwrap();
    let b = Symbol::function("b", &[]).unwrap();
    let both = [(a, true).into(), (b, true).into()];
    assert!(
        ctl.solve_with(SolveOptions::new().assumptions(&both))
            .unwrap()
            .is_unsat()
    );
    assert!(
        ctl.solve_with(SolveOptions::new().assumptions(&[(a, true).into()]))
            .unwrap()
            .is_sat()
    );
}

#[test]
fn solve_with_refuses_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let err = ctl.solve_with(SolveOptions::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "timeouts need threads, which the default WASM build lacks"
)]
fn a_timeout_interrupts_a_long_search() {
    let mut ctl = grounded(&[], MEDIUM);
    let options = SolveOptions::new().timeout(Duration::from_millis(200));
    let result = ctl.solve_with(options).unwrap();
    assert_interrupted_without_model(result);
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert_next_solve_not_interrupted(&mut ctl);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "timeouts need threads, which the default WASM build lacks"
)]
fn a_timeout_keeps_the_models_found_before_it() {
    let mut ctl = grounded(&["--models=0"], MANY_MODELS);
    let options = SolveOptions::new().timeout(Duration::from_secs(1));
    let result = ctl.solve_with(options).unwrap();
    assert!(result.is_sat(), "{result:?}");
    assert!(result.is_interrupted(), "{result:?}");
    assert!(!result.is_exhausted(), "{result:?}");
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "timeouts need threads, which the default WASM build lacks"
)]
fn a_search_that_ends_within_its_budget_returns_at_once() {
    let mut ctl = grounded(&[], "a.");
    let started = Instant::now();
    let options = SolveOptions::new().timeout(Duration::from_secs(120));
    let result = ctl.solve_with(options).unwrap();
    // `a.` solves in microseconds; 30 s is a generous bound that still proves
    // the call did not wait for its budget.
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
    assert!(result.is_sat());
    assert!(!result.is_interrupted());
    // A timeout that never fired leaves nothing behind either.
    let result = ctl.solve(&[]).unwrap();
    assert!(!result.is_interrupted());
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "timeouts need threads, which the default WASM build lacks"
)]
fn a_timeout_applies_the_assumptions_too() {
    let mut ctl = grounded(&[], MEDIUM);
    let options = SolveOptions::new()
        .assumptions(&conflicting())
        .timeout(Duration::from_secs(120));
    let result = ctl.solve_with(options).unwrap();
    assert!(result.is_unsat());
    assert!(!result.is_interrupted());
}

#[test]
#[cfg_attr(
    not(all(target_family = "wasm", not(target_feature = "atomics"))),
    ignore = "only a build without threads lacks timeouts"
)]
fn a_timeout_is_unsupported_without_threads() {
    let mut ctl = grounded(&[], "a.");
    let options = SolveOptions::new().timeout(Duration::from_secs(1));
    let err = ctl.solve_with(options).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(ctl.solve_with(SolveOptions::new()).unwrap().is_sat());
}

#[test]
fn solve_options_are_values() {
    fn option_traits<T: Clone + Default + std::fmt::Debug>() {}
    option_traits::<SolveOptions>();
    let options = SolveOptions::new().timeout(Duration::from_secs(5));
    let text = format!("{options:?}");
    assert!(text.contains("SolveOptions"), "{text}");
}

// ---------------------------------------------------------------------------
// Debug shows whether a search runs

#[test]
fn interrupt_handle_debug_shows_whether_a_search_runs() {
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    let stop = ctl.interrupt_handle();
    assert_eq!(format!("{stop:?}"), "InterruptHandle { solving: false }");
    {
        let mut handle = ctl.solve_yield(&[]).unwrap();
        assert!(handle.next_model().unwrap().is_some());
        assert_eq!(format!("{stop:?}"), "InterruptHandle { solving: true }");
        let _ = handle.close().unwrap();
    }
    assert_eq!(format!("{stop:?}"), "InterruptHandle { solving: false }");
}

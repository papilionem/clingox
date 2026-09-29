//! Solving in the background: `solve_async` and `AsyncSolveHandle`.
//!
//! Semantics were checked with the Python module `clingo` 5.8.2
//! (`ctl.solve(async_=True)`, `wait`, `cancel`, `get`). The search runs on
//! clasp's own thread, so these tests need threads, which the default WASM
//! build lacks; there `solve_async` is unsupported, and one test checks that.
//!
//! No test sleeps for correctness. `MEDIUM` runs for about 17 s natively, so
//! it is still running when a test polls or cancels it; a lost cancel or
//! interrupt shows as an unsatisfiable, uninterrupted result after that time,
//! not as a hang.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use clingox::{Assumption, Control, ErrorKind, Part, SolveResult, Symbol};

/// 11 pigeons in 10 holes: unsatisfiable, and clingo 5.8.2 needs about 17 s
/// natively to prove it.
const MEDIUM: &str =
    "p(1..11). h(1..10). 1 { at(P,H) : h(H) } 1 :- p(P). :- at(P,H), at(Q,H), P < Q.";

/// Generous: the searches that must finish within it take microseconds.
const PATIENCE: Duration = Duration::from_secs(60);

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
/// by propagation alone.
fn conflicting() -> [Assumption; 2] {
    [(at(1, 1), true).into(), (at(2, 1), true).into()]
}

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

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "async solving needs threads"
)]
fn wait_reports_a_finished_search_at_once() {
    let mut ctl = grounded(&[], "a.");
    let mut handle = ctl.solve_async(&[]).unwrap();
    let started = Instant::now();
    assert!(handle.wait(PATIENCE));
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "wait returns when the search ends"
    );
    let result = handle.get().unwrap();
    assert!(result.is_sat());
    assert!(!result.is_interrupted());
    assert!(
        handle.wait(Duration::ZERO),
        "a finished search stays finished"
    );
    assert_eq!(
        handle.get().unwrap(),
        result,
        "get returns the same result again"
    );
    assert_eq!(handle.close().unwrap(), result);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "async solving needs threads"
)]
fn get_waits_for_the_result() {
    let mut ctl = grounded(&[], "a :- not a.");
    let mut handle = ctl.solve_async(&[]).unwrap();
    let result = handle.get().unwrap();
    assert!(result.is_unsat());
    assert!(handle.wait(Duration::ZERO));
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "async solving needs threads"
)]
fn wait_returns_false_while_the_search_runs() {
    let mut ctl = grounded(&[], MEDIUM);
    let mut handle = ctl.solve_async(&[]).unwrap();
    assert!(!handle.wait(Duration::ZERO), "MEDIUM runs for seconds");
    assert!(!handle.wait(Duration::from_millis(50)));
    handle.cancel().unwrap();
    assert!(
        handle.wait(Duration::ZERO),
        "a cancelled search is finished"
    );
    assert_interrupted_without_model(handle.get().unwrap());
    drop(handle);
    assert_next_solve_not_interrupted(&mut ctl);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "async solving needs threads"
)]
fn a_cancelled_search_reports_the_models_it_found() {
    // 2^25 models take about 40 s to enumerate; the first is found at once.
    let mut ctl = grounded(&["--models=0"], "{p(1..25)}.");
    let mut handle = ctl.solve_async(&[]).unwrap();
    assert!(!handle.wait(Duration::from_millis(200)));
    handle.cancel().unwrap();
    let result = handle.close().unwrap();
    assert!(result.is_sat(), "{result:?}");
    assert!(result.is_interrupted(), "{result:?}");
    assert!(!result.is_exhausted(), "{result:?}");
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "async solving needs threads"
)]
fn solve_async_applies_assumptions() {
    let mut ctl = grounded(&[], "a :- not b. b :- not a.");
    let a = Symbol::function("a", &[]).unwrap();
    let b = Symbol::function("b", &[]).unwrap();
    let mut handle = ctl
        .solve_async(&[(a, true).into(), (b, true).into()])
        .unwrap();
    assert!(handle.get().unwrap().is_unsat());
    drop(handle);

    let mut ctl = grounded(&[], MEDIUM);
    let mut handle = ctl.solve_async(&conflicting()).unwrap();
    assert!(handle.wait(PATIENCE));
    assert!(handle.get().unwrap().is_unsat());
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "async solving needs threads"
)]
fn an_interrupt_handle_stops_an_async_search() {
    let mut ctl = grounded(&[], MEDIUM);
    let stop = ctl.interrupt_handle();
    let mut handle = ctl.solve_async(&[]).unwrap();
    assert!(stop.interrupt(), "the search is running");
    assert!(handle.wait(PATIENCE));
    assert_interrupted_without_model(handle.close().unwrap());
    assert_next_solve_not_interrupted(&mut ctl);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "async solving needs threads"
)]
fn an_interrupt_after_an_async_search_finished_does_not_reach_the_next_solve() {
    let mut ctl = grounded(&[], MEDIUM);
    let stop = ctl.interrupt_handle();
    let mut handle = ctl.solve_async(&conflicting()).unwrap();
    assert!(handle.wait(PATIENCE));
    // The search is over but the handle is open: clingo would queue this.
    let _ = stop.interrupt();
    let result = handle.close().unwrap();
    assert!(result.is_unsat());
    assert!(!result.is_interrupted());
    assert_next_solve_not_interrupted(&mut ctl);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "async solving needs threads"
)]
fn dropping_an_async_handle_cancels_the_search() {
    let mut ctl = grounded(&[], MEDIUM);
    let handle = ctl.solve_async(&[]).unwrap();
    drop(handle);
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
    assert_next_solve_not_interrupted(&mut ctl);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "async solving needs threads"
)]
fn a_forgotten_async_handle_is_closed_by_the_next_call() {
    // DESIGN S4: every entry point, `&self` ones included, first closes a
    // leftover search. Reading statistics during a search is forbidden.
    let mut ctl = grounded(&[], MEDIUM);
    let handle = ctl.solve_async(&[]).unwrap();
    std::mem::forget(handle);
    {
        let stats = ctl.statistics().unwrap();
        assert!(stats.value("summary.times.total").unwrap() >= 0.0);
    }
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
    assert_next_solve_not_interrupted(&mut ctl);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "async solving needs threads"
)]
fn async_handles_are_debug() {
    let mut ctl = grounded(&[], "a.");
    let handle = ctl.solve_async(&[]).unwrap();
    let text = format!("{handle:?}");
    assert!(text.contains("AsyncSolveHandle"), "{text}");
}

#[test]
fn solve_async_refuses_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let err = ctl.solve_async(&[]).map(|_| ()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

#[test]
#[cfg_attr(
    not(all(target_family = "wasm", not(target_feature = "atomics"))),
    ignore = "only a build without threads lacks async solving"
)]
fn solve_async_is_unsupported_without_threads() {
    let mut ctl = grounded(&[], "a.");
    let err = ctl.solve_async(&[]).map(|_| ()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    // clingo itself would have raised a logic error, which poisons; clingox
    // refuses before calling it.
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

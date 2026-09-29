//! `SolveEventHandler`, `ExtendableModel` and the three event-taking solve
//! entry points: `Control::solve_with_events`,
//! `Control::solve_yield_with_events`, `Control::solve_async_with_events`.
//! See DESIGN S6 for the contract these tests are written against.
//!
//! **A test design hazard, read before touching this file.** clingo's own
//! internal `ClingoSolveEventHandler` (`control.cc:1988-2020`) calls
//! `clingo_terminate` — an unconditional `std::_Exit(1)`, no unwinding — if
//! the trampoline ever returns `false` for the unsat, statistics or finish
//! event. If the implementation regresses to the naive "Err maps to
//! `false`" rule for these three events (safe only for the model event),
//! the test that would catch it does not fail normally: it kills the
//! whole test binary that was running it, including every other test
//! sharing that process, with no report at all. The tests below that probe
//! this (`an_err_from_on_unsat_does_not_abort_the_process` and its
//! `on_statistics`/`on_finish`/panic siblings) therefore run the dangerous
//! call in a **child process**, re-executing this same test binary
//! (`patch_u19_statistics_registry.rs`'s own pattern, for the same reason:
//! a failure mode that cannot report itself from inside the process where
//! it happens). A parent test spawns one child per scenario and checks the
//! child's own exit status and a completion marker on its stdout; the
//! child is a plain `#[test]` gated by an environment variable, so `cargo
//! test` alone never runs the dangerous half twice.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::float_cmp,
    reason = "the values compared exactly are the ones clingo reports as counts"
)]
#![allow(
    clippy::items_after_statements,
    reason = "each handler is defined next to the test that uses it"
)]
#![allow(
    clippy::manual_assert,
    reason = "the handlers panic on purpose, with a plain message as the payload"
)]

use std::error::Error as _;
use std::io::Write;
use std::ops::ControlFlow;
use std::panic::{self, AssertUnwindSafe};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use clingox::{
    Control, ErrorKind, ExtendableModel, MutableStatistics, Part, Result as ClingoxResult,
    ShowType, SolveEventHandler, SolveResult, Symbol,
};

fn grounded(program: &str) -> Control {
    // Unbounded enumeration: clingo's default `--models=1` would end every
    // search after its first model.
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl
}

// ---------------------------------------------------------------------------
// on_model: fires once per model, matches `for_each_model`, `Break` stops
// the search early exactly as `for_each_model`'s own early-stop test does.
//
// `solve_with_events` does not hand the handler back once the search
// closes, so every test below that needs to read what the handler saw
// shares a `Vec` with it through an `Arc<Mutex<_>>` instead.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SharedModels(Arc<Mutex<Vec<Vec<String>>>>);

impl SolveEventHandler for SharedModels {
    fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> ClingoxResult<ControlFlow<()>> {
        let mut atoms: Vec<String> = model
            .symbols(ShowType::SHOWN)?
            .iter()
            .map(ToString::to_string)
            .collect();
        atoms.sort();
        self.0.lock().unwrap().push(atoms);
        Ok(ControlFlow::Continue(()))
    }
}

#[test]
fn on_model_reports_the_same_models_for_each_model_would_see() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut ctl = grounded("{a;b}.");
    let _ = ctl
        .solve_with_events(
            clingox::SolveOptions::new(),
            SharedModels(Arc::clone(&seen)),
        )
        .unwrap();
    let mut via_events: Vec<Vec<String>> = seen.lock().unwrap().clone();
    via_events.sort();

    let mut ctl2 = grounded("{a;b}.");
    let mut via_for_each_model = Vec::new();
    let _ = ctl2
        .for_each_model(&[], |model| {
            let mut atoms: Vec<String> = model
                .symbols(ShowType::SHOWN)?
                .iter()
                .map(ToString::to_string)
                .collect();
            atoms.sort();
            via_for_each_model.push(atoms);
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    via_for_each_model.sort();

    assert_eq!(via_events, via_for_each_model);
    assert_eq!(via_events.len(), 4);
}

struct BreakAfter {
    remaining: usize,
    seen: Arc<AtomicUsize>,
}

impl SolveEventHandler for BreakAfter {
    fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> ClingoxResult<ControlFlow<()>> {
        self.seen.fetch_add(1, Ordering::SeqCst);
        if self.remaining == 0 {
            return Ok(ControlFlow::Break(()));
        }
        self.remaining -= 1;
        Ok(ControlFlow::Continue(()))
    }
}

#[test]
fn break_from_on_model_stops_the_search_early_as_clingo_reports_it() {
    let seen = Arc::new(AtomicUsize::new(0));
    let mut ctl = grounded("{a;b}.");
    let handler = BreakAfter {
        remaining: 1,
        seen: Arc::clone(&seen),
    };
    let result = ctl
        .solve_with_events(clingox::SolveOptions::new(), handler)
        .unwrap();
    // Oracle (pyclingo 5.8.2, `--models=0`, `{a;b}.`, `on_model` returning
    // `False` on the second model): two models seen, and the result is
    // satisfiable, neither exhausted nor interrupted. Stopping through
    // `goon` is not an interrupt, unlike `for_each_model`'s `Break`, which
    // cancels the search).
    assert!(result.is_sat());
    assert!(!result.is_exhausted());
    assert!(!result.is_interrupted());
    assert_eq!(
        seen.load(Ordering::SeqCst),
        2,
        "the model that triggers Break is still seen"
    );
}

// ---------------------------------------------------------------------------
// ExtendableModel::extend: added symbols are visible inside the same
// callback via `ExtendableModel`'s `Model` methods (S6, `H:2431-2434`'s own
// caveat that the addition is otherwise only meaningful to an application);
// test plus the Miri/ASan run.
// ---------------------------------------------------------------------------

struct ExtendAndReadBack {
    read_back: Arc<Mutex<Vec<String>>>,
}

impl SolveEventHandler for ExtendAndReadBack {
    fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> ClingoxResult<ControlFlow<()>> {
        model.extend([Symbol::number(99)])?;
        let mut atoms: Vec<String> = model
            .symbols(ShowType::SHOWN | ShowType::THEORY)?
            .iter()
            .map(ToString::to_string)
            .collect();
        atoms.sort();
        *self.read_back.lock().unwrap() = atoms;
        Ok(ControlFlow::Break(()))
    }
}

#[test]
fn extend_adds_symbols_that_appear_in_the_same_callbacks_symbols() {
    let read_back = Arc::new(Mutex::new(Vec::new()));
    let mut ctl = grounded("a.");
    let _ = ctl
        .solve_with_events(
            clingox::SolveOptions::new(),
            ExtendAndReadBack {
                read_back: Arc::clone(&read_back),
            },
        )
        .unwrap();
    assert_eq!(*read_back.lock().unwrap(), vec!["99", "a"]);
}

// ---------------------------------------------------------------------------
// on_unsat: the lower-bound array reported during core-guided optimisation.
// Ported from `test_control.py::test_lower` (checked directly against the
// installed clingo 5.8.2: `--opt-str=usc,oll,0 --stats=2`, the program
// below, `on_unsat=lower.append` reports `[[1], [2], [3]]`, the result is
// satisfiable, and `statistics["summary"]["lower"]` ends at `[3.0]`).
// ---------------------------------------------------------------------------

#[test]
fn on_unsat_reports_the_lower_bound_during_core_guided_optimisation() {
    let seen: Arc<Mutex<Vec<Vec<i64>>>> = Arc::default();

    struct Shared(Arc<Mutex<Vec<Vec<i64>>>>);
    impl SolveEventHandler for Shared {
        fn on_unsat(&mut self, lower_bound: &[i64]) -> ClingoxResult<ControlFlow<()>> {
            self.0.lock().unwrap().push(lower_bound.to_vec());
            Ok(ControlFlow::Continue(()))
        }
    }

    let mut ctl = Control::with_args(["--opt-str=usc,oll,0", "--stats=2"]).unwrap();
    ctl.add_base("1 { p(X); q(X) } 1 :- X=1..3. #minimize { 1,p,X: p(X); 1,q,X: q(X) }.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let result = ctl
        .solve_with_events(clingox::SolveOptions::new(), Shared(Arc::clone(&seen)))
        .unwrap();

    assert!(
        result.is_sat(),
        "the program is satisfiable, not unsat overall"
    );
    assert_eq!(*seen.lock().unwrap(), vec![vec![1], vec![2], vec![3]]);
    let stats = ctl.statistics().unwrap();
    assert_eq!(stats.value("summary.lower.0").unwrap(), 3.0);
}

// ---------------------------------------------------------------------------
// on_finish: the same `SolveResult` `get`/`close` would see for the same
// search.
// ---------------------------------------------------------------------------

#[test]
fn on_finish_receives_the_same_result_get_would() {
    struct RecordFinish(Arc<Mutex<Option<SolveResult>>>);
    impl SolveEventHandler for RecordFinish {
        fn on_finish(&mut self, result: SolveResult) -> ClingoxResult<ControlFlow<()>> {
            *self.0.lock().unwrap() = Some(result);
            Ok(ControlFlow::Continue(()))
        }
    }

    let seen: Arc<Mutex<Option<SolveResult>>> = Arc::default();
    let mut ctl = grounded("a :- not a.");
    let result = ctl
        .solve_with_events(
            clingox::SolveOptions::new(),
            RecordFinish(Arc::clone(&seen)),
        )
        .unwrap();
    assert!(result.is_unsat());
    assert_eq!(seen.lock().unwrap().unwrap(), result);
}

// ---------------------------------------------------------------------------
// Break vs Err, and a panic in `on_model`: the safe half (the model event's
// `false` return takes clingo's ordinary error path, never `clingo_terminate`,
// per `control.cc:1995-2000`), so these run in-process like any other test.
// ---------------------------------------------------------------------------

#[test]
fn an_err_from_on_model_is_returned_not_swallowed_and_does_not_poison() {
    struct Failing;
    impl SolveEventHandler for Failing {
        fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> ClingoxResult<ControlFlow<()>> {
            Err(clingox::Error::callback(std::io::Error::other(
                "on_model failed",
            )))
        }
    }

    let mut ctl = grounded("a.");
    let err = ctl
        .solve_with_events(clingox::SolveOptions::new(), Failing)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
    // `Error::callback`'s `Display` names the operation only (DESIGN S2);
    // the wrapped message is the source.
    assert!(
        err.source()
            .is_some_and(|source| source.to_string().contains("on_model failed")),
        "{err}"
    );
    // Mirrors `for_each_model`'s own "does not poison the control" contract
    // for a closure's own error (`src/solve.rs`'s doc on `for_each_model`).
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_panic_in_on_model_resumes_on_the_callers_thread() {
    struct Panicking;
    impl SolveEventHandler for Panicking {
        fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> ClingoxResult<ControlFlow<()>> {
            panic!("on_model panicked on purpose");
        }
    }

    let mut ctl = grounded("a.");
    let unwound = panic::catch_unwind(AssertUnwindSafe(|| {
        ctl.solve_with_events(clingox::SolveOptions::new(), Panicking)
    }))
    .expect_err("the panic resumes from solve_with_events");
    assert_eq!(
        unwound.downcast_ref::<&str>(),
        Some(&"on_model panicked on purpose")
    );
}

// ---------------------------------------------------------------------------
// The dangerous half: Err and a panic from on_unsat, on_statistics and
// on_finish must reach the caller as an error or a resumed panic, and must
// never abort the process. See the file-level doc comment for why each
// scenario runs in a child process.
// ---------------------------------------------------------------------------

/// The environment variable that puts this binary in child mode, one
/// scenario per value (mirrors `patch_u19_statistics_registry.rs`'s own
/// `CLINGOX_TEST_U19_CHILD` convention, generalised to name which scenario
/// to run).
const CHILD_SCENARIO: &str = "CLINGOX_TEST_SOLVE_EVENTS_CHILD";
const DONE: &str = "solve-events-child-done";

/// A program whose search fires every one of the four solve events at
/// least once: an optimisation problem, solved with a core-guided
/// strategy, under `--stats` so a statistics event has entries to report.
fn events_program() -> Control {
    let mut ctl = Control::with_args(["--opt-str=usc,oll,0", "--stats=2"]).unwrap();
    ctl.add_base("1 { p(X); q(X) } 1 :- X=1..3. #minimize { 1,p,X: p(X); 1,q,X: q(X) }.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl
}

/// A handler each of whose three dangerous-event methods panics when its
/// matching flag is set. Kept separate from `ErrOn` below: a method that
/// always panics cannot also sometimes return `Err`.
struct FailOn {
    unsat: bool,
    statistics: bool,
    finish: bool,
}

impl SolveEventHandler for FailOn {
    fn on_unsat(&mut self, _lower_bound: &[i64]) -> ClingoxResult<ControlFlow<()>> {
        if self.unsat {
            panic!("on_unsat panicked on purpose");
        }
        Ok(ControlFlow::Continue(()))
    }

    fn on_statistics(
        &mut self,
        _step: &mut MutableStatistics<'_>,
        _accumulated: &mut MutableStatistics<'_>,
    ) -> ClingoxResult<ControlFlow<()>> {
        if self.statistics {
            panic!("on_statistics panicked on purpose");
        }
        Ok(ControlFlow::Continue(()))
    }

    fn on_finish(&mut self, _result: SolveResult) -> ClingoxResult<ControlFlow<()>> {
        if self.finish {
            panic!("on_finish panicked on purpose");
        }
        Ok(ControlFlow::Continue(()))
    }
}

/// The `Err`-returning counterpart of `FailOn`.
struct ErrOn {
    unsat: bool,
    statistics: bool,
    finish: bool,
}

impl SolveEventHandler for ErrOn {
    fn on_unsat(&mut self, _lower_bound: &[i64]) -> ClingoxResult<ControlFlow<()>> {
        if self.unsat {
            return Err(clingox::Error::callback(std::io::Error::other(
                "on_unsat failed",
            )));
        }
        Ok(ControlFlow::Continue(()))
    }

    fn on_statistics(
        &mut self,
        _step: &mut MutableStatistics<'_>,
        _accumulated: &mut MutableStatistics<'_>,
    ) -> ClingoxResult<ControlFlow<()>> {
        if self.statistics {
            return Err(clingox::Error::callback(std::io::Error::other(
                "on_statistics failed",
            )));
        }
        Ok(ControlFlow::Continue(()))
    }

    fn on_finish(&mut self, _result: SolveResult) -> ClingoxResult<ControlFlow<()>> {
        if self.finish {
            return Err(clingox::Error::callback(std::io::Error::other(
                "on_finish failed",
            )));
        }
        Ok(ControlFlow::Continue(()))
    }
}

/// Runs one dangerous scenario and reports success on stdout, so the
/// parent test can tell "the child ran and the contract held" apart from
/// "the child's process died," which is exactly what a regression to the
/// naive `false`-on-`Err` trampoline looks like (`clingo_terminate`, an
/// unconditional `std::_Exit(1)`).
#[allow(
    clippy::too_many_lines,
    reason = "one arm per scenario, kept side by side"
)]
fn run_scenario(name: &str) {
    match name {
        "err-unsat" => {
            let mut ctl = events_program();
            let err = ctl
                .solve_with_events(
                    clingox::SolveOptions::new(),
                    ErrOn {
                        unsat: true,
                        statistics: false,
                        finish: false,
                    },
                )
                .expect_err("on_unsat's error reaches the caller");
            assert!(
                err.source()
                    .is_some_and(|source| source.to_string().contains("on_unsat failed")),
                "{err}"
            );
        }
        "err-statistics" => {
            let mut ctl = events_program();
            let err = ctl
                .solve_with_events(
                    clingox::SolveOptions::new(),
                    ErrOn {
                        unsat: false,
                        statistics: true,
                        finish: false,
                    },
                )
                .expect_err("on_statistics's error reaches the caller");
            assert!(
                err.source()
                    .is_some_and(|source| source.to_string().contains("on_statistics failed")),
                "{err}"
            );
        }
        "err-finish" => {
            let mut ctl = events_program();
            let err = ctl
                .solve_with_events(
                    clingox::SolveOptions::new(),
                    ErrOn {
                        unsat: false,
                        statistics: false,
                        finish: true,
                    },
                )
                .expect_err("on_finish's error reaches the caller");
            assert!(
                err.source()
                    .is_some_and(|source| source.to_string().contains("on_finish failed")),
                "{err}"
            );
        }
        "panic-unsat" => {
            let mut ctl = events_program();
            let unwound = panic::catch_unwind(AssertUnwindSafe(|| {
                ctl.solve_with_events(
                    clingox::SolveOptions::new(),
                    FailOn {
                        unsat: true,
                        statistics: false,
                        finish: false,
                    },
                )
            }))
            .expect_err("the panic resumes");
            assert_eq!(
                unwound.downcast_ref::<&str>().copied(),
                Some("on_unsat panicked on purpose")
            );
        }
        "panic-statistics" => {
            let mut ctl = events_program();
            let unwound = panic::catch_unwind(AssertUnwindSafe(|| {
                ctl.solve_with_events(
                    clingox::SolveOptions::new(),
                    FailOn {
                        unsat: false,
                        statistics: true,
                        finish: false,
                    },
                )
            }))
            .expect_err("the panic resumes");
            assert_eq!(
                unwound.downcast_ref::<&str>().copied(),
                Some("on_statistics panicked on purpose")
            );
        }
        "panic-finish" => {
            let mut ctl = events_program();
            let unwound = panic::catch_unwind(AssertUnwindSafe(|| {
                ctl.solve_with_events(
                    clingox::SolveOptions::new(),
                    FailOn {
                        unsat: false,
                        statistics: false,
                        finish: true,
                    },
                )
            }))
            .expect_err("the panic resumes");
            assert_eq!(
                unwound.downcast_ref::<&str>().copied(),
                Some("on_finish panicked on purpose")
            );
        }
        other => panic!("unknown scenario {other}"),
    }
}

/// The same scenarios in process, for targets that cannot spawn a child. A
/// regression here ends the whole test binary instead of failing one test,
/// which is still a failure.
#[test]
#[cfg_attr(
    not(any(target_os = "android", target_os = "ios", target_family = "wasm")),
    ignore = "the child-process guard below covers this where processes can be spawned"
)]
fn an_err_or_panic_from_on_unsat_on_statistics_or_on_finish_reaches_the_caller_in_process() {
    for scenario in [
        "err-unsat",
        "err-statistics",
        "err-finish",
        "panic-unsat",
        "panic-statistics",
        "panic-finish",
    ] {
        run_scenario(scenario);
    }
}

#[test]
fn solve_events_child() {
    let Ok(scenario) = std::env::var(CHILD_SCENARIO) else {
        return;
    };
    run_scenario(&scenario);
    let mut out = std::io::stdout();
    writeln!(out, "{DONE}").expect("stdout is a pipe to the parent test");
}

#[test]
#[cfg_attr(
    any(target_os = "android", target_os = "ios", target_family = "wasm"),
    ignore = "spawns a child process; the in-process test above runs instead"
)]
fn an_err_or_panic_from_on_unsat_on_statistics_or_on_finish_never_aborts_the_process() {
    if std::env::var_os(CHILD_SCENARIO).is_some() {
        return;
    }
    let exe = std::env::current_exe().expect("the test binary knows its path");
    let scenarios = [
        "err-unsat",
        "err-statistics",
        "err-finish",
        "panic-unsat",
        "panic-statistics",
        "panic-finish",
    ];
    let mut failures = Vec::new();
    for scenario in scenarios {
        let output = Command::new(&exe)
            .args([
                "--exact",
                "solve_events_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_SCENARIO, scenario)
            .output()
            .expect("the test binary runs again");
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || !stdout.contains(DONE) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let report: String = stderr.lines().take(40).collect::<Vec<_>>().join("\n");
            failures.push(format!(
                "{scenario}: {:?} (a non-success exit status here, with no panic message of \
                 its own, is `clingo_terminate`'s `std::_Exit(1)`: the trampoline returned \
                 `false` for an event that must never do so)\n{report}",
                output.status
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} scenarios failed:\n{}",
        failures.len(),
        scenarios.len(),
        failures.join("\n\n")
    );
}

// ---------------------------------------------------------------------------
// Composition, deterministic half: a user handler installed alongside an
// interrupt. The scaled, TSan-oriented version lives in
// `solve_events_interrupt_races.rs`; this is the single-shot functional
// check.
// ---------------------------------------------------------------------------

#[test]
fn a_user_handler_does_not_prevent_an_interrupt_from_being_reported() {
    struct NoOp;
    impl SolveEventHandler for NoOp {}

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b;c}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let stop = ctl.interrupt_handle();
    let mut handle = ctl.solve_yield_with_events(&[], NoOp).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    assert!(stop.interrupt());
    // The interrupted search's own result, then a later, unrelated solve on
    // the same control that must not itself be interrupted (S13's own
    // guarantee, now checked with a handler installed).
    assert!(handle.close().unwrap().is_interrupted());
    assert!(!ctl.solve(&[]).unwrap().is_interrupted());
}

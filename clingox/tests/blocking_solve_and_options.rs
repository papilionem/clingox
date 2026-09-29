//! Blocking solves and solver options on `Control`.
//!
//! Every expected value about clingo's behaviour was checked against the
//! Python module `clingo` 5.8.2: model counts and statistics through
//! `Control.solve`, and the configuration through its C functions
//! (`clingo._internal._lib`).

#![forbid(unsafe_code)]
#![allow(
    clippy::float_cmp,
    reason = "statistics values are whole numbers, exact in an f64"
)]

use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use clingox::{Control, ErrorKind, Part};

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// Two priority levels; clingo 5.8.2 finds two models with `--models=0`, the
/// second optimal with costs `[0, 1]`, and three with `--opt-mode=optN`.
const OPTIMISE: &str = "{p(1..6)}. :- not p(1), not p(2). \
    #minimize{ X@1 : p(X) }. #minimize{ 1@2,X : p(X), X > 3 }.";

fn count_models(ctl: &mut Control) -> u64 {
    let mut models = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            models += 1;
            Ok(ControlFlow::Continue(()))
        })
        .expect("the program solves");
    assert!(result.is_sat() && result.is_exhausted(), "{result:?}");
    models
}

#[test]
fn a_blocking_solve_enumerates_and_counts_like_clingo() {
    let mut ctl = grounded(&["--models=0"], "{p(1..10)}.");
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat() && result.is_exhausted() && !result.is_interrupted());
    let stats = ctl.statistics().unwrap();
    assert_eq!(stats.value("summary.models.enumerated").unwrap(), 1024.0);
    assert_eq!(stats.value("summary.call").unwrap(), 0.0);
    let _ = stats;
    assert_eq!(count_models(&mut ctl), 1024);
    assert_eq!(
        ctl.statistics().unwrap().value("summary.call").unwrap(),
        1.0
    );
}

#[test]
fn a_blocking_solve_optimises_like_clingo() {
    for (args, enumerated) in [
        (&["--models=0"][..], 2.0),
        (&[][..], 2.0),
        (&["--models=0", "--opt-mode=optN"][..], 3.0),
    ] {
        let mut ctl = grounded(args, OPTIMISE);
        let result = ctl.solve(&[]).unwrap();
        assert!(
            result.is_sat() && result.is_exhausted(),
            "{args:?}: {result:?}"
        );
        let stats = ctl.statistics().unwrap();
        assert_eq!(
            stats.value("summary.models.enumerated").unwrap(),
            enumerated,
            "{args:?}"
        );
        assert_eq!(
            stats.value("summary.models.optimal").unwrap(),
            1.0,
            "{args:?}"
        );
        assert_eq!(stats.value("summary.costs.0").unwrap(), 0.0, "{args:?}");
        assert_eq!(stats.value("summary.costs.1").unwrap(), 1.0, "{args:?}");
    }
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn a_parallel_blocking_solve_enumerates_every_model() {
    let mut ctl = Control::builder()
        .threads(4)
        .args(["--models=0"])
        .build()
        .unwrap();
    ctl.add_base("{p(1..12)}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat() && result.is_exhausted());
    assert_eq!(
        ctl.statistics()
            .unwrap()
            .value("summary.models.enumerated")
            .unwrap(),
        4096.0
    );
}

#[test]
fn a_rejected_partial_number_keeps_the_old_value() {
    // clingo 5.8.2 itself leaves `solve.models` at 1 after rejecting "1e3".
    let mut ctl = grounded(&[], "{a;b}.");
    let mut config = ctl.configuration();
    config.set("solve.models", "0").unwrap();
    let err = config.set("solve.models", "1e3").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
    assert_eq!(config.get("solve.models").unwrap().as_deref(), Some("0"));
    let _ = config;
    assert_eq!(count_models(&mut ctl), 4);
}

#[test]
fn a_rejected_value_on_an_unassigned_tester_option_leaves_clingo_default() {
    // clasp creates the tester configuration before it parses the value
    // (clasp_options.cpp:984-992), which assigns every tester option its
    // default, and the C API cannot unassign one. `Configuration::set`
    // documents this; the test pins clingo's actual behaviour.
    let mut ctl = Control::new().unwrap();
    let mut config = ctl.configuration();
    let path = "tester.solver.opt_strategy";
    assert_eq!(config.get(path).unwrap(), None);
    let err = config.set(path, "abc").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
    assert_eq!(config.get(path).unwrap().as_deref(), Some("bb,lin"));
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "timeouts and async solving need threads"
)]
fn the_longest_durations_do_not_overflow() {
    let mut ctl = grounded(&[], "a.");
    let start = Instant::now();
    let result = ctl
        .solve_with(clingox::SolveOptions::new().timeout(Duration::MAX))
        .unwrap();
    assert!(result.is_sat() && !result.is_interrupted());
    let mut handle = ctl.solve_async(&[]).unwrap();
    assert!(handle.wait(Duration::MAX));
    assert!(handle.close().unwrap().is_sat());
    assert!(start.elapsed() < Duration::from_secs(30));
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "timeouts need threads"
)]
fn a_zero_timeout_is_not_an_error_and_does_not_linger() {
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    for _ in 0..200 {
        // The result itself is not checked: clingo 5.8.2 reports a search
        // interrupted at its very start as unsatisfiable and exhausted now
        // and then, for this satisfiable program too.
        let _result = ctl
            .solve_with(clingox::SolveOptions::new().timeout(Duration::ZERO))
            .unwrap();
        let next = ctl.solve(&[]).unwrap();
        assert!(!next.is_interrupted() && next.is_exhausted(), "{next:?}");
    }
}

#[test]
fn a_blocking_solve_passes_on_the_warnings_of_a_blocking_solve() {
    // clingo 5.8.2 warns here in a blocking solve (`Control.solve()`), and
    // not in a yield search (`solve(yield_=True)`).
    let messages = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&messages);
    let mut ctl = Control::builder()
        .args(["--models=1"])
        .logger(move |_, text| sink.lock().unwrap().push(text.to_owned()))
        .build()
        .unwrap();
    ctl.add_base("{a;b}. #minimize{1:a; 1:b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
    let messages = messages.lock().unwrap();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("optimality of last model not guaranteed")),
        "{messages:?}"
    );
}

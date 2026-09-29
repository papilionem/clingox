//! `Outcome::Sat` with its search result, and the option-taking variants
//! `solve_first_with`, `solve_optimal_with` and `solve_all_with` .
//!
//! Models and costs were checked against the Python module `clingo` 5.8.2.
//! The long-running programs were timed with it: within 3 s, clingo 5.8.2
//! finds no model of `HARD_UNSAT`, and finds 17 improving models of
//! `HARD_OPTIMUM` without proving the optimum.

#![forbid(unsafe_code)]

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use clingox::prelude::*;
use clingox::{ErrorKind, SolveOptions};

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn constant(name: &str) -> Symbol {
    Symbol::function(name, &[]).expect("the name has no NUL byte")
}

fn texts(model: &OwnedModel) -> Vec<String> {
    model.symbols().iter().map(ToString::to_string).collect()
}

fn model_limit(ctl: &mut Control) -> String {
    ctl.configuration()
        .get("solve.models")
        .expect("the option exists")
        .expect("clingo always assigns it")
}

fn is_poisoned(ctl: &Control) -> bool {
    format!("{ctl:?}").contains("poisoned")
}

/// Whether this build of clingo has threads, which timeouts need.
fn has_threads() -> bool {
    clingox_sys::HAS_THREADS
}

const FOUR_MODELS: &str = "{a;b}.";

/// Its only optimum is `{c}` with cost 0; clingo 5.8.2 first finds `{}` with
/// cost 3.
const OPTIMISATION: &str = "{ a; b; c }. :~ a. [1] :~ b. [2] :~ not c. [3]";

/// 13 pigeons in 12 holes: unsatisfiable, and far from proven in a second.
const HARD_UNSAT: &str =
    "p(1..13). h(1..12). 1 { a(P,H) : h(H) } 1 :- p(P). :- a(P,H), a(Q,H), P < Q.";

/// The same, minimising collisions: a first model comes at once, and the
/// optimum (one collision) is far from proven in a second.
const HARD_OPTIMUM: &str =
    "p(1..13). h(1..12). 1 { a(P,H) : h(H) } 1 :- p(P). :~ a(P,H), a(Q,H), P < Q. [1,P,Q,H]";

/// Generous: the calls below must return soon after their budget, and a
/// search that ignored it would run for minutes.
const LATE: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Outcome::Sat carries the search result

#[test]
fn a_first_model_comes_with_its_search_result() {
    let mut ctl = grounded(&[], "a.");
    let Outcome::Sat(model, result) = ctl.solve_first().unwrap() else {
        panic!("`a.` has a model");
    };
    assert_eq!(texts(&model), ["a"]);
    assert!(result.is_sat());
    assert!(!result.is_interrupted());
}

#[test]
fn a_proven_optimum_comes_with_an_exhausted_result() {
    let mut ctl = grounded(&[], OPTIMISATION);
    let Outcome::Sat(best, result) = ctl.solve_optimal().unwrap() else {
        panic!("the problem is satisfiable");
    };
    assert_eq!(best.cost(), [0]);
    assert!(best.optimality_proven());
    assert!(result.is_sat() && result.is_exhausted(), "{result:?}");
    assert!(!result.is_interrupted());
}

#[test]
fn outcomes_compare_their_results_too() {
    let mut ctl = grounded(&[], OPTIMISATION);
    let optimal = ctl.solve_optimal().unwrap();
    let Outcome::Sat(model, exhausted) = optimal.clone() else {
        panic!("the problem is satisfiable");
    };
    assert_eq!(optimal, Outcome::Sat(model.clone(), exhausted));
    // One model of `{a;b}` under the default limit: satisfiable, not exhausted.
    let stopped = grounded(&[], FOUR_MODELS).solve(&[]).unwrap();
    assert!(stopped.is_sat() && !stopped.is_exhausted(), "{stopped:?}");
    assert_ne!(optimal, Outcome::Sat(model, stopped));
}

// ---------------------------------------------------------------------------
// Assumptions

#[test]
fn solve_first_with_applies_assumptions() {
    let mut ctl = grounded(&[], FOUR_MODELS);
    let options = SolveOptions::new()
        .assumptions(&[(constant("a"), true).into(), (constant("b"), false).into()]);
    let Outcome::Sat(model, _) = ctl.solve_first_with(options).unwrap() else {
        panic!("{{a;b}} has a model with a and without b");
    };
    assert_eq!(texts(&model), ["a"]);

    // An atom that does not occur is false, so assuming it true is
    // unsatisfiable, as in `Control::solve`.
    let options = SolveOptions::new().assumptions(&[(constant("zzz"), true).into()]);
    assert_eq!(ctl.solve_first_with(options).unwrap(), Outcome::Unsat);

    // The assumptions last for one call.
    assert!(matches!(ctl.solve_first().unwrap(), Outcome::Sat(..)));
}

#[test]
fn solve_optimal_with_applies_assumptions() {
    // With `c` false, the optimum is `{}` with cost 3.
    let mut ctl = grounded(&[], OPTIMISATION);
    let options = SolveOptions::new().assumptions(&[(constant("c"), false).into()]);
    let Outcome::Sat(best, result) = ctl.solve_optimal_with(options).unwrap() else {
        panic!("the problem is satisfiable without c");
    };
    assert_eq!(best.cost(), [3]);
    assert!(best.symbols().is_empty());
    assert!(best.optimality_proven());
    assert!(result.is_exhausted());
}

#[test]
fn solve_all_with_applies_assumptions() {
    let mut ctl = grounded(&[], FOUR_MODELS);
    let options = SolveOptions::new().assumptions(&[(constant("a"), true).into()]);
    let (result, models) = ctl.solve_all_with(options).unwrap();
    assert!(result.is_exhausted());
    let found: Vec<Vec<String>> = models.iter().map(texts).collect();
    assert_eq!(found, [vec!["a"], vec!["a", "b"]]);
}

#[test]
fn the_variants_without_options_are_the_default_options() {
    let mut ctl = grounded(&[], OPTIMISATION);
    assert_eq!(
        ctl.solve_optimal().unwrap(),
        ctl.solve_optimal_with(SolveOptions::new()).unwrap()
    );
    let mut ctl = grounded(&[], FOUR_MODELS);
    assert_eq!(
        ctl.solve_all().unwrap(),
        ctl.solve_all_with(SolveOptions::new()).unwrap()
    );
    assert_eq!(
        ctl.solve_first().unwrap(),
        ctl.solve_first_with(SolveOptions::new()).unwrap()
    );
}

#[test]
fn the_variants_refuse_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    for err in [
        ctl.solve_first_with(SolveOptions::new()).unwrap_err(),
        ctl.solve_optimal_with(SolveOptions::new()).unwrap_err(),
        ctl.solve_all_with(SolveOptions::new()).unwrap_err(),
    ] {
        assert_eq!(err.kind(), ErrorKind::Poisoned);
    }
}

// ---------------------------------------------------------------------------
// Timeouts

#[test]
fn solve_first_with_a_timeout_is_unknown_without_a_model() {
    if !has_threads() {
        return;
    }
    let mut ctl = grounded(&[], HARD_UNSAT);
    let started = Instant::now();
    let options = SolveOptions::new().timeout(Duration::from_millis(300));
    let Outcome::Unknown(result) = ctl.solve_first_with(options).unwrap() else {
        panic!("no model of the pigeonhole problem exists");
    };
    assert!(started.elapsed() < LATE, "{:?}", started.elapsed());
    assert!(result.is_interrupted() && result.is_unknown(), "{result:?}");
    assert!(!is_poisoned(&ctl));
    // The timeout does not reach the next call.
    let mut ctl = grounded(&[], "a.");
    assert!(!ctl.solve(&[]).unwrap().is_interrupted());
}

#[test]
fn solve_optimal_with_a_timeout_returns_the_best_model_so_far() {
    if !has_threads() {
        return;
    }
    let mut ctl = grounded(&["--models=5"], HARD_OPTIMUM);
    let started = Instant::now();
    let options = SolveOptions::new().timeout(Duration::from_millis(300));
    let Outcome::Sat(best, result) = ctl.solve_optimal_with(options).unwrap() else {
        panic!("a first model comes at once");
    };
    assert!(started.elapsed() < LATE, "{:?}", started.elapsed());
    assert!(result.is_sat() && result.is_interrupted(), "{result:?}");
    assert!(!result.is_exhausted());
    assert!(!best.optimality_proven());
    // 13 pigeons in 12 holes collide at least once.
    assert!(best.cost()[0] >= 1, "{:?}", best.cost());
    // The model limit the control was configured with is restored.
    assert_eq!(model_limit(&mut ctl), "5");
    assert!(!is_poisoned(&ctl));
}

#[test]
fn solve_all_with_a_timeout_returns_the_models_found_so_far() {
    if !has_threads() {
        return;
    }
    let mut ctl = grounded(&["--models=3"], HARD_OPTIMUM);
    let started = Instant::now();
    let options = SolveOptions::new().timeout(Duration::from_millis(300));
    let (result, models) = ctl.solve_all_with(options).unwrap();
    assert!(started.elapsed() < LATE, "{:?}", started.elapsed());
    assert!(result.is_sat() && result.is_interrupted(), "{result:?}");
    assert!(!models.is_empty());
    assert!(models.iter().all(|m| !m.optimality_proven()));
    assert_eq!(model_limit(&mut ctl), "3");
}

#[test]
fn a_search_that_ends_within_its_budget_is_not_interrupted() {
    if !has_threads() {
        return;
    }
    let mut ctl = grounded(&[], OPTIMISATION);
    let options = SolveOptions::new().timeout(Duration::from_secs(120));
    let started = Instant::now();
    let Outcome::Sat(best, result) = ctl.solve_optimal_with(options).unwrap() else {
        panic!("the problem is satisfiable");
    };
    assert!(started.elapsed() < LATE, "{:?}", started.elapsed());
    assert!(best.optimality_proven());
    assert!(
        result.is_exhausted() && !result.is_interrupted(),
        "{result:?}"
    );
}

#[test]
fn a_timeout_is_unsupported_without_threads_in_every_variant() {
    if has_threads() {
        return;
    }
    let mut ctl = grounded(&[], "a.");
    let timed = || SolveOptions::new().timeout(Duration::from_secs(1));
    for err in [
        ctl.solve_first_with(timed()).unwrap_err(),
        ctl.solve_optimal_with(timed()).unwrap_err(),
        ctl.solve_all_with(timed()).unwrap_err(),
    ] {
        assert_eq!(err.kind(), ErrorKind::Unsupported);
    }
    assert!(!is_poisoned(&ctl));
    assert!(matches!(
        ctl.solve_first_with(SolveOptions::new()).unwrap(),
        Outcome::Sat(..)
    ));
}

// ---------------------------------------------------------------------------
// A timeout that is ignored fails the test instead of hanging it

/// Runs `solve` on a control of its own on another thread and returns its
/// result, failing if it has not come within `LATE`. Called directly, a
/// variant that ignored its timeout would block the test for hours on these
/// programs instead of failing it.
fn returns_in_time<T: Send + 'static>(
    program: &'static str,
    solve: impl FnOnce(&mut Control) -> T + Send + 'static,
) -> T {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut ctl = grounded(&[], program);
        // The receiver is gone only when the test has already failed.
        let _ = sender.send(solve(&mut ctl));
    });
    receiver
        .recv_timeout(LATE)
        .expect("a call with a 300 ms timeout returns within 30 s")
}

#[test]
fn every_timed_variant_returns_within_its_bound() {
    if !has_threads() {
        return;
    }
    let budget = || SolveOptions::new().timeout(Duration::from_millis(300));

    let first = returns_in_time(HARD_UNSAT, move |c| c.solve_first_with(budget()));
    let Ok(Outcome::Unknown(result)) = first else {
        panic!("no model of HARD_UNSAT comes within the budget: {first:?}");
    };
    assert!(result.is_interrupted(), "{result:?}");

    let optimal = returns_in_time(HARD_OPTIMUM, move |c| c.solve_optimal_with(budget()));
    let Ok(Outcome::Sat(best, result)) = optimal else {
        panic!("a first model of HARD_OPTIMUM comes at once: {optimal:?}");
    };
    assert!(result.is_interrupted(), "{result:?}");
    assert!(!best.optimality_proven());

    let all = returns_in_time(HARD_OPTIMUM, move |c| c.solve_all_with(budget()));
    let (result, models) = all.expect("an interrupted search is not an error");
    assert!(result.is_interrupted(), "{result:?}");
    assert!(!models.is_empty());
}

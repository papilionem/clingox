//! Solving with models: the solve handle and its lending rules, the model loop,
//! and the convenience calls `solve_first`, `solve_optimal` and `solve_all`.
//!
//! Expected values were checked against the Python module `clingo` 5.8.2. Where
//! a test depends on which model clingo finds first, it relies on the
//! deterministic defaults (one solver thread, fixed seed; DESIGN S16) and says
//! so.

#![forbid(unsafe_code)]

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;

// The prelude alone must be enough for model loops, `ControlFlow` included.
use clingox::prelude::*;
use clingox::{Error, ErrorKind};

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn constant(name: &str) -> Symbol {
    Symbol::function(name, &[]).expect("the name has no NUL byte")
}

fn symbols(texts: &[&str]) -> Vec<Symbol> {
    texts
        .iter()
        .map(|t| t.parse().expect("the test term parses"))
        .collect()
}

/// The shown symbols of a model as a sorted list of their texts.
fn shown(model: &Model) -> Vec<String> {
    let mut texts: Vec<String> = model
        .symbols(ShowType::SHOWN)
        .expect("clingo reports the symbols")
        .iter()
        .map(ToString::to_string)
        .collect();
    texts.sort();
    texts
}

fn answer_sets(sets: &[&[&str]]) -> BTreeSet<Vec<String>> {
    sets.iter()
        .map(|set| set.iter().map(|s| (*s).to_owned()).collect())
        .collect()
}

/// `{a;b}.` has four answer sets.
const FOUR_MODELS: &str = "{a;b}.";

/// An optimisation example. Its only optimum is `{c}` with
/// cost 0. Under the default configuration clingo 5.8.2 first finds `{}` with
/// cost 3.
const OPTIMISATION: &str = "{ a; b; c }. :~ a. [1] :~ b. [2] :~ not c. [3]";

/// Nine pigeons, eight holes: unsatisfiable, but not within one conflict, so
/// `--solve-limit=1` leaves the result unknown.
const PIGEONS: &str =
    "p(1..9). h(1..8). 1 { at(P,H) : h(H) } 1 :- p(P). :- at(P,H), at(Q,H), P < Q.";

/// A bound on model loops, so that a handle that never advances fails the test
/// instead of hanging it.
const LOOP_BOUND: usize = 100;

// ---------------------------------------------------------------------------
// SolveHandle

#[test]
fn next_model_lends_every_model_then_none() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut seen = Vec::new();
    while let Some(model) = handle.next_model().unwrap() {
        seen.push(shown(model));
        assert!(seen.len() <= LOOP_BOUND, "the handle does not advance");
    }
    assert!(handle.next_model().unwrap().is_none(), "None stays None");
    assert!(handle.next_model().unwrap().is_none(), "None stays None");
    let result = handle.close().unwrap();

    assert_eq!(seen.len(), 4);
    let seen: BTreeSet<Vec<String>> = seen.into_iter().collect();
    assert_eq!(seen, answer_sets(&[&[], &["a"], &["b"], &["a", "b"]]));
    assert!(result.is_sat());
    assert!(result.is_exhausted());
    assert!(!result.is_interrupted());
}

#[test]
fn the_default_model_limit_is_one() {
    let mut ctl = grounded(&[], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut count = 0;
    while handle.next_model().unwrap().is_some() {
        count += 1;
        assert!(count <= LOOP_BOUND, "the handle does not advance");
    }
    let result = handle.close().unwrap();
    assert_eq!(count, 1);
    assert!(result.is_sat());
    assert!(!result.is_exhausted());
}

#[test]
fn an_unsatisfiable_program_yields_no_model() {
    let mut ctl = grounded(&[], "a :- not a.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_none());
    let result = handle.close().unwrap();
    assert!(result.is_unsat());
    assert!(result.is_exhausted());
}

#[test]
fn get_after_the_last_model_returns_the_final_result_and_can_repeat() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut count = 0;
    while handle.next_model().unwrap().is_some() {
        count += 1;
        assert!(count <= LOOP_BOUND, "the handle does not advance");
    }
    let first = handle.get().unwrap();
    let second = handle.get().unwrap();
    let closed = handle.close().unwrap();
    assert!(first.is_sat() && first.is_exhausted());
    assert_eq!(first, second);
    assert_eq!(first, closed);
}

#[test]
fn get_on_a_model_returns_a_partial_result_and_the_loop_continues() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert_eq!(handle.next_model().unwrap().map(Model::number), Some(1));
    let partial = handle.get().unwrap();
    assert!(partial.is_sat());
    assert!(!partial.is_exhausted());
    assert!(!partial.is_interrupted());
    assert_eq!(handle.next_model().unwrap().map(Model::number), Some(2));
}

#[test]
fn get_before_the_first_model_loses_no_model() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let partial = handle.get().unwrap();
    assert!(partial.is_sat());
    assert!(!partial.is_exhausted());
    // get waited for the first model; next_model must lend that one.
    assert_eq!(handle.next_model().unwrap().map(Model::number), Some(1));
    let mut count = 1;
    while handle.next_model().unwrap().is_some() {
        count += 1;
        assert!(count <= LOOP_BOUND, "the handle does not advance");
    }
    assert_eq!(count, 4);
}

#[test]
fn cancel_stops_the_search_and_reports_an_interruption() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    handle.cancel().unwrap();
    assert!(handle.next_model().unwrap().is_none());
    let result = handle.get().unwrap();
    assert!(result.is_sat(), "a model was found before the cancel");
    assert!(result.is_interrupted());
    assert!(!result.is_exhausted());
    assert_eq!(handle.close().unwrap(), result);
}

#[test]
fn cancel_before_the_first_model_leaves_the_result_unknown() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    handle.cancel().unwrap();
    assert!(handle.next_model().unwrap().is_none());
    let result = handle.close().unwrap();
    assert!(result.is_unknown());
    assert!(result.is_interrupted());
}

#[test]
fn close_in_the_middle_returns_the_partial_result() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    let result = handle.close().unwrap();
    assert!(result.is_sat());
    assert!(!result.is_exhausted());
    assert!(!result.is_interrupted());
}

#[test]
fn dropping_a_handle_early_leaves_the_control_usable() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    drop(handle);

    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_exhausted());
    assert_eq!(models.len(), 4);
    ctl.add("more", &[], "c.").unwrap();
    ctl.ground(&[Part::new("more", &[]).unwrap()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_forgotten_handle_is_finished_by_the_next_call() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    // Leaking is safe Rust, so the control cannot rely on the handle's Drop
    // (S4).
    std::mem::forget(handle);

    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());
    assert!(result.is_exhausted());
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 4);
}

#[test]
fn a_forgotten_handle_is_finished_before_grounding() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);

    ctl.add("more", &[], "c.").unwrap();
    ctl.ground(&[Part::new("more", &[]).unwrap()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 4);
    assert!(models.iter().all(|m| m.contains(constant("c"))));
}

#[test]
fn a_control_with_a_forgotten_handle_drops_cleanly() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);
    drop(ctl);
}

#[test]
fn solve_yield_respects_assumptions() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[(constant("a"), true).into()]).unwrap();
    let mut seen = BTreeSet::new();
    while let Some(model) = handle.next_model().unwrap() {
        seen.insert(shown(model));
        assert!(seen.len() <= LOOP_BOUND, "the handle does not advance");
    }
    assert_eq!(seen, answer_sets(&[&["a"], &["a", "b"]]));
}

#[test]
fn a_solve_handle_describes_itself() {
    let mut ctl = grounded(&[], "a.");
    let handle = ctl.solve_yield(&[]).unwrap();
    let text = format!("{handle:?}");
    assert!(text.contains("SolveHandle"), "{text}");
}

// ---------------------------------------------------------------------------
// for_each_model

#[test]
fn for_each_model_visits_every_model() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut numbers = Vec::new();
    let mut seen = BTreeSet::new();
    let result = ctl
        .for_each_model(&[], |model| {
            numbers.push(model.number());
            seen.insert(shown(model));
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(numbers, [1, 2, 3, 4]);
    assert_eq!(seen, answer_sets(&[&[], &["a"], &["b"], &["a", "b"]]));
    assert!(result.is_sat());
    assert!(result.is_exhausted());
    assert!(!result.is_interrupted());
}

#[test]
fn for_each_model_respects_the_model_limit() {
    let mut ctl = grounded(&[], FOUR_MODELS);
    let mut count = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            count += 1;
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(result.is_sat());
    assert!(!result.is_exhausted());
}

#[test]
fn for_each_model_respects_assumptions() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut seen = BTreeSet::new();
    let result = ctl
        .for_each_model(&[(constant("b"), false).into()], |model| {
            seen.insert(shown(model));
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(seen, answer_sets(&[&[], &["a"]]));
    assert!(result.is_exhausted());
}

#[test]
fn breaking_out_of_for_each_model_interrupts_the_search() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut count = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            count += 1;
            Ok(ControlFlow::Break(()))
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(result.is_sat());
    assert!(result.is_interrupted());
    assert!(!result.is_exhausted());
    // The search is closed: the control solves again from scratch.
    assert!(ctl.solve(&[]).unwrap().is_exhausted());
}

#[derive(Debug)]
struct Rejected(u64);

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "model {} rejected", self.0)
    }
}

impl std::error::Error for Rejected {}

#[test]
fn an_error_from_the_model_closure_is_returned_and_does_not_poison() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut count = 0;
    let err = ctl
        .for_each_model(&[], |model| {
            count += 1;
            Err(Error::callback(Rejected(model.number())))
        })
        .unwrap_err();
    assert_eq!(count, 1, "the loop stops at the first error");
    assert_eq!(err.kind(), ErrorKind::Callback);
    let source = std::error::Error::source(&err).expect("the user error is the source");
    let rejected = source
        .downcast_ref::<Rejected>()
        .expect("the source keeps its type");
    assert_eq!(rejected.0, 1);
    // Display carries the context, and the user's own
    // text is reached through source(), so reporters do not print it twice.
    assert!(!err.to_string().contains("model 1 rejected"), "{err}");
    assert!(source.to_string().contains("model 1 rejected"), "{source}");

    assert!(!format!("{ctl:?}").contains("poisoned"));
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_exhausted());
    assert_eq!(models.len(), 4);
}

#[test]
fn a_clingox_error_from_the_model_closure_is_returned_unchanged() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let err = ctl
        .for_each_model(&[], |_| {
            // A clingox error raised inside the closure, here a NUL byte.
            let _symbol = Symbol::function("bad\0name", &[])?;
            Ok(ControlFlow::Continue(()))
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_panic_in_the_model_closure_reaches_the_caller_and_the_control_survives() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctl.for_each_model(&[], |_| panic!("stop here"))
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"stop here"));

    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_exhausted());
    assert_eq!(models.len(), 4);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn for_each_model_runs_the_closure_on_the_calling_thread() {
    // With four solver threads clingo's own model event would run on whichever
    // thread found the model (DESIGN S10). The closure captures an `Rc`, which
    // is not `Send`, and checks the thread of every call.
    let mut ctl = grounded(&["--parallel-mode=4", "--models=0"], "{a;b;c;d;e;f}.");
    let caller = std::thread::current().id();
    let count = Rc::new(Cell::new(0_u32));
    let counter = Rc::clone(&count);
    let result = ctl
        .for_each_model(&[], move |_| {
            assert_eq!(std::thread::current().id(), caller);
            counter.set(counter.get() + 1);
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(count.get(), 64);
    assert!(result.is_exhausted());
}

// ---------------------------------------------------------------------------
// solve_first

#[test]
fn solve_first_returns_one_model_and_stops() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let Outcome::Sat(model, _) = ctl.solve_first().unwrap() else {
        panic!("{{a;b}} is satisfiable");
    };
    assert_eq!(model.number(), 1);
    assert!(
        answer_sets(&[&[], &["a"], &["b"], &["a", "b"]]).contains(
            &model
                .symbols()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        )
    );
    // The control's own model limit is untouched.
    let mut count = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            count += 1;
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(count, 4);
    assert!(result.is_exhausted());
}

#[test]
fn solve_first_reports_an_unsatisfiable_program() {
    let mut ctl = grounded(&[], "a :- not a.");
    assert_eq!(ctl.solve_first().unwrap(), Outcome::Unsat);
}

#[test]
fn solve_first_reports_an_undecided_search_with_its_result() {
    let mut ctl = grounded(&["--solve-limit=1"], PIGEONS);
    let Outcome::Unknown(result) = ctl.solve_first().unwrap() else {
        panic!("one conflict does not decide the pigeonhole problem");
    };
    assert!(result.is_unknown());
    assert!(!result.is_interrupted());
    assert!(!result.is_exhausted());
}

#[test]
fn solve_first_on_an_optimisation_problem_returns_the_first_model_not_the_best() {
    // Depends on the search order: under the default configuration clingo 5.8.2
    // finds `{}` with cost 3 before the optimum `{c}` with cost 0.
    let mut ctl = grounded(&[], OPTIMISATION);
    let Outcome::Sat(model, _) = ctl.solve_first().unwrap() else {
        panic!("the problem is satisfiable");
    };
    assert_eq!(model.number(), 1);
    assert_eq!(model.cost(), [3]);
    assert!(!model.optimality_proven());
}

#[test]
fn outcomes_can_be_matched_without_a_wildcard() {
    fn describe(outcome: &Outcome<OwnedModel>) -> &'static str {
        // No `_` arm: `Outcome` is exhaustive on purpose.
        match outcome {
            Outcome::Sat(..) => "sat",
            Outcome::Unsat => "unsat",
            Outcome::Unknown(_) => "unknown",
        }
    }
    fn outcome_traits<T: Clone + PartialEq + std::fmt::Debug>() {}
    outcome_traits::<Outcome<OwnedModel>>();

    let mut ctl = grounded(&[], "a.");
    assert_eq!(describe(&ctl.solve_first().unwrap()), "sat");
}

// ---------------------------------------------------------------------------
// solve_optimal

#[test]
fn solve_optimal_returns_a_proven_optimum() {
    let mut ctl = grounded(&[], OPTIMISATION);
    let Outcome::Sat(best, _) = ctl.solve_optimal().unwrap() else {
        panic!("the problem is satisfiable");
    };
    assert_eq!(best.cost(), [0]);
    assert!(best.optimality_proven());
    assert_eq!(best.symbols(), symbols(&["c"]));
}

#[test]
fn solve_optimal_lifts_a_model_limit_of_one() {
    // With `--models=1` alone, clingo stops at the first model, with cost 3.
    let mut ctl = grounded(&["--models=1"], OPTIMISATION);
    let Outcome::Sat(best, _) = ctl.solve_optimal().unwrap() else {
        panic!("the problem is satisfiable");
    };
    assert_eq!(best.cost(), [0]);
    assert!(best.optimality_proven());
}

#[test]
fn solve_optimal_orders_costs_by_priority() {
    let mut ctl = grounded(
        &[],
        "{a;b;c}. :- not a, not b. :~ a. [1@2] :~ b. [2@1] :~ c. [1@1]",
    );
    let Outcome::Sat(best, _) = ctl.solve_optimal().unwrap() else {
        panic!("the problem is satisfiable");
    };
    assert_eq!(best.cost(), [0, 2]);
    assert_eq!(best.symbols(), symbols(&["b"]));
    assert!(best.optimality_proven());
}

#[test]
fn solve_optimal_without_optimisation_returns_one_model() {
    // clingo's default limit computes one model when nothing is optimised; with
    // the limit lifted it would enumerate all four and return the fourth.
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let Outcome::Sat(model, _) = ctl.solve_optimal().unwrap() else {
        panic!("{{a;b}} is satisfiable");
    };
    assert_eq!(model.number(), 1);
    assert_eq!(model.cost(), []);
    assert!(!model.optimality_proven());
}

#[test]
fn solve_optimal_restores_the_model_limit() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let _ = ctl.solve_optimal().unwrap();
    let mut count = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            count += 1;
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(count, 4);
    assert!(result.is_exhausted());
}

#[test]
fn solve_optimal_reports_an_unsatisfiable_program() {
    let mut ctl = grounded(&[], "{a}. :~ a. [1] b :- not b.");
    assert_eq!(ctl.solve_optimal().unwrap(), Outcome::Unsat);
}

#[test]
fn solve_optimal_reports_an_undecided_search() {
    let mut ctl = grounded(&["--solve-limit=1"], PIGEONS);
    assert!(matches!(
        ctl.solve_optimal().unwrap(),
        Outcome::Unknown(result) if result.is_unknown() && !result.is_interrupted()
    ));
}

// ---------------------------------------------------------------------------
// solve_all

#[test]
fn solve_all_returns_every_model_sorted() {
    // clingo 5.8.2 finds these in the order {}, {b}, {a}, {a,b}.
    let mut ctl = grounded(&[], FOUR_MODELS);
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert!(result.is_exhausted());
    let found: Vec<&[Symbol]> = models.iter().map(OwnedModel::symbols).collect();
    let expected = [
        symbols(&[]),
        symbols(&["a"]),
        symbols(&["a", "b"]),
        symbols(&["b"]),
    ];
    let expected: Vec<&[Symbol]> = expected.iter().map(Vec::as_slice).collect();
    assert_eq!(found, expected);
}

#[test]
fn solve_all_sorts_the_symbols_of_each_model() {
    // clingo reports this model's shown symbols as `p(2) p(1) t(2) t(1) 42`.
    let mut ctl = grounded(
        &[],
        "p(2). p(1). hidden. #show p/1. #show 42. #show t(X) : p(X).",
    );
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(
        models[0].symbols(),
        symbols(&["42", "p(1)", "p(2)", "t(1)", "t(2)"])
    );
    assert_eq!(models[0].all_atoms(), symbols(&["hidden", "p(1)", "p(2)"]));
}

#[test]
fn solve_all_keeps_the_model_numbers_clingo_gave() {
    let mut ctl = grounded(&[], FOUR_MODELS);
    let (_, models) = ctl.solve_all().unwrap();
    let numbers: BTreeSet<u64> = models.iter().map(OwnedModel::number).collect();
    assert_eq!(numbers, BTreeSet::from([1, 2, 3, 4]));
}

#[test]
fn solve_all_restores_the_model_limit() {
    let mut ctl = grounded(&[], FOUR_MODELS);
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 4);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut count = 0;
    while handle.next_model().unwrap().is_some() {
        count += 1;
        assert!(count <= LOOP_BOUND, "the handle does not advance");
    }
    assert_eq!(count, 1, "the default limit of one model is back");
}

#[test]
fn solve_all_of_an_unsatisfiable_program_is_empty() {
    let mut ctl = grounded(&[], "a :- not a.");
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_unsat());
    assert_eq!(models, []);
}

#[test]
fn solve_all_of_an_undecided_search_reports_unknown() {
    let mut ctl = grounded(&["--solve-limit=1"], PIGEONS);
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_unknown());
    assert_eq!(models, []);
}

// ---------------------------------------------------------------------------
// Poisoning

#[test]
fn every_solving_entry_point_refuses_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let poisoned = |kind: ErrorKind| assert_eq!(kind, ErrorKind::Poisoned);

    poisoned(ctl.solve_yield(&[]).unwrap_err().kind());
    poisoned(
        ctl.for_each_model(&[], |_| Ok(ControlFlow::Continue(())))
            .unwrap_err()
            .kind(),
    );
    poisoned(ctl.solve_first().unwrap_err().kind());
    poisoned(ctl.solve_optimal().unwrap_err().kind());
    poisoned(ctl.solve_all().unwrap_err().kind());
}

#[test]
fn callback_errors_have_the_expected_traits() {
    fn error_traits<T: std::error::Error + Send + Sync + 'static>() {}
    error_traits::<Error>();
    let err = Error::callback(Rejected(7));
    assert_eq!(err.kind(), ErrorKind::Callback);
    // The user's text is reached through `source()`, and Display does not
    // repeat it.
    assert!(!err.to_string().contains("model 7 rejected"), "{err}");
    let source = std::error::Error::source(&err).expect("the user error is the source");
    assert!(source.to_string().contains("model 7 rejected"), "{source}");
}

//! Solve handle lifecycle: leaked and dropped handles, panics and errors in
//! the model closure, and semantics checked against the Python module
//! `clingo` 5.8.2.

#![forbid(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use clingox::prelude::*;

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn constant(name: &str) -> Symbol {
    Symbol::function(name, &[]).expect("the name has no NUL byte")
}

const FOUR_MODELS: &str = "{a;b}. #external e.";

/// A control with a search left open by a forgotten handle, a model current.
fn leaked() -> Control {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).expect("the search starts");
    assert!(handle.next_model().expect("model 1").is_some());
    assert!(handle.next_model().expect("model 2").is_some());
    std::mem::forget(handle);
    ctl
}

fn count(ctl: &mut Control) -> usize {
    let mut n = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            n += 1;
            assert!(n <= 100, "the loop does not advance");
            Ok(ControlFlow::Continue(()))
        })
        .expect("the search succeeds");
    assert!(result.is_exhausted());
    n
}

/// One entry point of `Control`, named for the failure message.
type Entry<'a> = (&'a str, Box<dyn Fn(&mut Control)>);

#[test]
fn dropping_a_handle_closes_the_search_at_once() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    drop(handle);
    // Drop closes the search itself; the control is idle, not waiting for its
    // next call to close a leftover (Debug shows `idle` or `poisoned`).
    let text = format!("{ctl:?}");
    assert!(text.contains("idle"), "{text}");
    assert_eq!(count(&mut ctl), 4);
}

#[test]
fn every_entry_point_finishes_a_forgotten_handle() {
    let e = constant("e");
    let a = constant("a");
    let entries: Vec<Entry<'_>> = vec![
        ("add", Box::new(|c| c.add("p", &[], "x.").unwrap())),
        ("add_base", Box::new(|c| c.add_base("y.").unwrap())),
        (
            "ground",
            Box::new(|c| c.ground(&[Part::new("nothing", &[]).unwrap()]).unwrap()),
        ),
        (
            "solve",
            Box::new(|c| assert!(c.solve(&[]).unwrap().is_exhausted())),
        ),
        (
            "solve_yield",
            Box::new(|c| {
                let mut h = c.solve_yield(&[]).unwrap();
                assert_eq!(h.next_model().unwrap().map(Model::number), Some(1));
            }),
        ),
        ("for_each_model", Box::new(|c| assert_eq!(count(c), 4))),
        (
            "solve_first",
            Box::new(|c| assert!(matches!(c.solve_first().unwrap(), Outcome::Sat(..)))),
        ),
        (
            "solve_optimal",
            Box::new(|c| assert!(matches!(c.solve_optimal().unwrap(), Outcome::Sat(..)))),
        ),
        (
            "solve_all",
            Box::new(|c| assert_eq!(c.solve_all().unwrap().1.len(), 4)),
        ),
        (
            "assign_external",
            Box::new(move |c| c.assign_external(e, TruthValue::True).unwrap()),
        ),
        (
            "try_assign_external",
            Box::new(move |c| c.try_assign_external(e, TruthValue::Free).unwrap()),
        ),
        (
            "release_external",
            Box::new(move |c| c.release_external(e).unwrap()),
        ),
        (
            "solve with assumptions",
            Box::new(move |c| assert!(c.solve(&[(a, true).into()]).unwrap().is_sat())),
        ),
    ];
    for (name, entry) in &entries {
        let mut ctl = leaked();
        entry(&mut ctl);
        let text = format!("{ctl:?}");
        assert!(!text.contains("poisoned"), "{name}: {text}");
        // The control still enumerates from scratch afterwards.
        assert!(ctl.solve(&[]).unwrap().is_sat(), "{name}");
    }
}

#[test]
fn dropping_a_control_with_a_forgotten_handle_is_clean() {
    drop(leaked());
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let handle = ctl.solve_yield(&[]).unwrap();
    std::mem::forget(handle);
    drop(ctl);
}

#[test]
fn a_panic_in_the_model_closure_leaves_the_control_usable() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.for_each_model(&[], |model| {
            assert!(model.number() != 2, "boom");
            Ok(ControlFlow::Continue(()))
        })
    }));
    assert!(caught.is_err());
    let text = format!("{ctl:?}");
    assert!(text.contains("idle"), "{text}");
    assert_eq!(count(&mut ctl), 4);
}

#[test]
fn an_error_of_a_poisoning_kind_from_the_closure_does_not_poison() {
    let mut ctl = grounded(&["--models=0"], FOUR_MODELS);
    let err = ctl
        // An error of kind Logic that the closure got from clingo itself.
        .for_each_model(&[], |_| {
            Err(Control::with_args(["--no-such-option"]).unwrap_err())
        })
        .unwrap_err();
    assert_eq!(err.kind(), clingox::ErrorKind::Logic);
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert_eq!(count(&mut ctl), 4);
}

#[test]
fn get_on_a_lent_model_then_next_model_continues_without_loss() {
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut numbers = Vec::new();
    while let Some(model) = handle.next_model().unwrap() {
        numbers.push(model.number());
        let partial = handle.get().unwrap();
        assert!(partial.is_sat() && !partial.is_exhausted());
        assert!(numbers.len() <= 10);
    }
    assert_eq!(numbers, [1, 2, 3, 4]);
    let result = handle.get().unwrap();
    assert!(result.is_exhausted());
    assert_eq!(handle.get().unwrap(), result);
}

#[test]
fn cancel_before_the_first_model_is_unknown_and_interrupted() {
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    handle.cancel().unwrap();
    assert!(handle.next_model().unwrap().is_none());
    let result = handle.close().unwrap();
    assert!(result.is_interrupted());
    assert!(!result.is_sat());
    assert!(!result.is_unsat());
}

// Checked against Python clingo 5.8.2: with `--models=2` configured,
// solve_optimal must leave `2` in place.
#[test]
fn solve_optimal_and_solve_all_restore_a_custom_limit() {
    let mut ctl = grounded(&["--models=2"], "{a;b}.");
    let _ = ctl.solve_optimal().unwrap();
    let _ = ctl.solve_all().unwrap();
    let mut n = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            n += 1;
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(n, 2);
    assert!(!result.is_exhausted());
}

// Python clingo 5.8.2, `--opt-mode=optN`, solve.models=-1: the last model is
// `{c}`, cost [0], optimality proven.
#[test]
fn solve_optimal_under_opt_n() {
    let mut ctl = grounded(
        &["--opt-mode=optN"],
        "{ a; b; c }. :~ a. [1] :~ b. [2] :~ not c. [3]",
    );
    let Outcome::Sat(best, _) = ctl.solve_optimal().unwrap() else {
        panic!("satisfiable");
    };
    assert_eq!(best.cost(), [0]);
    assert!(best.optimality_proven());
    assert_eq!(best.symbols(), [constant("c")]);
}

// Python clingo 5.8.2: `--solve-limit=1` on a hard unsat instance gives
// UNKNOWN, not interrupted; solve_first reports Unknown.
#[test]
fn solve_first_reports_an_undecided_search() {
    let mut ctl = grounded(
        &["--solve-limit=1"],
        "p(1..6). h(1..5). 1 { in(P,H) : h(H) } 1 :- p(P). :- in(P,H), in(Q,H), P < Q.",
    );
    assert!(matches!(
        ctl.solve_first().unwrap(),
        Outcome::Unknown(r) if r.is_unknown() && !r.is_interrupted()
    ));
}

// Python clingo 5.8.2: with `--solve-limit=20` this optimisation stops after
// three models; the result is SAT, not exhausted, and `last()` is the cost-31
// model with optimality not proven.
#[test]
fn solve_optimal_at_a_solve_limit_returns_the_best_model_so_far() {
    let mut ctl = grounded(
        &["--solve-limit=20"],
        "p(1..8). h(1..8). 1 { in(P,H) : h(H) } 1 :- p(P). \
         :- in(P,H), in(Q,H), P < Q. \
         #minimize { W,P,H : in(P,H), W=(P*H*7) \\ 13 }.",
    );
    let Outcome::Sat(best, _) = ctl.solve_optimal().unwrap() else {
        panic!("a model was found before the limit");
    };
    assert_eq!(best.cost(), [31]);
    assert!(!best.optimality_proven());
    assert!(!format!("{ctl:?}").contains("poisoned"));
}

#[test]
fn every_show_flag_together_selects_the_complement_of_all() {
    // `ShowType` is clingox's own type and holds only clingo's flags, so
    // the unknown bits this test used to pass cannot be built any more. Python
    // clingo 5.8.2, for the two models of this program: with every flag the
    // model without `q` gives `[p(1), 42]`, and with COMPLEMENT added `[q]`;
    // the model with `q` gives `[q, p(1), 42]`, and with COMPLEMENT `[]`.
    let mut ctl = grounded(&["--models=0"], "p(1). {q}. #show p/1. #show 42.");
    let mut seen = Vec::new();
    let result = ctl
        .for_each_model(&[], |model| {
            let texts = |show| -> clingox::Result<Vec<String>> {
                let mut texts: Vec<String> = model
                    .symbols(show)?
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                texts.sort();
                Ok(texts)
            };
            seen.push((
                texts(ShowType::ALL)?,
                texts(ShowType::ALL | ShowType::COMPLEMENT)?,
            ));
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(result.is_exhausted());
    seen.sort();
    let owned = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        seen,
        [
            (owned(&["42", "p(1)"]), owned(&["q"])),
            (owned(&["42", "p(1)", "q"]), owned(&[])),
        ]
    );
}

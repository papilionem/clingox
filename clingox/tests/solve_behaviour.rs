//! Behaviour of solving beyond `api_solve.rs` and `api_model.rs`: leftover
//! searches, the state the control reports, and the text forms of models and
//! callback errors.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

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

#[test]
fn a_forgotten_handle_shows_in_debug_until_the_next_call() {
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);
    assert!(format!("{ctl:?}").contains("solving"), "{ctl:?}");

    assert!(ctl.solve(&[]).unwrap().is_exhausted());
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
}

#[test]
fn a_forgotten_handle_is_finished_by_the_next_solve_yield() {
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);

    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut numbers = Vec::new();
    while let Some(model) = handle.next_model().unwrap() {
        numbers.push(model.number());
        assert!(numbers.len() <= 4, "{{a;b}} has four models");
    }
    assert_eq!(numbers, [1, 2, 3, 4]);
}

#[test]
fn a_forgotten_handle_is_finished_before_assigning_an_external() {
    let mut ctl = grounded(&["--models=0"], "#external e. {a}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);

    ctl.assign_external(constant("e"), TruthValue::True)
        .unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 2);
    assert!(models.iter().all(|m| m.contains(constant("e"))));
}

#[test]
fn the_convenience_calls_leave_no_search_open() {
    let mut ctl = grounded(&["--models=0"], "{a;b}. :~ a. [1]");
    let _ = ctl.solve_first().unwrap();
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
    let _ = ctl.solve_optimal().unwrap();
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
    let _ = ctl.solve_all().unwrap();
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
    let _ = ctl
        .for_each_model(&[], |_| Ok(ControlFlow::Break(())))
        .unwrap();
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
}

#[test]
fn solve_optimal_restores_a_configured_limit_of_two() {
    let mut ctl = grounded(&["--models=2"], "{a;b;c}.");
    let _ = ctl.solve_optimal().unwrap();
    let _ = ctl.solve_all().unwrap();
    let mut count = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            count += 1;
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(count, 2);
    assert!(!result.is_exhausted());
}

#[test]
fn get_after_cancel_is_interrupted_every_time() {
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    handle.cancel().unwrap();
    let first = handle.get().unwrap();
    assert!(first.is_interrupted());
    assert_eq!(handle.get().unwrap(), first);
    handle.cancel().unwrap();
    assert!(handle.next_model().unwrap().is_none());
}

#[test]
fn every_model_of_a_yield_loop_is_distinct() {
    let mut ctl = grounded(&["--models=0"], "{a;b;c}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut seen = BTreeSet::new();
    while let Some(model) = handle.next_model().unwrap() {
        assert!(seen.insert(model.snapshot().unwrap().symbols().to_vec()));
        assert!(seen.len() <= 8, "{{a;b;c}} has eight models");
    }
    assert_eq!(seen.len(), 8);
}

#[test]
fn model_debug_shows_the_number_and_cost() {
    let mut ctl = grounded(&[], "a. :~ a. [4]");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle.next_model().unwrap().expect("`a.` has a model");
    let text = format!("{model:?}");
    assert!(text.contains("number: 1"), "{text}");
    assert!(text.contains("cost: [4]"), "{text}");
}

#[test]
fn an_owned_model_does_not_contain_a_shown_term() {
    // Checked with the Python module clingo 5.8.2: shown `[b]`, atoms `[a]`.
    let mut ctl = grounded(&[], "a. #show. #show b : a.");
    let Outcome::Sat(model, _) = ctl.solve_first().unwrap() else {
        panic!("the program has a model");
    };
    assert_eq!(model.symbols(), [constant("b")]);
    assert!(!model.contains(constant("b")));
    assert!(model.contains(constant("a")));
}

#[test]
fn show_type_debug_names_the_flags() {
    let text = format!("{:?}", ShowType::ATOMS | ShowType::TERMS);
    assert!(text.contains("ATOMS") && text.contains("TERMS"), "{text}");
}

#[derive(Debug)]
struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("refused")
    }
}

impl std::error::Error for Refused {}

#[test]
fn callback_errors_show_their_source_in_debug() {
    let err = Error::callback(Refused);
    // the user text is the source's, not repeated in Display.
    assert_eq!(err.to_string(), "the callback failed");
    let text = format!("{err:?}");
    assert!(
        text.contains("Callback") && text.contains("Refused"),
        "{text}"
    );
}

#[test]
fn a_callback_error_in_a_model_loop_keeps_its_kind() {
    let mut ctl = grounded(&[], "a.");
    let err = ctl
        .for_each_model(&[], |_| Err(Error::callback(Refused)))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

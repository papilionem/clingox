//! Creating a control, adding, grounding and solving programs, and errors.

#![forbid(unsafe_code)]

use clingox::{Assumption, Control, ErrorKind, Part, SolveResult, Symbol};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("creating a control");
    ctl.add_base(program).expect("adding the program");
    ctl.ground(&[Part::base()])
        .expect("grounding the base part");
    ctl
}

#[test]
fn solves_a_program_with_an_answer_set() {
    let mut ctl = grounded("a. b :- a.");
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());
    assert!(!result.is_unsat());
    assert!(!result.is_unknown());
    assert!(!result.is_interrupted());
}

#[test]
fn reports_an_unsatisfiable_program() {
    let mut ctl = grounded("a :- not a.");
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_unsat());
    assert!(result.is_exhausted());
    assert!(!result.is_sat());
}

#[test]
fn solve_results_print_like_clingo() {
    let sat = grounded("a.").solve(&[]).unwrap();
    let unsat = grounded("a :- not a.").solve(&[]).unwrap();
    assert_eq!(sat.to_string(), "SATISFIABLE");
    assert_eq!(unsat.to_string(), "UNSATISFIABLE");
}

#[test]
fn grounds_a_part_with_parameters() {
    let mut ctl = Control::new().unwrap();
    ctl.add("step", &["t"], "p(t).").unwrap();
    ctl.ground(&[Part::new("step", &[Symbol::number(3)]).unwrap()])
        .unwrap();

    let p3 = Symbol::function("p", &[Symbol::number(3)]).unwrap();
    // p(3) is a fact, so assuming it true is satisfiable and assuming it false is not.
    assert!(ctl.solve(&[Assumption::from((p3, true))]).unwrap().is_sat());
    assert!(
        ctl.solve(&[Assumption::from((p3, false))])
            .unwrap()
            .is_unsat()
    );
}

#[test]
fn assumptions_select_between_answer_sets() {
    let mut ctl = grounded("a :- not b. b :- not a.");
    let a = Symbol::function("a", &[]).unwrap();
    let b = Symbol::function("b", &[]).unwrap();
    assert!(ctl.solve(&[(a, true).into()]).unwrap().is_sat());
    assert!(
        ctl.solve(&[(a, true).into(), (b, true).into()])
            .unwrap()
            .is_unsat()
    );
}

#[test]
fn accepts_arguments_in_any_string_form() {
    Control::with_args(["--models=0"]).unwrap();
    Control::with_args(vec![String::from("--models=0")]).unwrap();
    Control::with_args(Vec::<String>::new()).unwrap();
}

#[test]
fn an_unknown_option_is_an_error() {
    let err = Control::with_args(["--no-such-option"]).unwrap_err();
    assert!(
        err.to_string().contains("no-such-option"),
        "the message should name the option: {err}"
    );
}

#[test]
fn a_parse_error_carries_clingo_messages_with_locations() {
    let mut ctl = Control::new().unwrap();
    let err = ctl.add_base("a :- b c.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);

    let message = err
        .messages()
        .first()
        .expect("clingo reports the syntax error");
    assert!(
        message.text().contains("syntax error"),
        "{}",
        message.text()
    );
    let location = message.location().expect("the message has a position");
    assert_eq!(location.file(), "<block>");
    assert_eq!(location.line(), 1);
    assert_eq!(location.column(), 8);
}

#[test]
fn error_messages_name_the_operation_and_read_as_one_line() {
    let mut ctl = Control::new().unwrap();
    let err = ctl.add_base("a :- b c.").unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("base"),
        "the message should name the part: {text}"
    );
    assert!(!text.contains('\n'), "Display is one line: {text:?}");
    assert!(!text.ends_with('.'), "no trailing period: {text:?}");
}

#[test]
fn control_is_poisoned_after_a_parse_error() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
    let err = ctl.add_base("a.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
    assert!(format!("{ctl:?}").contains("poisoned"));
}

#[test]
fn a_new_control_works_after_another_was_poisoned() {
    let mut bad = Control::new().unwrap();
    bad.add_base("a :- b c.").unwrap_err();
    drop(bad);
    assert!(grounded("a.").solve(&[]).unwrap().is_sat());
}

#[test]
fn a_part_name_with_nul_is_rejected() {
    let err = Part::new("st\0ep", &[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    let mut ctl = Control::new().unwrap();
    let err = ctl.add_base("a.\0").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
}

#[test]
fn a_nul_in_the_input_does_not_poison_the_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.\0").unwrap_err();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn debug_shows_the_state_of_a_control() {
    let ctl = Control::new().unwrap();
    let text = format!("{ctl:?}");
    assert!(text.contains("Control"), "{text}");
    assert!(text.contains("idle"), "{text}");
}

#[test]
fn many_controls_can_be_created_and_dropped() {
    for i in 0..50 {
        let mut ctl = grounded(&format!("p({i})."));
        assert!(ctl.solve(&[]).unwrap().is_sat());
    }
}

#[test]
fn errors_and_results_have_the_expected_traits() {
    fn error_traits<T: std::error::Error + Send + Sync + 'static>() {}
    fn result_traits<T: Copy + Eq + std::fmt::Debug + std::fmt::Display>() {}
    error_traits::<clingox::Error>();
    result_traits::<SolveResult>();
}

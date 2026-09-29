//! Behaviour of `Control` beyond `api_control.rs`: assumptions on atoms
//! that do not exist, grounding errors, messages and results.

#![forbid(unsafe_code)]

use clingox::{Control, ErrorKind, MessageCode, Part, Symbol};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a control can be created");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn constant(name: &str) -> Symbol {
    Symbol::function(name, &[]).expect("the name has no NUL byte")
}

#[test]
fn assuming_an_unknown_atom_true_is_unsatisfiable() {
    let mut ctl = grounded("a.");
    let result = ctl.solve(&[(constant("unknown"), true).into()]).unwrap();
    assert!(result.is_unsat());
}

#[test]
fn assuming_an_unknown_atom_false_changes_nothing() {
    let mut ctl = grounded("a.");
    assert!(
        ctl.solve(&[(constant("unknown"), false).into()])
            .unwrap()
            .is_sat()
    );
    let mut ctl = grounded("a :- not a.");
    assert!(
        ctl.solve(&[(constant("unknown"), false).into()])
            .unwrap()
            .is_unsat()
    );
}

#[test]
fn assumptions_on_unknown_atoms_work_before_anything_is_grounded() {
    let mut ctl = Control::new().unwrap();
    assert!(
        ctl.solve(&[(constant("u"), true).into()])
            .unwrap()
            .is_unsat()
    );
    assert!(
        ctl.solve(&[(constant("u"), false).into()])
            .unwrap()
            .is_sat()
    );
}

#[test]
fn assumptions_apply_to_one_solve_call_only() {
    let mut ctl = grounded("a :- not b. b :- not a.");
    let a = constant("a");
    let b = constant("b");
    assert!(
        ctl.solve(&[(a, true).into(), (b, true).into()])
            .unwrap()
            .is_unsat()
    );
    assert!(ctl.solve(&[]).unwrap().is_sat());
    assert!(
        ctl.solve(&[(a, false).into(), (b, false).into()])
            .unwrap()
            .is_unsat()
    );
}

#[test]
fn an_unsafe_variable_fails_grounding_with_a_located_message() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(X).").unwrap();
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
    let message = &err.messages()[0];
    assert_eq!(message.code(), MessageCode::RuntimeError);
    assert!(message.text().contains("unsafe"), "{}", message.text());
    assert_eq!(
        message.location().map(|l| (l.line(), l.column())),
        Some((1, 1))
    );
    assert!(err.to_string().starts_with("grounding `base`: "), "{err}");
    assert!(!format!("{ctl:?}").contains("poisoned"));
}

#[test]
fn the_poisoned_error_names_its_cause() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
    assert!(err.to_string().contains("syntax error"), "{err}");
    assert!(format!("{ctl:?}").contains("syntax error"));
}

#[test]
fn a_nul_in_a_parameter_or_part_name_is_rejected_without_poisoning() {
    let mut ctl = Control::new().unwrap();
    assert_eq!(
        ctl.add("st\0ep", &[], "a.").unwrap_err().kind(),
        ErrorKind::Nul
    );
    assert_eq!(
        ctl.add("step", &["t\0"], "a.").unwrap_err().kind(),
        ErrorKind::Nul
    );
    assert_eq!(
        Control::with_args(["--models=\0"]).unwrap_err().kind(),
        ErrorKind::Nul
    );
    ctl.add("step", &["t"], "p(t).").unwrap();
    ctl.ground(&[Part::new("step", &[Symbol::number(1)]).unwrap()])
        .unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn an_unknown_option_is_reported_as_clingo_does() {
    // clingo 5.8.2 reports a bad option as a logic error, although clingo.h
    // documents a runtime error for failed argument parsing.
    let err = Control::with_args(["--no-such-option"]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Logic);
    assert!(err.to_string().starts_with("creating a control: "), "{err}");
}

#[test]
fn parts_print_with_their_parameters() {
    assert_eq!(Part::base().to_string(), "base");
    let part = Part::new("step", &[Symbol::number(1), constant("c")]).unwrap();
    assert_eq!(part.to_string(), "step(1,c)");
}

#[test]
fn grounding_a_part_that_was_never_added_is_not_an_error() {
    let mut ctl = grounded("a.");
    ctl.ground(&[Part::new("missing", &[]).unwrap()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn solve_results_describe_themselves() {
    let sat = grounded("a.").solve(&[]).unwrap();
    assert_eq!(format!("{sat:?}"), "SolveResult(SATISFIABLE)");
    assert!(!sat.is_exhausted());
    let unsat = grounded("a :- not a.").solve(&[]).unwrap();
    assert_eq!(
        format!("{unsat:?}"),
        "SolveResult(UNSATISFIABLE, exhausted)"
    );
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let all = ctl.solve(&[]).unwrap();
    assert!(all.is_sat() && all.is_exhausted());
}

#[test]
fn a_part_can_be_grounded_again_with_other_parameters() {
    let mut ctl = Control::new().unwrap();
    ctl.add("step", &["t"], "p(t).").unwrap();
    for t in 1..=3 {
        ctl.ground(&[Part::new("step", &[Symbol::number(t)]).unwrap()])
            .unwrap();
        let p = Symbol::function("p", &[Symbol::number(t)]).unwrap();
        assert!(ctl.solve(&[(p, false).into()]).unwrap().is_unsat());
    }
}

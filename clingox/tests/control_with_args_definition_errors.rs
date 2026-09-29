//! A syntax error in a `-c` definition passed to `Control::with_args` is
//! `ErrorKind::Parse`, like every other parse failure (smoke notes); an error
//! in the options themselves stays what it was.
//!
//! Oracle: pyclingo 5.8.2 reports all of these as errors (code 1, "parsing
//! failed" or "too many messages"), and prints clingo's parse messages, `<a=b
//! c>:1:5-6: error: syntax error, ...`, which `with_args` also carries in its
//! message. `--nosuch` and `-c` without a value are option errors (code 2,
//! `ErrorKind::Logic`).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use clingox::{Control, ErrorKind};

fn kind(args: &[&str]) -> ErrorKind {
    Control::with_args(args.iter().copied())
        .err()
        .unwrap_or_else(|| panic!("{args:?} must fail"))
        .kind()
}

#[test]
fn a_definition_that_does_not_parse_is_a_parse_error() {
    for args in [
        &["-c", "a"][..],
        &["-c", "a=("],
        &["-c", "A=1"],
        &["-c", "a=b c"],
        &["-c", "=1"],
        &["--const", "x=+"],
        &["--const=a"],
        &["-c", "a=1", "-c", "b"],
    ] {
        assert_eq!(kind(args), ErrorKind::Parse, "{args:?}");
    }
}

#[test]
fn the_parse_error_carries_clingos_message() {
    let error = Control::with_args(["-c", "a=b c"]).err().unwrap();
    assert!(
        error
            .to_string()
            .contains("syntax error, unexpected <IDENTIFIER>"),
        "{error}"
    );
}

#[test]
fn option_errors_are_not_parse_errors() {
    assert_eq!(kind(&["--nosuch"]), ErrorKind::Logic);
    assert_eq!(kind(&["-c"]), ErrorKind::Logic);
}

#[test]
fn a_good_definition_still_works() {
    let mut ctl = Control::with_args(["-c", "n=3"]).unwrap();
    ctl.add_base("a(n).").unwrap();
    ctl.ground(&[clingox::Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models[0].symbols().len(), 1);
    assert_eq!(models[0].symbols()[0].to_string(), "a(3)");
}

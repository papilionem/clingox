//! Patch U1 (`clingox-sys/patches/U1-division-traps.patch`): integer division
//! and modulo that trap in clingo 5.8.2 behave like clingo's other undefined
//! arithmetic instead of killing the process (UPSTREAM-ISSUES U1).
//!
//! Without the patch, every test that evaluates a trapping case kills this
//! test binary with SIGFPE (or a WebAssembly trap), which is why these tests
//! have a binary of their own.
//!
//! The behaviour to match was pinned with the Python module `clingo` 5.8.2 on
//! the neighbouring cases that do not trap:
//! - `clingo.parse_term("1/0")` and `clingo.parse_term("p(1/0)")` raise
//!   `RuntimeError: parsing failed` and log nothing;
//! - in a program, `p(1/0).`, `p(1\0).`, `p(X/Y) :- X=1, Y=0.` and
//!   `p(X\Y) :- X=1, Y=0.` each log one message of code `OperationUndefined`,
//!   `<block>:1:3-6: info: operation undefined:\n  (X/Y)` (with the
//!   operation's own text), and drop the rule instance: the model is empty.
//!
//! The trapping cases (`1\0` as a term, and the smallest number divided by -1
//! with `/` or `\`, anywhere) must do exactly the same. `INT_MIN \ -1` is
//! mathematically 0, but C++ leaves it undefined and x86 traps on it, so the
//! patch treats it as undefined, like `INT_MIN / -1`.
//!
//! A system library does not get the patch (RULES 8); on one, these tests
//! return early, because the cases would kill the process.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail loudly on unexpected errors"
)]

use std::sync::{Arc, Mutex};

use clingox::{Control, ErrorKind, MessageCode, Part, Symbol};

fn patched() -> bool {
    if clingox_sys::VENDORED {
        assert!(
            clingox_sys::PATCHES.contains(&"U1"),
            "the vendored build applies U1: {:?}",
            clingox_sys::PATCHES
        );
    }
    clingox_sys::VENDORED
}

/// The models' shown symbols, and every message clingo logged, for `program`.
fn solve(program: &str) -> (Vec<Vec<String>>, Vec<(MessageCode, String)>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    let mut ctl = Control::builder()
        .logger(move |code, text| sink.lock().unwrap().push((code, text.to_owned())))
        .build()
        .unwrap();
    ctl.add_base(program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_exhausted());
    let models = models
        .iter()
        .map(|m| m.symbols().iter().map(ToString::to_string).collect())
        .collect();
    let messages = log.lock().unwrap().clone();
    (models, messages)
}

fn assert_undefined_and_dropped(program: &str, operation: &str) {
    let (models, messages) = solve(program);
    assert_eq!(models, [Vec::<String>::new()], "{program}");
    assert_eq!(messages.len(), 1, "{program}: {messages:?}");
    let (code, text) = &messages[0];
    assert_eq!(*code, MessageCode::OperationUndefined, "{program}");
    assert!(
        text.contains("info: operation undefined"),
        "{program}: {text}"
    );
    assert!(text.contains(operation), "{program}: {text}");
}

fn assert_term_undefined(text: &str) {
    let err = text.parse::<Symbol>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse, "{text}");
    assert_eq!(err.messages().len(), 1, "{text}: {:?}", err.messages());
    assert_eq!(err.messages()[0].text(), "parsing failed", "{text}");
}

#[test]
fn a_zero_divisor_of_modulo_in_a_term_is_a_parse_error() {
    if !patched() {
        return;
    }
    // The reference: `/` by zero, which clingo already checks.
    assert_term_undefined("1/0");
    assert_term_undefined("1\\0");
    assert_term_undefined("p(1\\0)");
    assert_term_undefined("(3-3)\\(2-2)");
}

#[test]
fn the_smallest_number_divided_by_minus_one_in_a_term_is_a_parse_error() {
    if !patched() {
        return;
    }
    for text in [
        "(-2147483647-1)/-1",
        "(-2147483647-1)\\-1",
        "-2147483648/-1",
        "-2147483648\\-1",
        "p((-2147483647-1)/-1)",
    ] {
        assert_term_undefined(text);
    }
}

#[test]
fn the_smallest_number_divided_by_minus_one_in_a_program_is_undefined() {
    if !patched() {
        return;
    }
    assert_undefined_and_dropped("p(X/Y) :- X=-2147483647-1, Y=-1.", "(X/Y)");
    assert_undefined_and_dropped("p(X\\Y) :- X=-2147483647-1, Y=-1.", "(X\\Y)");
    assert_undefined_and_dropped("p(-2147483648/-1).", "/");
    assert_undefined_and_dropped("p(X) :- X = (-2147483647-1) / -1.", "/");
}

#[test]
fn a_program_goes_on_after_an_undefined_division() {
    if !patched() {
        return;
    }
    let (models, messages) = solve("q. p(X/Y) :- X=-2147483647-1, Y=-1. r :- q.");
    assert_eq!(models, [vec!["q".to_owned(), "r".to_owned()]]);
    assert_eq!(messages.len(), 1, "{messages:?}");
}

#[test]
fn matching_a_negated_variable_against_the_smallest_number_does_not_crash() {
    if !patched() {
        return;
    }
    // U1 review G1: `LinearTerm::match` (`gringo/domain.hh` binds `X` from
    // `p(X)`, then matches `-X+0` against p's domain) divided `c / m_` with
    // `m_ = -1` and no guard. No int `X` satisfies `-X = -2147483648` (that
    // needs `X = 2147483648`, outside `i32`), so the match must fail and `q`
    // stays empty, not crash the process.
    let (models, messages) = solve("p(-2147483648). q(X) :- p(X), p(-X+0).");
    assert_eq!(models, [vec!["p(-2147483648)".to_owned()]]);
    assert!(messages.is_empty(), "{messages:?}");
}

#[test]
fn unifying_a_negated_head_variable_against_the_smallest_number_does_not_crash() {
    if !patched() {
        return;
    }
    // U1 review G1: `GLinearTerm::match`, reached from dependency analysis
    // (`GValTerm::unify` unifies the head term `-Y+0` against `q`'s domain
    // while building the dependency graph, before any solving happens). The
    // same non-existent solution for `-Y = -2147483648` must not crash
    // grounding. `p` is never a fact, so the body literal `p(-2147483648)` is
    // undefined (gringo warns that it occurs in no rule head in the shape it
    // needs) and the rule never fires; no `p` atom is ever derived.
    let (models, messages) = solve("q(-2147483648). p(-Y+0) :- p(-2147483648), q(Y).");
    assert_eq!(models, [vec!["q(-2147483648)".to_owned()]]);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].0, MessageCode::AtomUndefined, "{messages:?}");
}

#[test]
fn divisions_that_do_not_trap_keep_their_values() {
    if !patched() {
        return;
    }
    // Python clingo 5.8.2, `clingo.parse_term`.
    for (text, value) in [
        ("7\\-2", 1),
        ("-7\\2", -1),
        ("-7/2", -3),
        ("7/-2", -3),
        ("(-2147483647-1)/1", -2_147_483_648),
        ("(-2147483647-1)\\2", 0),
        ("2147483647/-1", -2_147_483_647),
        ("(-2147483647-1)/2", -1_073_741_824),
    ] {
        let symbol = text.parse::<Symbol>().unwrap();
        assert_eq!(symbol.as_number(), Some(value), "{text}");
    }
    // Python clingo 5.8.2: `[p(-2147483648), q(0), r(-2147483647), s(-1),
    // t(-3)]`, and no message.
    let (models, messages) = solve(
        "p(X/Y) :- X=-2147483647-1, Y=1. q(X\\Y) :- X=-2147483647-1, Y=2. \
         r(X/Y) :- X=2147483647, Y=-1. s(-7\\2). t(7/-2).",
    );
    let mut expected = vec!["p(-2147483648)", "q(0)", "r(-2147483647)", "s(-1)", "t(-3)"];
    expected.sort_unstable();
    let mut found = models[0].clone();
    found.sort();
    assert_eq!(found, expected);
    assert!(messages.is_empty(), "{messages:?}");
}

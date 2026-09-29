//! Patch U35 (`clingox-sys/patches/U35-*.patch`, not written yet): an
//! `#external` statement whose type term is an arithmetic term that
//! grounding rewrites (`[X\2]`, `[X**2]`, `[|X|]`, ...) crashes clingo 5.8.2
//! with a null pointer dereference, instead of being grounded or rejected
//! like the neighbouring `[X+1]` (UPSTREAM-ISSUES U35).
//!
//! Every case here runs in a child process (this binary again), so a crash
//! fails one assertion with the signal in its message instead of aborting the
//! whole test binary and hiding the other cases.
//!
//! The behaviour to match, checked with the Python module `clingo` 5.8.2 and
//! the `clingo` command line on the neighbouring input that does not crash:
//! - a type term that is not `true`, `false`, `free` or `release` is skipped
//!   without a message (`ExternalStatement::report` has "TODO: report
//!   something"): `#external e(X) : f(X). [X+1]` and `[1]` and `[t(X\2)]`
//!   ground and declare no external;
//! - a variable of the type term that nothing binds is a grounding error,
//!   `unsafe variables`, exactly as for `#external e(X). [X+2]` and
//!   `#external e(1). [Y]`;
//! - a type term that is undefined for one instance (`[X/0]`) is logged as
//!   `operation undefined` and the instance is dropped; nothing crashes.
//!
//! So after the fix, each crashing term with a bound variable grounds and
//! declares no external, and each with an unbound variable is a `Runtime`
//! error naming unsafe variables. A patched build is required; a system
//! library does not get the patch (RULES 8), and on one this file returns
//! early. Spawning a child needs a host with processes, so the file is
//! host-only.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail loudly on unexpected errors"
)]
#![cfg(not(any(target_os = "android", target_os = "ios", target_family = "wasm")))]

use std::io::Write;
use std::process::Command;

use clingox::{Control, ErrorKind};

/// The environment variable that puts this binary in child mode, with the
/// program to ground as its value.
const CHILD: &str = "CLINGOX_TEST_U35_CHILD";
/// What a child prints in front of its outcome; the parent looks for this.
const RESULT: &str = "u35-result: ";

/// Whether this build is the vendored, patched one.
fn patched() -> bool {
    clingox_sys::VENDORED
}

#[test]
fn the_vendored_build_applies_u35() {
    if patched() {
        assert!(
            clingox_sys::PATCHES.contains(&"U35"),
            "the vendored build applies U35: {:?}",
            clingox_sys::PATCHES
        );
    }
}

/// Grounds `program` and describes what came out: the sorted external atoms
/// of the grounding, or the kind of the error and whether it names unsafe
/// variables.
fn outcome(program: &str) -> String {
    let mut ctl = Control::new().unwrap();
    ctl.add("base", &[], program).unwrap();
    if let Err(err) = ctl.ground(&[clingox::Part::base()]) {
        let unsafe_variables = err.to_string().contains("unsafe variables");
        return format!("error {:?} unsafe={unsafe_variables}", err.kind());
    }
    let atoms = ctl.symbolic_atoms().unwrap();
    let mut externals: Vec<String> = atoms
        .iter()
        .map(Result::unwrap)
        .filter(clingox::SymbolicAtom::is_external)
        .map(|atom| atom.symbol().to_string())
        .collect();
    externals.sort();
    format!("externals {externals:?}")
}

/// The child: grounds the program in `CLINGOX_TEST_U35_CHILD` and prints the
/// outcome. In a normal run of this binary the variable is unset and the test
/// does nothing.
#[test]
fn child_grounds_one_program() {
    let Ok(program) = std::env::var(CHILD) else {
        return;
    };
    // A newline first: the test harness has printed "test ... " on this line.
    let mut out = std::io::stdout();
    writeln!(out, "\n{RESULT}{}", outcome(&program)).expect("stdout is a pipe to the parent test");
    out.flush().expect("stdout is a pipe to the parent test");
}

/// Grounds `program` in a child process. Returns the outcome, or the way the
/// child died.
fn in_child(program: &str) -> String {
    let exe = std::env::current_exe().expect("the test binary knows its path");
    let output = Command::new(exe)
        .args([
            "--exact",
            "child_grounds_one_program",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, program)
        .output()
        .expect("the test binary runs again");
    let stdout = String::from_utf8_lossy(&output.stdout);
    match stdout.lines().find_map(|l| l.strip_prefix(RESULT)) {
        Some(found) if output.status.success() => found.to_owned(),
        _ => format!("child died: {:?}", output.status),
    }
}

/// Every program must produce its expected outcome. All failures are
/// collected, so one run shows the whole crashing set.
fn check(cases: &[(&str, &str)]) {
    if std::env::var_os(CHILD).is_some() || !patched() {
        return;
    }
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(program, expected)| {
            let got = in_child(program);
            (got != *expected)
                .then(|| format!("{program}\n    expected {expected}\n    got      {got}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} programs differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

const NONE: &str = "externals []";
const UNSAFE: &str = "error Runtime unsafe=true";

/// The binary operators that grounding rewrites into an auxiliary variable,
/// with a bound variable in the type term. The report was `\`, `**` and
/// `|.|`; `/`, `*` on two variables, `+`, `-`, `&`, `?` and `^` crash as well.
/// `X+1`, `X-1`, `X*2` and `2*X` are linear terms and never did.
#[test]
fn a_binary_operation_as_the_type_of_a_conditional_external_grounds() {
    check(&[
        ("f(1..3). #external e(X) : f(X). [X\\2]", NONE),
        ("f(1..3). #external e(X) : f(X). [X**2]", NONE),
        ("f(1..3). #external e(X) : f(X). [X/2]", NONE),
        ("f(1..3). #external e(X) : f(X). [X&2]", NONE),
        ("f(1..3). #external e(X) : f(X). [X?2]", NONE),
        ("f(1..3). #external e(X) : f(X). [X^2]", NONE),
        ("f(1..3). #external e(X) : f(X). [2\\X]", NONE),
        ("f(1..3). #external e(X) : f(X). [2/X]", NONE),
        ("f(1..3). #external e(X) : f(X). [X+X]", NONE),
        ("f(1..3). #external e(X) : f(X). [X-X]", NONE),
        ("f(1..3). #external e(X) : f(X). [X*X]", NONE),
        ("f(1..3). #external e(X) : f(X). [X**X]", NONE),
    ]);
}

/// The unary operators other than `-`: `|X|` and `~X`. `-X` is a linear
/// term and never crashed.
#[test]
fn a_unary_operation_as_the_type_of_a_conditional_external_grounds() {
    check(&[
        ("f(1..3). #external e(X) : f(X). [|X|]", NONE),
        ("f(1..3). #external e(X) : f(X). [~X]", NONE),
        ("f(1..3). #external e(X) : f(X). [|X-2|]", NONE),
    ]);
}

/// Nesting: the crash needs the outermost node of the type term to be
/// rewritten, however deep the offending operation sits below it.
#[test]
fn a_nested_arithmetic_type_grounds() {
    check(&[
        ("f(1..3). #external e(X) : f(X). [(X\\2)+1]", NONE),
        ("f(1..3). #external e(X) : f(X). [(X+1)\\2]", NONE),
        ("f(1..3). #external e(X) : f(X). [|X-2|+1]", NONE),
        ("f(1..3). #external e(X) : f(X). [-(X\\2)]", NONE),
    ]);
}

/// The condition can be more than one literal, negative, or over two
/// variables.
#[test]
fn other_conditions_with_an_arithmetic_type_ground() {
    check(&[
        ("f(1..3). #external e(X,Y) : f(X), f(Y). [X\\Y]", NONE),
        ("f(1..3). #external e(X) : f(X), not g(X). [X\\2]", NONE),
        ("f(1..3). #external e(X) : f(X), X\\2 == 1. [X\\2]", NONE),
    ]);
}

/// A variable that no literal binds is an error, as for the linear
/// `#external e(X). [X+2]`; today the check that would report it is the code
/// that crashes.
#[test]
fn an_unbound_variable_in_an_arithmetic_type_is_an_unsafe_variable_error() {
    check(&[
        ("#external e(X). [X\\2]", UNSAFE),
        ("#external e(X). [|X|]", UNSAFE),
        ("#external e(X). [~X]", UNSAFE),
        ("f(1..3). #external e(1) : f(1). [Y\\2]", UNSAFE),
        ("f(1..3). #external e(1) : f(1). [Y**2]", UNSAFE),
        // The references that already behave this way.
        ("#external e(X). [X+2]", UNSAFE),
        ("#external e(1). [Y]", UNSAFE),
    ]);
}

/// An operation that is undefined for every instance drops the instances and
/// nothing else; it did not crash before, and must not start to.
#[test]
fn an_undefined_arithmetic_type_declares_nothing() {
    check(&[
        ("f(1..3). #external e(X) : f(X). [X/0]", NONE),
        ("#external e(1). [1/0]", NONE),
    ]);
}

/// The statements around a formerly crashing one are unaffected, and the
/// ordinary types still declare their externals.
#[test]
fn other_externals_are_still_declared() {
    check(&[
        (
            "f(1..3). #external a. #external e(X) : f(X). [X\\2] #external b.",
            "externals [\"a\", \"b\"]",
        ),
        (
            "f(1..2). #external e(X) : f(X). [true]",
            "externals [\"e(1)\", \"e(2)\"]",
        ),
        // The references that never crashed: skipped types.
        ("f(1..3). #external e(X) : f(X). [X+1]", NONE),
        ("f(1..3). #external e(X) : f(X). [-X]", NONE),
        ("f(1..3). #external e(X) : f(X). [t(X\\2)]", NONE),
        ("#external e(1). [1\\2]", NONE),
    ]);
}

/// The error kind is the one clingo uses for the linear neighbour, so the
/// wrapper's mapping needs no special case.
#[test]
fn the_unsafe_error_has_the_same_kind_as_for_a_linear_type() {
    if std::env::var_os(CHILD).is_some() || !patched() {
        return;
    }
    let linear = in_child("#external e(X). [X+2]");
    let arithmetic = in_child("#external e(X). [X\\2]");
    assert_eq!(
        linear,
        format!("error {:?} unsafe=true", ErrorKind::Runtime)
    );
    assert_eq!(arithmetic, linear);
}

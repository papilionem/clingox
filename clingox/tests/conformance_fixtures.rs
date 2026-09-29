//! Ported from potassco/clingo v5.8.2
//! Source: `app/clingo/tests/lp/` (8 fixtures)
//! Differences: the normaliser is written in Rust, matching `run.py::normalize()`.
//! Fixtures read through `include_str!` from the submodule.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

mod conformance;

use conformance::assert_fixture_matches;

/// Include a .lp fixture from the submodule.
macro_rules! lp {
    ($name:literal) => {
        include_str!(concat!(
            "../../clingox-sys/clingo/app/clingo/tests/lp/",
            $name
        ))
    };
}

// ---------------------------------------------------------------------------
// Fixture: aggregates.lp -- uses #sum aggregates, unsatisfiable.
// ---------------------------------------------------------------------------

#[test]
fn lp_aggregates() {
    let program = lp!("aggregates.lp");
    let expected = lp!("aggregates.sol");
    assert_fixture_matches(&[], program, expected);
}

// ---------------------------------------------------------------------------
// Fixture: elevator.lp -- #include <incmode>, incremental elevator.
// The incmode driver is in conformance/mod.rs (ported from incmode.cc).
// ---------------------------------------------------------------------------

#[test]
fn lp_elevator() {
    let program = lp!("elevator.lp");
    let expected = lp!("elevator.sol");
    assert_fixture_matches(&[], program, expected);
}

// ---------------------------------------------------------------------------
// Fixture: external.lp -- #external directive.
// ---------------------------------------------------------------------------

#[test]
fn lp_external() {
    let program = lp!("external.lp");
    let expected = lp!("external.sol");
    assert_fixture_matches(&[], program, expected);
}

// ---------------------------------------------------------------------------
// Fixture: istop.lp -- #include <incmode>, #const istop="UNSAT".
// The incmode driver stops early when result matches istop.
// ---------------------------------------------------------------------------

#[test]
fn lp_istop() {
    let program = lp!("istop.lp");
    let expected = lp!("istop.sol");
    assert_fixture_matches(&[], program, expected);
}

// ---------------------------------------------------------------------------
// Fixture: numbers.lp -- numeric literal bases (42 in hex/octal/binary).
// ---------------------------------------------------------------------------

#[test]
fn lp_numbers() {
    let program = lp!("numbers.lp");
    let expected = lp!("numbers.sol");
    assert_fixture_matches(&["--models=0"], program, expected);
}

// ---------------------------------------------------------------------------
// Fixture: project.lp -- #project directive, .cmd gives `--project`.
// ---------------------------------------------------------------------------

#[test]
fn lp_project() {
    let program = lp!("project.lp");
    let expected = lp!("project.sol");
    assert_fixture_matches(&["--models=0", "--project"], program, expected);
}

// ---------------------------------------------------------------------------
// Fixture: show.lp -- #include <incmode>, #show, multi-step.
// The incmode driver is in conformance/mod.rs (ported from incmode.cc).
// ---------------------------------------------------------------------------

#[test]
fn lp_show() {
    let program = lp!("show.lp");
    let expected = lp!("show.sol");
    assert_fixture_matches(&["--models=0"], program, expected);
}

// ---------------------------------------------------------------------------
// Fixture: subset.lp -- #heuristic directives, .cmd gives `--heuristic=domain`.
// ---------------------------------------------------------------------------

#[test]
fn lp_subset() {
    let program = lp!("subset.lp");
    let expected = lp!("subset.sol");
    assert_fixture_matches(&["--models=0", "--heuristic=domain"], program, expected);
}

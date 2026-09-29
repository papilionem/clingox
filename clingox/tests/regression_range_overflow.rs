//! `p(2147483646..2147483647).` grounded forever
//! until the process was OOM-killed, because `RangeBinder::next`
//! (`libgringo/src/ground/literals.cc`) incremented an `int` past `INT_MAX`.
//! Patch U49 stops a range that ends at `INT_MAX`. Both values are in range,
//! so the program has exactly two atoms; clingo's semantics of `a..b` is the
//! integers from `a` to `b` inclusive.
//!
//! The program runs in a child process under a watchdog and an address-space
//! cap, because before the patch it never returns and grows without bound.
//! A system clingo has no patch (RULES 8): the tests return early there.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use std::time::Duration;

use clingox::{Control, Part};

const ENTRY: &str = "child_entry";

fn program(case: &str) -> &'static str {
    match case {
        "top" => "p(2147483646..2147483647).",
        "single" => "p(2147483647..2147483647).",
        "variable" => "b(2147483647). p(X..Y) :- b(Y), X = Y - 1.",
        "pool" => "p(2147483646..2147483647; 1..2).",
        "bottom" => "p(-2147483648..-2147483647).",
        "descending" => "p(2147483647..2147483646).",
        other => panic!("unknown case {other}"),
    }
}

/// The child side: grounds the case's program and prints its atoms.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    let mut ctl = Control::new().unwrap();
    ctl.add_base(program(&case)).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 1);
    let mut atoms: Vec<String> = models[0]
        .symbols()
        .iter()
        .map(ToString::to_string)
        .collect();
    atoms.sort();
    println!("CHILD-ATOMS {}", atoms.join(" "));
}

/// Runs `case` in a child under a 60 s watchdog and a 2 GiB address-space
/// cap, and returns its sorted atoms. `None` on a system clingo or a host that
/// cannot start the child.
fn atoms_of(case: &str) -> Option<String> {
    if !clingox_sys::VENDORED {
        eprintln!("SKIPPED: a system clingo has no range patch (RULES 8)");
        return None;
    }
    assert!(
        clingox_sys::PATCHES.contains(&"U49"),
        "the vendored build applies U49: {:?}",
        clingox_sys::PATCHES
    );
    // The cap keeps a runaway from taking the machine down, but a sanitizer
    // (`cargo xtask sanitize` sets its options variables, which the child
    // inherits) reserves terabytes of shadow memory and Android's runtime
    // needs the same; there the 60 s watchdog is the only guard.
    let sanitized = ["ASAN_OPTIONS", "LSAN_OPTIONS", "TSAN_OPTIONS"]
        .iter()
        .any(|v| std::env::var_os(v).is_some());
    let cap = (child::CAN_CAP_ADDRESS_SPACE && !sanitized).then_some(2 << 20);
    let out = child::run_child_with(ENTRY, case, Duration::from_secs(60), cap)?;
    assert_eq!(out.code, Some(0), "{case}: {out:?}");
    // libtest prints "test child_entry ... " on the same line.
    let (_, rest) = out
        .stdout
        .split_once("CHILD-ATOMS ")
        .unwrap_or_else(|| panic!("{case}: no atoms reported: {out:?}"));
    Some(rest.lines().next().unwrap_or_default().to_owned())
}

#[test]
fn a_range_ending_at_int_max_grounds_both_values() {
    if let Some(atoms) = atoms_of("top") {
        assert_eq!(atoms, "p(2147483646) p(2147483647)");
    }
}

#[test]
fn a_range_of_the_single_value_int_max_grounds_one_atom() {
    if let Some(atoms) = atoms_of("single") {
        assert_eq!(atoms, "p(2147483647)");
    }
}

#[test]
fn a_range_with_variable_bounds_ending_at_int_max_terminates() {
    if let Some(atoms) = atoms_of("variable") {
        assert_eq!(atoms, "b(2147483647) p(2147483646) p(2147483647)");
    }
}

#[test]
fn a_pool_with_a_range_ending_at_int_max_terminates() {
    if let Some(atoms) = atoms_of("pool") {
        assert_eq!(atoms, "p(1) p(2) p(2147483646) p(2147483647)");
    }
}

/// The symmetric boundary: the smallest ints, counting up, never overflow.
/// Kept so a fix that clamps both ends is checked at both.
#[test]
fn a_range_starting_at_int_min_grounds_both_values() {
    if let Some(atoms) = atoms_of("bottom") {
        assert_eq!(atoms, "p(-2147483647) p(-2147483648)");
    }
}

#[test]
fn an_empty_range_at_int_max_grounds_nothing() {
    if let Some(atoms) = atoms_of("descending") {
        assert_eq!(atoms, "");
    }
}

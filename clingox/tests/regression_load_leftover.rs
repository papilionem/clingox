//! Regression: a failed `Control::load` poisons the control, because clingo
//! keeps the rest of the file queued.
//!
//! Oracle: pyclingo 5.8.2 at the C level (the script and syntax cases):
//!
//! - `#script (bar) x #end. a. b.` with `bar` unregistered:
//!   `clingo_control_load` fails (runtime, `bar support not available`), and a
//!   following `clingo_control_add("z.")` and `ground` **succeed** with the
//!   model `[a, b, z]`: the remainder of the file was queued and is resumed.
//! - `a.` / `x :- (.` / `b.`: `load` fails, but the parse error leaves clingo
//!   in its error state, so the next `add` and `ground` fail too and the model
//!   is empty: the remainder is never grounded (already poisoned by kind).
//!
//! The crate must refuse the next call in both cases. No registration is
//! needed, so the cases run in process.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

#[path = "common/child.rs"]
mod child;

use clingox::{Control, ErrorKind, Part};

fn control() -> Control {
    Control::builder()
        .args(["0"])
        .logger(|_, _| {})
        .build()
        .unwrap()
}

/// After the failed load, every further call is refused, so the queued
/// remainder (`b`) can never reach the grounder.
fn assert_refused(ctl: &mut Control) {
    let err = ctl.add_base("z.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned, "add: {err}");
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned, "ground: {err}");
    let err = ctl.solve_all().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned, "solve: {err}");
}

#[test]
fn a_load_that_fails_on_an_unknown_script_type_poisons_the_control() {
    let file = child::fixture("leftover_script.lp", "#script (bar) x #end. a. b.\n");
    let mut ctl = control();
    let err = ctl.load(&file).unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::Runtime,
        "the kind is clingo's, unchanged"
    );
    assert!(
        err.to_string().contains("bar support not available"),
        "{err}"
    );
    assert_refused(&mut ctl);
}

#[test]
fn a_load_with_a_syntax_error_in_the_middle_poisons_the_control() {
    let file = child::fixture("leftover_syntax.lp", "a.\nx :- (.\nb.\n");
    let mut ctl = control();
    let err = ctl.load(&file).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    assert_refused(&mut ctl);
}

#[test]
fn a_load_that_succeeds_leaves_the_control_usable() {
    let file = child::fixture("leftover_ok.lp", "a.\nb.\n");
    let mut ctl = control();
    ctl.load(&file).unwrap();
    ctl.add_base("z.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    let mut atoms: Vec<String> = models[0]
        .symbols()
        .iter()
        .map(ToString::to_string)
        .collect();
    atoms.sort();
    assert_eq!(atoms, ["a", "b", "z"]);
}

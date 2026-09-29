//! An interrupted result is never conclusive.
//!
//! clingo 5.8.2 can report a search interrupted at its very start as
//! unsatisfiable and exhausted, for a satisfiable program too (checked with
//! the Python module on `{a;b}.`). clingox never passes that on: an
//! interrupted result is neither unsatisfiable nor exhausted, whatever
//! clingo's flags say. Which searches clingo misreports depends on timing, so
//! these tests stop many searches at their start and check every result; the
//! assertions hold for each one, so they cannot be flaky. The unit tests in
//! `SolveResult` and `Outcome` force the misreported flags deterministically.

#![forbid(unsafe_code)]
#![cfg(not(all(target_family = "wasm", not(target_feature = "atomics"))))]

use std::time::Duration;

use clingox::{Control, Part, SolveOptions, SolveResult};

/// clingo misreports about 1 in 4,000 of these searches natively, so each test
/// meets the case several times on most runs.
const RUNS: usize = 20_000;

/// `{a;b}.` has four answer sets, so no result of it may say unsatisfiable.
fn satisfiable() -> Control {
    let mut ctl = Control::with_args(["--models=0"]).expect("the arguments are valid");
    ctl.add_base("{a;b}.").expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn assert_inconclusive_if_interrupted(result: SolveResult) {
    assert!(!result.is_unsat(), "{result:?}");
    if result.is_interrupted() {
        assert!(!result.is_exhausted(), "{result:?}");
        assert!(result.is_sat() || result.is_unknown(), "{result:?}");
    }
}

#[test]
fn a_zero_timeout_never_reports_a_satisfiable_program_unsatisfiable() {
    let mut ctl = satisfiable();
    for _ in 0..RUNS {
        let result = ctl
            .solve_with(SolveOptions::new().timeout(Duration::ZERO))
            .expect("the program solves");
        assert_inconclusive_if_interrupted(result);
    }
}

#[test]
fn an_early_cancel_never_reports_a_satisfiable_program_unsatisfiable() {
    let mut ctl = satisfiable();
    for _ in 0..RUNS {
        let mut handle = ctl.solve_async(&[]).expect("the search starts");
        handle.cancel().expect("the search stops");
        let result = handle.close().expect("the search closes");
        assert_inconclusive_if_interrupted(result);
    }
}

//! Patch U53 (`clingox-sys/patches/U53-reify-steps.patch`): through
//! `clingo_control_register_backend`, `reify_steps` alone had no effect and
//! `reify_sccs` turned on step numbers as well, because clingo built the
//! reifier with the SCC flag in both places (UPSTREAM-ISSUES U53).
//!
//! The expected texts are the output of the `clingo` command line, which takes
//! a different, correct path: `python3 -m clingo --output=reify` with
//! `--reify-steps` or `--reify-sccs` (pyclingo 5.8.2, 2026-10-03), for
//! [`PROGRAM`]. With both flags set, the backend writer and the command line
//! print the same 26 lines byte for byte, so the two paths format alike.
//!
//! A system clingo has no patch (RULES 8): the tests that need it return early
//! there.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test helpers fail loudly on unexpected errors, and say why they skip"
)]

use std::path::PathBuf;

use clingox::backend::BackendWriterKind;
use clingox::{Control, Part};

/// A positive cycle (`a` and `b`), so `reify_sccs` has a component to report.
const PROGRAM: &str = "a :- b. b :- a. a :- not c. c :- not a.";

/// `clingo --output=reify --reify-steps`: every fact carries step 0.
const STEPS: &str = r"tag(incremental).
atom_tuple(0,0).
atom_tuple(0,1,0).
literal_tuple(0,0).
literal_tuple(0,-2,0).
rule(disjunction(0),normal(0),0).
atom_tuple(1,0).
atom_tuple(1,2,0).
literal_tuple(1,0).
literal_tuple(1,-1,0).
rule(disjunction(1),normal(1),0).
atom_tuple(2,0).
atom_tuple(2,3,0).
literal_tuple(2,0).
literal_tuple(2,2,0).
rule(disjunction(2),normal(2),0).
literal_tuple(3,0).
literal_tuple(3,3,0).
rule(disjunction(1),normal(3),0).
output(b,3,0).
output(a,2,0).
literal_tuple(4,0).
literal_tuple(4,1,0).
output(c,4,0).
";

/// `clingo --output=reify --reify-sccs`: two `scc` facts, no step numbers.
const SCCS: &str = r"tag(incremental).
atom_tuple(0).
atom_tuple(0,1).
literal_tuple(0).
literal_tuple(0,-2).
rule(disjunction(0),normal(0)).
atom_tuple(1).
atom_tuple(1,2).
literal_tuple(1).
literal_tuple(1,-1).
rule(disjunction(1),normal(1)).
atom_tuple(2).
atom_tuple(2,3).
literal_tuple(2).
literal_tuple(2,2).
rule(disjunction(2),normal(2)).
literal_tuple(3).
literal_tuple(3,3).
rule(disjunction(1),normal(3)).
output(b,3).
output(a,2).
literal_tuple(4).
literal_tuple(4,1).
output(c,4).
scc(0,2).
scc(0,3).
";

/// `clingo --output=reify`: neither.
const PLAIN: &str = r"tag(incremental).
atom_tuple(0).
atom_tuple(0,1).
literal_tuple(0).
literal_tuple(0,-2).
rule(disjunction(0),normal(0)).
atom_tuple(1).
atom_tuple(1,2).
literal_tuple(1).
literal_tuple(1,-1).
rule(disjunction(1),normal(1)).
atom_tuple(2).
atom_tuple(2,3).
literal_tuple(2).
literal_tuple(2,2).
rule(disjunction(2),normal(2)).
literal_tuple(3).
literal_tuple(3,3).
rule(disjunction(1),normal(3)).
output(b,3).
output(a,2).
literal_tuple(4).
literal_tuple(4,1).
output(c,4).
";

fn reified(kind: BackendWriterKind, name: &str) -> String {
    // `CARGO_TARGET_TMPDIR` is a path on the build host; on an Android device
    // the test binary writes to the device's own temporary directory.
    let base = if cfg!(target_os = "android") {
        std::env::temp_dir()
    } else {
        PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
    };
    let dir = base.join("patch_u53_reify_steps");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let mut ctl = Control::new().unwrap();
    ctl.register_backend_writer(kind, &path, false).unwrap();
    ctl.add_base(PROGRAM).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let _ = ctl.solve(&[]).unwrap();
    drop(ctl);
    std::fs::read_to_string(&path).unwrap()
}

#[test]
fn the_vendored_build_applies_u53() {
    if !clingox_sys::VENDORED {
        eprintln!("SKIPPED: a system clingo has no reify patch (RULES 8)");
        return;
    }
    assert!(
        clingox_sys::PATCHES.contains(&"U53"),
        "the vendored build applies U53: {:?}",
        clingox_sys::PATCHES
    );
}

#[test]
fn reify_steps_alone_adds_step_numbers() {
    if !clingox_sys::VENDORED {
        eprintln!("SKIPPED: a system clingo keeps U53 (RULES 8)");
        return;
    }
    assert_eq!(
        reified(BackendWriterKind::REIFY.reify_steps(), "steps.lp"),
        STEPS
    );
}

#[test]
fn reify_sccs_alone_adds_no_step_numbers() {
    if !clingox_sys::VENDORED {
        eprintln!("SKIPPED: a system clingo keeps U53 (RULES 8)");
        return;
    }
    assert_eq!(
        reified(BackendWriterKind::REIFY.reify_sccs(), "sccs.lp"),
        SCCS
    );
}

/// Unaffected by the defect: a guard that the fixture and the comparison are
/// sound, so the two tests above fail for the flag and nothing else.
#[test]
fn plain_reify_matches_the_command_line() {
    assert_eq!(reified(BackendWriterKind::REIFY, "plain.lp"), PLAIN);
}

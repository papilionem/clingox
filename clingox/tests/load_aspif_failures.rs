//! Regression test: a clasp-side failure while parsing an aspif file (as
//! opposed to a plain aspif syntax error) stayed `ErrorKind::Runtime`, which
//! does not poison by default (`Control::note`'s by-kind rule only poisons
//! `Parse`, `Logic`, `BadAlloc` and `Unknown`). `Control::load_aspif` then left
//! the control holding whatever partial program clasp had already accepted,
//! silently solvable and missing whatever came after the failure (a fuzz run
//! found 45 further undocumented poisoning `Logic` errors this way). Any
//! failure of `clingo_control_load_aspif`, once the files themselves opened,
//! poisons the control with its real error kind.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::path::Path;

use clingox::{Control, ErrorKind, Part};

/// A directory under the test binary's own scratch space (mirrors
/// `theory_and_load_regressions.rs::scratch_dir`), so parallel test binaries
/// and repeated runs never collide.
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let base = if cfg!(target_os = "android") {
        std::env::temp_dir()
    } else {
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
    };
    let dir = base.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fixture(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
    let file = dir.join(name);
    std::fs::write(&file, contents).unwrap();
    file
}

fn poisoned(ctl: &Control) -> bool {
    format!("{ctl:?}").contains("poisoned")
}

/// A well-formed aspif file whose weight rule (atom 2, lower bound `i32::MAX`,
/// two body atoms of weight `i32::MAX`) sums to more than an `i32`, which
/// `clingo_control_load_aspif` accepts as aspif syntax but clasp itself rejects
/// while building the program: a clasp-side failure, not an aspif parse error.
/// The file still declares later atoms (`c`, from `4 1 c 1 4`) that this
/// failure must keep out of the program.
///
/// Oracle: checked directly against clingo 5.8.2 through the Python module's
/// C API, 2026-09-29: `clingo_control_load_aspif` on this file fails with a
/// runtime error ("Integer overflow!" from clasp's `simplifySum`, without a
/// file position, so not a located aspif syntax error).
///
/// The out-of-range literal `i32::MIN` also makes clasp reject a load, but only
/// after clingo has grown its tables in proportion to it (UPSTREAM-ISSUES U22),
/// which takes about 19 GB on 64-bit hosts and exhausts a 32-bit address space,
/// so it is no fixture for a test that must run everywhere.
const CLASP_SIDE_FAILURE: &str = "asp 1 0 0\n1 0 1 1 0 0\n4 1 a 1 1\n\
     1 0 1 2 1 2147483647 2 3 2147483647 4 2147483647\n1 0 1 3 0 0\n1 0 1 4 0 0\n4 1 c 1 4\n0\n";

#[test]
fn a_clasp_side_load_aspif_failure_poisons_the_control() {
    let dir = scratch_dir("load_aspif_failures_clasp_side");
    let file = write_fixture(&dir, "clasp_side.aspif", CLASP_SIDE_FAILURE);

    let mut ctl = Control::new().unwrap();
    let loaded = ctl.load_aspif([&file]);
    assert!(
        loaded.is_err(),
        "the overflowing weight rule must fail to load"
    );
    assert_ne!(
        loaded.as_ref().unwrap_err().kind(),
        ErrorKind::Parse,
        "this is a clasp-side failure, not an aspif syntax error (the syntax error case is \
         already covered by theory_and_load_regressions.rs)"
    );
    assert!(
        poisoned(&ctl),
        "a clasp-side load_aspif failure must poison the control, whatever its error kind: {ctl:?}"
    );

    // The control cannot silently keep going with the partial program.
    let grounded = ctl.ground(&[Part::base()]);
    assert_eq!(
        grounded.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Poisoned),
        "{grounded:?}"
    );
    let added = ctl.add_base("d.");
    assert_eq!(
        added.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Poisoned)
    );
}

/// Every failure `Control::load_aspif` can report after its files opened
/// poisons: this covers the aspif-syntax case too (which was already
/// `ErrorKind::Parse`, one of the kinds that poisons by default) so a
/// future refactor cannot silently narrow the fix to only the clasp-side
/// kind above.
#[test]
fn a_malformed_aspif_file_also_poisons() {
    let dir = scratch_dir("load_aspif_failures_malformed");
    let file = write_fixture(&dir, "malformed.aspif", "asp 1 0 0\n1 0 1 1 0 0\nXYZZ\n0\n");

    let mut ctl = Control::new().unwrap();
    let err = ctl.load_aspif([&file]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse, "{err}");
    assert!(poisoned(&ctl), "{ctl:?}");
}

/// A file that never opens at all (checked from Rust before clingo sees
/// anything, as `load` already does) is unaffected by this decision: it is
/// a plain, non-poisoning `Runtime` error, since clingo's own aspif loading
/// never started.
#[test]
fn a_missing_aspif_file_does_not_poison() {
    let mut ctl = Control::new().unwrap();
    let missing = Path::new("/nonexistent/load_aspif_failures_missing.aspif");
    let err = ctl.load_aspif([missing]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime, "{err}");
    assert!(
        !poisoned(&ctl),
        "a file that never opened must not poison: {ctl:?}"
    );
}

//! Regression tests for the theory, load and consequence edge cases below.
//!
//! Each test states its oracle. All but the `is_consequence` one reproduce a
//! bug that was fixed; the `is_consequence` implementation already matched
//! clingo, so its test only pins the real (undocumented) behaviour the
//! documentation now states.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::fmt::Write as _;
use std::path::Path;

use clingox::{Consequence, Control, ErrorKind, Part, Symbol, TheoryTerm, TheoryTermKind};

fn sym(text: &str) -> Symbol {
    text.parse().expect("the term parses")
}

/// A directory under the test binary's own scratch space (mirrors
/// `api_control_load.rs::scratch_dir`), so parallel test binaries and
/// repeated runs never collide.
fn scratch_dir(name: &str) -> std::path::PathBuf {
    // `CARGO_TARGET_TMPDIR` is a path on the build host; on an Android
    // device the test binary writes to the device's own temporary directory.
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

// ---------------------------------------------------------------------------
// `TheoryElement::condition` lent a slice of clingo's
// reused scratch buffer.
// ---------------------------------------------------------------------------

/// Oracle: read, `libgringo/src/output/literals.cc:1551-1560`
/// (`DomainData::elemCond`, backing `clingo_theory_atoms_element_condition`):
/// every call clears and refills the shared `tempLits_` vector, which
/// reallocates as it grows. Also checked directly against clingo 5.8.2
/// through the Python module's C API (`clingo._internal`, `_lib`/`_ffi`):
/// the condition calls for a short and a much longer element returned the
/// same pointer, with the shorter call's contents overwritten once a later
/// one reallocated it (2026-09-27). Before the fix, `condition()` returned a
/// borrow of that buffer (`&'c [ProgramLiteral]`), so holding one condition
/// across later, longer ones read stale or freed memory (UPSTREAM-ISSUES
/// U24). The fix copies the literals into an owned `Vec` inside the raw
/// call, so a later, longer condition can no longer invalidate an earlier
/// one still held.
///
/// The smallest shape in the finding (`&a { 1: p; 2: p,q; 3: p,q,r }.`, three
/// elements) did not reliably show a wrong value in a plain debug build: the
/// freed allocation was not always overwritten before the read. Eight
/// elements, with conditions of growing length, reproduced a wrong value in
/// every one of eight runs tried while writing this test, so that shape is
/// used here; `cargo xtask sanitize`'s `AddressSanitizer` run catches the same
/// bug directly, as a heap-use-after-free, regardless of the allocator's
/// behaviour on a given run.
#[test]
fn theory_element_condition_survives_later_calls_that_reuse_clingos_scratch_buffer() {
    const ELEMENTS: i32 = 8;

    let mut choices = String::new();
    let mut elements_source = String::new();
    for i in 1..=ELEMENTS {
        let _ = write!(choices, "{{p{i}}}. ");
        let condition: Vec<String> = (1..=i).map(|j| format!("p{j}")).collect();
        if i > 1 {
            elements_source.push_str("; ");
        }
        let _ = write!(elements_source, "{i}: {}", condition.join(","));
    }
    let program = format!(
        "{choices}#theory t {{ term {{ }}; &a/0 : term, head }}. &a {{ {elements_source} }}."
    );

    let mut ctl = Control::new().unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let p1_lit = ctl
        .symbolic_atoms()
        .unwrap()
        .find(sym("p1"))
        .unwrap()
        .expect("p1 is an atom")
        .literal();

    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let mut elements = atom.elements().unwrap();
    // Order by the element's own tuple value, not clingo's internal order,
    // which `clingo.h` does not document.
    elements.sort_by_key(|e| match atoms.term(e.tuple().unwrap()[0]).unwrap() {
        TheoryTerm::Number(n) => n,
        other => panic!("expected a number term, got {other:?}"),
    });
    assert_eq!(elements.len(), usize::try_from(ELEMENTS).unwrap());

    // Hold the shortest element's condition live across every later, longer
    // read, which clears, refills and eventually reallocates clingo's
    // shared scratch buffer.
    let first = elements[0].condition().unwrap();
    assert_eq!(first, vec![p1_lit], "the first element's own literal");
    for element in &elements[1..] {
        let _later = element.condition().unwrap();
    }

    assert_eq!(
        first,
        vec![p1_lit],
        "the first element's condition must still hold its own literal after \
         later, longer conditions reused and reallocated clingo's buffer"
    );
}

// ---------------------------------------------------------------------------
// `TheoryElement::condition_id` poisoned the control for
// clingo's "no condition" sentinel, `0`.
// ---------------------------------------------------------------------------

/// Oracle: checked directly against clingo 5.8.2 through the Python module's
/// C API (`clingo._internal`, `from clingo._internal import _lib, _ffi`,
/// 2026-09-27): `clingo_theory_atoms_element_condition_id` on a bare element
/// such as the `1` in `&a { 1 }.` reports `0`, clingo's documented sentinel
/// for "no condition". Before the fix this reached
/// `ProgramLiteral::from_raw`, which rejects `0`, turning an ordinary,
/// unconditioned element into a poisoning `ErrorKind::Unknown`.
#[test]
fn condition_id_does_not_poison_the_control_for_an_element_with_no_condition() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("#theory t { term { }; &a/0 : term, head }. &a { 1 }.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let elements = atom.elements().unwrap();
    assert_eq!(elements.len(), 1);

    let result = elements[0].condition_id();
    assert!(
        result.is_ok(),
        "reading a bare element's condition id must not be an error: {result:?}"
    );
    assert_eq!(result.unwrap(), None, "a bare element has no condition id");
    assert!(!format!("{ctl:?}").contains("poisoned"), "{ctl:?}");

    // The control must still be usable: a real bug here poisons it (S3),
    // which would make everything from here on fail with `Poisoned`.
    ctl.add("more", &[], "b.").unwrap();
}

// ---------------------------------------------------------------------------
// A malformed aspif file bricked the control while
// `load_aspif` documented it as usable.
// ---------------------------------------------------------------------------

/// Oracle: checked directly against clingo 5.8.2 through the Python module's
/// C API, 2026-09-27: `clingo_control_load_aspif` on
/// `asp 1 0 0\n1 0 1 1 0 0\nXYZZ\n0\n` (a file that opens, then fails aspif
/// parsing) fails with a located "aspif error" message; a later
/// `clingo_control_add` then fails with "parsing failed" and
/// `clingo_control_ground` with "grounding stopped because of errors" (the
/// same logger-latch bug documented for `load` as UPSTREAM-ISSUES U23).
#[test]
fn load_aspif_on_a_malformed_file_is_a_parse_error_and_poisons() {
    let dir = scratch_dir("theory_and_load_regressions_load_aspif_malformed");
    let file = write_fixture(&dir, "malformed.aspif", "asp 1 0 0\n1 0 1 1 0 0\nXYZZ\n0\n");

    let mut ctl = Control::new().unwrap();
    let err = ctl.load_aspif([&file]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse, "{err}");
    assert!(format!("{ctl:?}").contains("poisoned"), "{ctl:?}");

    // As U23 for `load`: the control cannot parse or ground anything else
    // afterward.
    let err = ctl.add_base("a.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned, "{err}");
}

// ---------------------------------------------------------------------------
// `Model::is_consequence`'s docs claimed agreement with
// `is_true` outside brave/cautious enumeration, which projection breaks.
// ---------------------------------------------------------------------------

/// Oracle: checked directly against the Python module `clingo` 5.8.2,
/// 2026-09-27: solving `a. b. #project a.` with `--project`,
/// `Model.is_true` is `True` for both `a` and `b`, but `Model.is_consequence`
/// is `True` for `a` and `False` for `b`. clingo's implementation
/// (`libclingo/clingo/clingocontrol.hh:427-439`) forces `False` for any
/// literal that is not shown or projected once projection is on. clingox's
/// wrapper already matches this: only the rustdoc claim was wrong, so this
/// test needs no code change, only the doc fix that goes with it.
#[test]
fn is_consequence_disagrees_with_is_true_under_projection() {
    let mut ctl = Control::with_args(["--project"]).unwrap();
    ctl.add_base("a. b. #project a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let atoms = ctl.symbolic_atoms().unwrap();
    let lit_a = atoms
        .find(sym("a"))
        .unwrap()
        .expect("a is an atom")
        .literal();
    let lit_b = atoms
        .find(sym("b"))
        .unwrap()
        .expect("b is an atom")
        .literal();

    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle
        .next_model()
        .unwrap()
        .expect("the program has a model");

    assert!(model.is_true(lit_a).unwrap());
    assert_eq!(model.is_consequence(lit_a).unwrap(), Consequence::True);

    assert!(
        model.is_true(lit_b).unwrap(),
        "b is true in the single model"
    );
    assert_eq!(
        model.is_consequence(lit_b).unwrap(),
        Consequence::False,
        "b is not projected, so is_consequence disagrees with is_true"
    );
}

// ---------------------------------------------------------------------------
// `load` and `load_aspif` opened the file before checking
// for poisoning.
// ---------------------------------------------------------------------------

/// A poisoned control asked to `load` a missing file must report `Poisoned`,
/// not `Runtime`: poisoning is checked first, before clingox ever touches
/// the filesystem.
#[test]
fn load_on_a_poisoned_control_reports_poisoned_not_runtime() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err(); // a syntax error poisons (S3)
    assert!(format!("{ctl:?}").contains("poisoned"), "{ctl:?}");

    let missing = Path::new("/nonexistent/theory_and_load_regressions_load.lp");
    let err = ctl.load(missing).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned, "{err}");
}

/// As above, for `load_aspif`.
#[test]
fn load_aspif_on_a_poisoned_control_reports_poisoned_not_runtime() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    assert!(format!("{ctl:?}").contains("poisoned"), "{ctl:?}");

    let missing = Path::new("/nonexistent/theory_and_load_regressions_load.aspif");
    let err = ctl.load_aspif([missing]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned, "{err}");
}

// ---------------------------------------------------------------------------
// `TheoryTerm`'s public fields allowed `Compound` values
// whose `Display` panicked.
// ---------------------------------------------------------------------------

/// A function compound with no name (never produced by clingo itself, but
/// representable through the public fields) must not panic `Display`.
#[test]
fn theory_term_display_never_panics_on_a_function_without_a_name() {
    let invalid = TheoryTerm::Compound {
        kind: TheoryTermKind::Function,
        name: None,
        arguments: vec![TheoryTerm::Number(1), TheoryTerm::Number(2)],
    };
    // The exact text is not contractual for a value clingo itself never
    // produces; only that formatting it does not panic.
    let text = invalid.to_string();
    assert_ne!(text, "");
}

/// A `Compound` with `kind` set to `Number` or `Symbol` (the two kinds the
/// type's own docs say never appear there) must not panic `Display` either.
#[test]
fn theory_term_display_never_panics_on_a_compound_with_a_number_or_symbol_kind() {
    for kind in [TheoryTermKind::Number, TheoryTermKind::Symbol] {
        let invalid = TheoryTerm::Compound {
            kind,
            name: Some("x"),
            arguments: vec![TheoryTerm::Number(1)],
        };
        let _ = invalid.to_string();
    }
}

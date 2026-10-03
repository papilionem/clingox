//! Reading a program from a file (`Control::load`) and from aspif files
//! (`Control::load_aspif`).
//!
//! Expected error kinds were checked against clingo 5.8.2 directly: `ctypes`
//! calls into the shared library the installed Python module bundles, at
//! `clingo_control_load`/`clingo_control_add`/`clingo_error_code` level, show
//! that a missing file and a genuine syntax error inside a loaded file both
//! raise `clingo_error_runtime` with one logged message; they differ in
//! whether that message carries a location.
//! The aspif fixture used here was checked against the Python module's own
//! `load_aspif` to reproduce `a. b :- a.` exactly before being embedded here.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::path::Path;

use clingox::{Control, ErrorKind, Part, Symbol};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a control can be created");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// A directory under the test binary's own scratch space, so parallel test
/// binaries and repeated runs never collide (mirrors
/// `api_errors.rs::a_file_name_with_colons_and_dashes_is_read_whole`).
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fixture(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
    let file = dir.join(name);
    std::fs::write(&file, contents).unwrap();
    file
}

// ---------------------------------------------------------------------------
// `load`
// ---------------------------------------------------------------------------

#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn load_reads_a_file_and_its_rules_are_grounded_and_solved() {
    let dir = scratch_dir("api_control_load_success");
    let file = write_fixture(&dir, "program.lp", "a. b :- a.\n");

    let mut ctl = Control::new().unwrap();
    ctl.load(&file).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    clingox::testing::assert_models!(models, ["a b"]);
}

#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn load_on_a_missing_file_is_a_runtime_error_and_does_not_poison() {
    let dir = scratch_dir("api_control_load_missing");
    let missing = dir.join("does-not-exist.lp");

    let mut ctl = Control::new().unwrap();
    let err = ctl.load(&missing).unwrap_err();
    // Pinned against clingo 5.8.2 itself (see the module doc comment): a
    // missing file gives `clingo_error_runtime` with a message that carries
    // no location, unlike a syntax error inside a file that was opened.
    assert_eq!(err.kind(), ErrorKind::Runtime, "{err}");
    assert!(!format!("{ctl:?}").contains("poisoned"), "{ctl:?}");

    // The control is still usable afterward, as every other `Runtime` error
    // in the crate leaves it (`ground`, `try_assign_external`).
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn load_on_a_malformed_file_is_a_parse_error_with_captured_messages() {
    let dir = scratch_dir("api_control_load_malformed");
    let file = write_fixture(&dir, "bad.lp", "a :- b c.\n");

    let mut ctl = Control::new().unwrap();
    let err = ctl.load(&file).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse, "{err}");

    // Negative control #6: `load` must capture clingo's messages, exactly as
    // `add` does for a syntax error.
    assert!(
        !err.messages().is_empty(),
        "a malformed file must report at least one message"
    );
    let message = &err.messages()[0];
    assert!(
        message.text().contains("syntax error"),
        "{}",
        message.text()
    );
    let location = message.location().expect("a syntax error has a position");
    assert_eq!(location.line(), 1);

    // Poisons on the same terms as a parse error from `add` (S3, "Poisoning,
    // settled").
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

// ---------------------------------------------------------------------------
// `load_aspif`
// ---------------------------------------------------------------------------

/// A hand-written aspif program equivalent to `a. b :- a.`: one fact atom
/// (id 1, `a`), one rule deriving atom 2 (`b`) from it, and the two output
/// statements that give them their names. Checked against the Python
/// module's `load_aspif` to solve to the same single model, `{a, b}`, as the
/// text form.
const ASPIF_A_THEN_B: &str = "asp 1 0 0\n1 0 1 1 0 0\n1 0 1 2 0 1 1\n4 1 a 1 1\n4 1 b 1 2\n0\n";

#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn load_aspif_reproduces_the_model_of_the_equivalent_source_text() {
    let dir = scratch_dir("api_control_load_aspif");
    let file = write_fixture(&dir, "fact_ab.aspif", ASPIF_A_THEN_B);

    let mut ctl = Control::new().unwrap();
    ctl.load_aspif([&file]).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    clingox::testing::assert_models!(models, ["a b"]);

    // Same result as the equivalent program text, read with `add_base`.
    assert_eq!(
        models[0].symbols(),
        solved_symbols(&mut grounded("a. b :- a."))
    );
}

/// The shown symbols of the first model of a fresh solve, or an empty vector
/// if the search has none.
fn solved_symbols(ctl: &mut Control) -> Vec<Symbol> {
    match ctl.solve_first().unwrap() {
        clingox::Outcome::Sat(model, _) => model.symbols().to_vec(),
        clingox::Outcome::Unsat | clingox::Outcome::Unknown(_) => Vec::new(),
    }
}

#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn load_aspif_accepts_more_than_one_file_merged_into_one() {
    // The header says only the first file should carry the preamble
    // (`asp 1 0 0`); a second file with just its rules is merged into the
    // same program.
    let dir = scratch_dir("api_control_load_aspif_merge");
    let first = write_fixture(
        &dir,
        "first.aspif",
        "asp 1 0 0\n1 0 1 1 0 0\n4 1 a 1 1\n0\n",
    );
    let second = write_fixture(&dir, "second.aspif", "1 0 1 2 0 1 1\n4 1 b 1 2\n0\n");

    let mut ctl = Control::new().unwrap();
    ctl.load_aspif([&first, &second]).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    clingox::testing::assert_models!(models, ["a b"]);
}

// `load_aspif` has the same
// upstream bug as `load` (UPSTREAM-ISSUES U23, checked with pyclingo 5.8.2: a
// missing aspif file makes every later `add` fail with "parsing failed").
#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn load_aspif_on_a_missing_file_is_a_runtime_error_and_leaves_the_control_usable() {
    let dir = scratch_dir("api_control_load_aspif_missing");
    let missing = dir.join("does-not-exist.aspif");

    let mut ctl = Control::new().unwrap();
    let err = ctl.load_aspif([&missing]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime, "{err}");
    assert!(!format!("{ctl:?}").contains("poisoned"), "{ctl:?}");

    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

/// A string symbol is read lossily, unlike AST text: clingo accepts a
/// string constant that is not valid UTF-8 from a file, and
/// `Symbol::as_string` returns it with U+FFFD in place of the invalid byte
/// (a `&'static str` from clingo's symbol table cannot carry an error; AST
/// reads return `ErrorKind::Utf8` instead, see `api_ast_parse.rs`).
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
#[test]
fn a_non_utf8_string_constant_from_a_file_reads_back_with_a_replacement_character() {
    let dir = scratch_dir("api_control_load_non_utf8_string");
    let file = dir.join("bad_utf8_string.lp");
    std::fs::write(&file, b"p(\"a\xffb\").\n").unwrap();
    let mut ctl = Control::new().unwrap();
    ctl.load(&file).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert_eq!(models.len(), 1);
    let symbols = models[0].symbols();
    assert_eq!(symbols.len(), 1);
    let argument = symbols[0].arguments().unwrap()[0];
    assert_eq!(argument.as_string(), Some("a\u{fffd}b"));
}

/// The string symbol of `p("a\xffb").` read from a file, its whole fact, and the
/// control that holds it.
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn non_utf8_fact(dir: &str) -> (Control, Symbol) {
    let dir = scratch_dir(dir);
    let file = dir.join("bad_utf8_string.lp");
    std::fs::write(&file, b"p(\"a\xffb\").\n").unwrap();
    let mut ctl = Control::new().unwrap();
    ctl.load(&file).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    let fact = models[0].symbols()[0];
    (ctl, fact)
}

/// Displaying such a symbol must not fail: `Display` writes U+FFFD for the
/// invalid byte, the rule `as_string` follows. It used to return `fmt::Error`,
/// which made `to_string` and `{}` panic.
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
#[test]
fn a_non_utf8_string_symbol_displays_with_a_replacement_character() {
    let (_ctl, fact) = non_utf8_fact("api_control_load_non_utf8_display");
    assert_eq!(fact.to_string(), "p(\"a\u{fffd}b\")");
    assert_eq!(format!("{fact:?}"), "Symbol(p(\"a\u{fffd}b\"))");
}

/// A model that holds such a symbol displays too.
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
#[test]
fn a_model_with_a_non_utf8_string_displays() {
    let dir = scratch_dir("api_control_load_non_utf8_model_display");
    let file = dir.join("bad_utf8_string.lp");
    std::fs::write(&file, b"p(\"a\xffb\").\n").unwrap();
    let mut ctl = Control::new().unwrap();
    ctl.load(&file).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle.next_model().unwrap().unwrap();
    assert!(model.to_string().contains("p(\"a\u{fffd}b\")"), "{model}");
    drop(handle);
}

/// `add_facts` must not add a different fact than it was given: printing the
/// symbol lossily would turn the byte into U+FFFD. It reports `Utf8`, which
/// does not poison, and adds nothing. It used to report `BadAlloc`, which
/// poisons.
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
#[test]
fn add_facts_refuses_a_non_utf8_string_symbol_without_poisoning() {
    let (mut ctl, fact) = non_utf8_fact("api_control_load_non_utf8_add_facts");
    let err = ctl.add_facts([fact]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Utf8, "{err}");
    ctl.add_base("q.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert_eq!(models.len(), 1);
}

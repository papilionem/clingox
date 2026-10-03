//! `register_backend_writer`: dumping the ground program to
//! a file in aspif, smodels or reify format.
//!
//! Every expected value was checked directly against clingo 5.8.2 (the
//! Python module `clingo`, `ctl.register_backend`), 2026-09-27. One
//! discrepancy: the installed clingo 5.8.2 build's `smodels` backend writer
//! produces the same aspif-format text as the `aspif` backend, not the
//! legacy smodels format, for every fixture tried here.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::path::PathBuf;

use clingox::backend::BackendWriterKind;
use clingox::{Control, ErrorKind, Part};

/// A directory under the test binary's own scratch space (mirrors
/// `api_control_load.rs::scratch_dir`).
fn scratch_dir(name: &str) -> PathBuf {
    // `CARGO_TARGET_TMPDIR` is a path on the build host; on an Android
    // device the test binary writes to the device's own temporary directory.
    let base = if cfg!(target_os = "android") {
        std::env::temp_dir()
    } else {
        PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
    };
    let dir = base.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn register_backend_writer_aspif_writes_a_file_load_aspif_can_read_back() {
    let dir = scratch_dir("api_backend_writer_aspif");
    let path = dir.join("out.aspif");

    let mut writer = Control::new().unwrap();
    writer
        .register_backend_writer(BackendWriterKind::ASPIF, &path, false)
        .unwrap();
    writer.add_base("a. b :- a.").unwrap();
    writer.ground(&[Part::base()]).unwrap();
    assert!(writer.solve(&[]).unwrap().is_sat());

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.starts_with("asp 1 0 0"), "{content}");

    let mut reader = Control::with_args(["--models=0"]).unwrap();
    reader.load_aspif([&path]).unwrap();
    let (_, models) = reader.solve_all().unwrap();
    assert_eq!(models.len(), 1);
    let mut symbols: Vec<String> = models[0]
        .symbols()
        .iter()
        .map(ToString::to_string)
        .collect();
    symbols.sort();
    assert_eq!(symbols, vec!["a".to_owned(), "b".to_owned()]);
}

/// `replace: true` behaves like `register_observer`'s: the file still gets
/// the ground program, but nothing reaches the solver. Checked directly
/// against clingo 5.8.2.
#[test]
fn register_backend_writer_replace_true_still_writes_but_nothing_reaches_the_solver() {
    let dir = scratch_dir("api_backend_writer_replace");
    let path = dir.join("out.aspif");

    let mut ctl = Control::new().unwrap();
    ctl.register_backend_writer(BackendWriterKind::ASPIF, &path, true)
        .unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let result = ctl.solve(&[]).unwrap();
    assert!(!result.is_sat());
    assert!(!result.is_unsat());

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("asp 1 0 0"), "{content}");
}

/// Checked directly against clingo 5.8.2, for a program using a weight
/// constraint and a weak constraint as well as a plain fact and rule: the
/// `smodels` backend's file is byte-for-byte identical to the `aspif`
/// backend's, in the installed 5.8.2 build. This document does not guess a
/// "traditional" smodels format text: it pins the behaviour actually
/// observed and records the discrepancy.
#[test]
fn register_backend_writer_smodels_matches_aspif_byte_for_byte_in_this_clingo_build() {
    let program = "{x;y}. head :- 2 #sum {1,x:x; 1,y:y} >= 2. :~ x. [1@1]";
    let dir = scratch_dir("api_backend_writer_smodels");
    let aspif_path = dir.join("out.aspif");
    let smodels_path = dir.join("out.smodels");

    let mut aspif_ctl = Control::new().unwrap();
    aspif_ctl
        .register_backend_writer(BackendWriterKind::ASPIF, &aspif_path, false)
        .unwrap();
    aspif_ctl.add_base(program).unwrap();
    aspif_ctl.ground(&[Part::base()]).unwrap();
    let _ = aspif_ctl.solve(&[]).unwrap();

    let mut smodels_ctl = Control::new().unwrap();
    smodels_ctl
        .register_backend_writer(BackendWriterKind::SMODELS, &smodels_path, false)
        .unwrap();
    smodels_ctl.add_base(program).unwrap();
    smodels_ctl.ground(&[Part::base()]).unwrap();
    let _ = smodels_ctl.solve(&[]).unwrap();

    let aspif_content = std::fs::read_to_string(&aspif_path).unwrap();
    let smodels_content = std::fs::read_to_string(&smodels_path).unwrap();
    assert_eq!(aspif_content, smodels_content);

    // Since the content is aspif-format text, it also round-trips.
    let mut reader = Control::new().unwrap();
    reader.load_aspif([&smodels_path]).unwrap();
    assert!(reader.solve(&[]).unwrap().is_sat());
}

/// Checked directly against clingo 5.8.2: reifying `a. b :- a. {c}.` produces
/// `atom_tuple`/`literal_tuple`/`rule` facts and an `output/2` fact per shown
/// atom.
#[test]
fn register_backend_writer_reify_writes_reified_facts() {
    let dir = scratch_dir("api_backend_writer_reify");
    let path = dir.join("out.lp");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.register_backend_writer(BackendWriterKind::REIFY, &path, false)
        .unwrap();
    ctl.add_base("a. b :- a. {c}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 2);

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("tag(incremental)."), "{content}");
    assert!(content.contains("output(a,0)."), "{content}");
    assert!(content.contains("output(b,0)."), "{content}");
    assert!(content.contains("output(c,"), "{content}");
    assert!(content.contains("atom_tuple("), "{content}");
    assert!(content.contains("rule("), "{content}");
}

#[test]
fn register_backend_writer_refuses_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let dir = scratch_dir("api_backend_writer_poisoned");
    let path = dir.join("out.aspif");
    let err = ctl
        .register_backend_writer(BackendWriterKind::ASPIF, &path, false)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

/// Checked at the C level directly (`clingo_control_register_backend`,
/// `clingo_error_code`), since pyclingo reports every clingo error as
/// `RuntimeError`: a path that cannot be opened is `clingo_error_runtime`
/// (code 1), a plain, non-poisoning error, like a missing file for
/// `Control::load`.
#[test]
fn register_backend_writer_on_an_unopenable_path_is_a_runtime_error_that_does_not_poison() {
    let mut ctl = Control::new().unwrap();
    let err = ctl
        .register_backend_writer(
            BackendWriterKind::ASPIF,
            "/no/such/directory/out.aspif",
            false,
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
    assert!(!format!("{ctl:?}").contains("poisoned"));
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

/// `reify_sccs`/`reify_steps` only change anything combined with
/// `BackendWriterKind::REIFY`; this test only checks that the builder
/// methods exist and produce a still-valid, still-parseable file, not the
/// exact scc/step reification shape (out of this document's scope).
#[test]
fn reify_sccs_and_reify_steps_still_produce_a_readable_file() {
    let dir = scratch_dir("api_backend_writer_reify_sccs_steps");
    let path = dir.join("out.lp");
    let mut ctl = Control::new().unwrap();
    ctl.register_backend_writer(
        BackendWriterKind::REIFY.reify_sccs().reify_steps(),
        &path,
        false,
    )
    .unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert_ne!(content, "");
}

/// A writer whose file cannot take the output reports nothing (U52): clingo
/// writes through a `std::ofstream` and never checks it, so on a full disk
/// `ground` and `solve` succeed and the file is short or empty. `/dev/full`
/// opens fine and fails every write with `ENOSPC`. pyclingo 5.8.2 behaves the
/// same (`ctl.register_backend(BackendType.Aspif, "/dev/full")`, then a
/// successful ground and a `SAT` solve), 2026-10-03. If this starts failing,
/// clingo reports write errors now: update U52, the `register_backend_writer`
/// docs and DESIGN S3.
#[cfg(target_os = "linux")]
#[test]
fn a_write_error_in_the_backend_writer_is_not_reported() {
    use std::fmt::Write as _;

    let mut ctl = Control::new().unwrap();
    ctl.register_backend_writer(BackendWriterKind::ASPIF, "/dev/full", false)
        .unwrap();
    // About 6 MB of aspif, far more than one stream buffer: traced with strace,
    // the first write fails during `ground` and the stream writes nothing after
    // it, not even at the end-of-step flush.
    let mut program = String::new();
    for i in 0..200_000 {
        write!(program, "p({i}).").unwrap();
    }
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
    assert!(!format!("{ctl:?}").contains("poisoned"));
}

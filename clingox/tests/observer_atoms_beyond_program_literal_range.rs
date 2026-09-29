//! Regression test: the ground program observer trampolines handed a caller's
//! `GroundProgramObserver::rule` an `Atom`/`ProgramLiteral` built straight from
//! whatever aspif said, with no check against the types' own invariants; a
//! hostile aspif file naming an atom or literal outside
//! `ProgramLiteral::MAX_MAGNITUDE` reached `Atom::pos`/`neg`, whose
//! `debug_assert!` (relied on elsewhere in the crate as an invariant, not a
//! caller-facing check) then aborted a debug build. Observer trampolines
//! validate every atom and literal clingo passes against the types' invariants;
//! a value outside them stops grounding with a poisoning `ErrorKind::Runtime`
//! error that names the value, before the observer ever sees it.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::path::Path;
use std::sync::{Arc, Mutex};

use clingox::backend::Atom;
use clingox::observer::GroundProgramObserver;
use clingox::{Control, ErrorKind, ProgramLiteral, Result, Symbol};

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

/// Records every head atom and body literal `rule` sees, calling `pos`/`neg`
/// on each atom and negating each literal, exactly as a caller doing
/// ordinary bookkeeping would (this is what previously reached the
/// `debug_assert!`).
#[derive(Default, Clone)]
struct RecordEverything(Arc<Mutex<Vec<String>>>);
impl GroundProgramObserver for RecordEverything {
    fn rule(&mut self, _choice: bool, head: &[Atom], body: &[ProgramLiteral]) -> Result<()> {
        let mut log = self.0.lock().unwrap();
        for atom in head {
            log.push(format!(
                "head pos={} neg={}",
                atom.pos().get(),
                atom.neg().get()
            ));
        }
        for literal in body {
            log.push(format!(
                "body {} negated={}",
                literal.get(),
                (-*literal).get()
            ));
        }
        Ok(())
    }
}

/// `asp 1 0 0\n1 0 1 <atom> 0 0\n0\n`: one fact rule whose head atom is
/// `atom`, raw aspif, out of `ProgramLiteral::MAX_MAGNITUDE`'s range.
fn rule_with_head_atom(atom: u32) -> String {
    format!("asp 1 0 0\n1 0 1 {atom} 0 0\n0\n")
}

/// `asp 1 0 0\n1 0 1 1 0 1 <literal>\n0\n`: one rule `a1 :- <literal>.`
/// whose body literal is out of range.
fn rule_with_body_literal(literal: i64) -> String {
    format!("asp 1 0 0\n1 0 1 1 0 1 {literal}\n0\n")
}

fn load(
    dir_name: &str,
    file_name: &str,
    aspif: &str,
) -> (Control, clingox::Result<()>, Vec<String>) {
    let dir = scratch_dir(dir_name);
    let path = write_fixture(&dir, file_name, aspif);
    let mut ctl = Control::new().unwrap();
    let recorder = RecordEverything::default();
    ctl.register_observer(recorder.clone(), false).unwrap();
    let loaded = ctl.load_aspif([&path]);
    let log = recorder.0.lock().unwrap().clone();
    (ctl, loaded, log)
}

#[test]
fn a_head_atom_at_2_pow_30_is_rejected_with_runtime_and_poisons() {
    let (ctl, loaded, log) = load(
        "observer_atoms_beyond_range_head_2_30",
        "head_2_30.aspif",
        &rule_with_head_atom(1 << 30),
    );
    assert_eq!(
        loaded.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Runtime),
        "{loaded:?}"
    );
    assert!(poisoned(&ctl), "{ctl:?}");
    assert!(
        log.is_empty(),
        "the observer must never see an atom outside ProgramLiteral's invariants: {log:?}"
    );
}

#[test]
fn a_head_atom_at_i32_max_is_rejected_with_runtime_and_poisons() {
    let (ctl, loaded, log) = load(
        "observer_atoms_beyond_range_head_i32max",
        "head_i32max.aspif",
        &rule_with_head_atom(u32::try_from(i32::MAX).unwrap()),
    );
    assert_eq!(
        loaded.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Runtime)
    );
    assert!(poisoned(&ctl));
    assert!(log.is_empty(), "{log:?}");
}

#[test]
fn a_body_literal_at_2_pow_30_is_rejected_with_runtime_and_poisons() {
    let (ctl, loaded, log) = load(
        "observer_atoms_beyond_range_body_2_30",
        "body_2_30.aspif",
        &rule_with_body_literal(1 << 30),
    );
    assert_eq!(
        loaded.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Runtime)
    );
    assert!(poisoned(&ctl));
    assert!(log.is_empty(), "{log:?}");
}

#[test]
fn a_body_literal_at_negative_2_pow_31_is_rejected_with_runtime_and_poisons() {
    let (ctl, loaded, log) = load(
        "observer_atoms_beyond_range_body_neg_2_31",
        "body_neg_2_31.aspif",
        &rule_with_body_literal(i64::from(i32::MIN)),
    );
    assert_eq!(
        loaded.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Runtime)
    );
    assert!(poisoned(&ctl));
    assert!(log.is_empty(), "{log:?}");
}

/// An ordinary, in-range atom must still reach the observer and be usable
/// through `pos`/`neg` exactly as before: the new validation must not
/// reject legitimate input.
#[test]
fn an_ordinary_atom_still_reaches_the_observer() {
    let mut ctl = Control::new().unwrap();
    let recorder = RecordEverything::default();
    ctl.register_observer(recorder.clone(), false).unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[clingox::Part::base()]).unwrap();
    assert!(!poisoned(&ctl));
    let log = recorder.0.lock().unwrap().clone();
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(log[0].starts_with("head pos=1 neg=-1"), "{log:?}");
    let _ = Symbol::function("a", &[]).unwrap();
}

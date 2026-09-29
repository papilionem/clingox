//! Regression test: `Display` for
//! `TheoryAtom` and `TheoryElement` mapped any clingo failure to
//! `fmt::Error`; `ToString::to_string`'s blanket implementation `expect`s a
//! `Display` implementation never to fail, so `to_string()` (and any other
//! `Display` consumer) panicked instead of the caller ever seeing an
//! `Error`. `Display` never returns `fmt::Error` for a clingo failure;
//! it writes a placeholder such as `<error: ...>`, as the `Debug` impls
//! already do (`TheoryAtom`'s and `TheoryElement`'s own `Debug`, in
//! `clingox/src/theory.rs`, already format an unreadable field as
//! `<{err}>` instead of failing).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::panic::{AssertUnwindSafe, catch_unwind};

use clingox::{Control, Part};

const THEORY: &str = "#theory t { term { }; &a/0 : term, head }.";

/// A theory term id read from one control's grounding is meaningless (or
/// dangling) on a different control's later grounding (`Id`'s own rustdoc:
/// ids are reused from zero after every `solve`): reading it back through
/// `TheoryAtoms::term` on the fresh control fails, which is enough to feed
/// `TheoryAtom`'s `Display` a foreign, invalid id through code that is
/// itself sound but exercises a real `Err` path.
fn a_theory_atom_and_a_foreign_out_of_range_term_id() -> (Control, clingox::Id) {
    let mut other = Control::new().unwrap();
    // 50 distinct elements so the foreign id (the last element's tuple
    // entry) is certainly out of range on the fresh control below, which
    // only ever grounds one.
    let many: Vec<String> = (1..=50).map(|n| n.to_string()).collect();
    other
        .add_base(&format!("{THEORY} &a {{ {} }}.", many.join("; ")))
        .unwrap();
    other.ground(&[Part::base()]).unwrap();
    let foreign = {
        let atoms = other.theory_atoms().unwrap();
        let atom = atoms.iter().next().unwrap().unwrap();
        atom.elements().unwrap().last().unwrap().tuple().unwrap()[0]
    };
    (other, foreign)
}

#[test]
fn theory_term_display_does_not_panic_on_a_foreign_id_and_reports_it() {
    let (_kept_alive, foreign) = a_theory_atom_and_a_foreign_out_of_range_term_id();

    let mut ctl = Control::new().unwrap();
    ctl.add_base(&format!("{THEORY} &a {{ 1 }}.")).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atoms = ctl.theory_atoms().unwrap();
    // Read the atom (and settle its `Display` failure path's real subject)
    // before poisoning the control below: once poisoned, `TheoryAtoms::iter`
    // itself starts reporting `ErrorKind::Poisoned` for every item, which
    // would otherwise be what this test caught instead of the bug
    // it targets.
    let atom = atoms.iter().next().unwrap().unwrap();

    // Confirm the id really is unreadable here, or the test proves nothing.
    let stale = atoms.term(foreign);
    assert!(
        stale.is_err(),
        "the foreign id must be unreadable on this grounding"
    );

    // `TheoryAtoms::term` returns a `TheoryTerm`, not something `Display`
    // covers; `Display`'s own failure path is exercised through
    // `TheoryAtom`/`TheoryElement` instead, once the control is poisoned by
    // the failed read above (`ErrorKind::Logic` poisons, DESIGN S3).
    assert!(
        format!("{ctl:?}").contains("poisoned"),
        "an out-of-range theory id poisons the control: {ctl:?}"
    );

    let shown = catch_unwind(AssertUnwindSafe(|| atom.to_string()));
    assert!(
        shown.is_ok(),
        "Display must not panic once the control is poisoned: {:?}",
        shown
            .err()
            .and_then(|p| p.downcast_ref::<String>().cloned())
    );
    let text = shown.unwrap();
    assert!(
        text.contains("error") || text.contains("poisoned"),
        "a failed Display must write a visible placeholder, not blank or clingo's own text: \
         {text:?}"
    );

    let element = atom.elements();
    if let Ok(elements) = element {
        let shown = catch_unwind(AssertUnwindSafe(|| elements[0].to_string()));
        assert!(
            shown.is_ok(),
            "TheoryElement's Display must not panic either"
        );
    }
}

/// The same check directly through `format!`, which is how most callers
/// reach `Display` (not only the `ToString` blanket impl `to_string()`
/// uses).
#[test]
fn theory_atom_display_via_format_does_not_panic_once_poisoned() {
    let (_kept_alive, foreign) = a_theory_atom_and_a_foreign_out_of_range_term_id();
    let mut ctl = Control::new().unwrap();
    ctl.add_base(&format!("{THEORY} &a {{ 1 }}.")).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let _ = atoms.term(foreign);
    assert!(format!("{ctl:?}").contains("poisoned"));

    let shown = catch_unwind(AssertUnwindSafe(|| format!("{atom}")));
    assert!(shown.is_ok(), "{shown:?}");
}

//! `PropagateInit`'s own methods: watches, `freeze_literal`,
//! `symbolic_atoms`/`theory_atoms`, `check_mode`/`undo_mode`, `assignment`,
//! `add_literal`, `add_clause`/`add_weight_constraint`/
//! `add_minimize`/`propagate`, the post-`Stop` guard, and panics.
//!
//! Every expected value below was checked directly against clingo 5.8.2 (the
//! Python module `clingo`, 2026-09-28), never assumed from `clingo.h`'s prose.
//! Two findings are worth stating: the entailment direction of
//! `WeightConstraintKind::ImplicationLeft`/`ImplicationRight`, and that
//! `Assignment::size()` at `init` time is not necessarily zero.
//!
//! The trampoline dispatch is wired for all five `Propagator` methods (`init`,
//! `propagate`, `undo`, `check`, `decide`), not only `init`'s.
//! `PropagateControl` offers `thread_id`, `assignment`, `add_clause`,
//! `add_literal`, `add_watch`, `has_watch`, `remove_watch` and `propagate`. The
//! two tests here that dispatch through `propagate`/`undo`
//! (`add_watch_to_thread_restricts_the_watch_to_one_thread`,
//! `a_panic_in_undo_is_caught_and_resumes_after_the_solve`) use only
//! `thread_id`, or no `PropagateControl` method at all.
//! `api_propagator_control.rs` has the one test that uses `add_clause` (it is
//! `PropagateControl` behaviour).
//!
//! `PerThread<T>` is not used here: the one test that needs per-thread state
//! (`add_watch_to_thread_restricts_the_ watch_to_one_thread`) keeps its own
//! `OnceLock<Vec<AtomicBool>>` instead.
//!
//! `every_literal_taking_propagate_init_method_rejects_a_foreign_literal`
//! covers the cross-control literal validation guard: a `SolverLiteral`
//! legitimately obtained from a different, larger control is out of range here
//! and must be refused with `ErrorKind::InvalidInput` by every method that
//! takes one, never passed to clingo (which segfaults on it unguarded,
//! reproduced directly).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use clingox::propagate::{
    CheckMode, Flow, PropagateControl, PropagateInit, Propagator, SolverLiteral, UndoMode,
    WeightConstraintKind,
};
use clingox::{Control, ErrorKind, Part, Result, Signature};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a fresh control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn program_literal(init: &PropagateInit<'_>, name: &str) -> Result<clingox::ProgramLiteral> {
    let sig = Signature::new(name, 0)?;
    Ok(init
        .symbolic_atoms()?
        .by_signature(sig)
        .next()
        .unwrap_or_else(|| panic!("{name} is an atom"))?
        .literal())
}

/// A `SolverLiteral` legitimately obtained from a much larger control's
/// grounding: valid there, but out of range for a small control's own
/// assignment. Reproduced directly:
/// passing this literal straight to an unguarded `add_clause` on a small
/// control segfaults the process. `SolverLiteral` has no public raw
/// constructor, so this is the only way safe code can ever hold an invalid
/// one; the guard tests below check clingox's own validation refuses it
/// with `ErrorKind::InvalidInput` instead of reaching clingo.
fn foreign_literal() -> SolverLiteral {
    let biggest: Arc<Mutex<Option<SolverLiteral>>> = Arc::new(Mutex::new(None));

    struct Capture(Arc<Mutex<Option<SolverLiteral>>>);
    impl Propagator for Capture {
        fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
            let mut lits = Vec::new();
            for atom in &init.symbolic_atoms()? {
                lits.push(init.solver_literal(atom?.literal())?);
            }
            *self.0.lock().unwrap() = lits.into_iter().max();
            Ok(())
        }
    }

    // Two hundred independent choices give the propagator's own control a
    // solver literal far beyond anything a one- or two-atom control (the
    // ones used below) ever allocates.
    let mut ctl = Control::new().unwrap();
    ctl.add_base("{p(1..200)}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(Capture(Arc::clone(&biggest)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    biggest.lock().unwrap().expect("at least one atom exists")
}

// ---------------------------------------------------------------------------
// add_watch_to_thread watches only the given thread; add_watch (no thread
// argument) watches every active thread.
//
// The `propagate` trampoline dispatch and
// `PropagateControl::thread_id` are enough: a literal watched
// only on thread 0 must never appear in another thread's change set.
//
// This test keeps its own per-thread
// state as a plain `OnceLock<Vec<AtomicBool>>`, sized once in `init` from
// `number_of_threads()` -- the "own `Vec<Mutex<T>>` or similar" users are
// expected to reach for.

/// The program literal of `p(i)`.
fn p_literal(init: &PropagateInit<'_>, i: u32) -> Result<clingox::ProgramLiteral> {
    let want: clingox::Symbol = format!("p({i})").parse()?;
    let atom = init
        .symbolic_atoms()?
        .find(want)?
        .unwrap_or_else(|| panic!("p({i}) is an atom"));
    Ok(atom.literal())
}

/// Watches `p(i)` only on thread `i % threads`, and records any `propagate`
/// call that reports a watched literal on another thread.
struct PerThreadWatch {
    owner: OnceLock<Vec<(SolverLiteral, u32)>>,
    fired: AtomicUsize,
    foreign: AtomicUsize,
}

impl Propagator for PerThreadWatch {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let threads = init.number_of_threads();
        let mut owner = Vec::new();
        for i in 1..=8_u32 {
            let plit = p_literal(init, i)?;
            let slit = init.solver_literal(plit)?;
            let thread = i % threads;
            init.add_watch_to_thread(slit, thread)?;
            owner.push((slit, thread));
        }
        let _ = self.owner.set(owner);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        let owner = self.owner.get().expect("init ran first");
        for change in changes {
            if let Some(&(_, thread)) = owner.iter().find(|(lit, _)| lit == change) {
                self.fired.fetch_add(1, Ordering::SeqCst);
                if thread != control.thread_id() {
                    self.foreign.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        Ok(())
    }
}

/// Every watched literal is reported only on the thread it was registered
/// on. With `add_watch_to_thread` wired to `add_watch` (the negative
/// control), every thread watches every literal, and with four threads
/// enumerating all 256 models some thread reports a literal it does not own.
/// The sanity check needs only that some watch fired somewhere, which no
/// scheduling can prevent while all models are enumerated.
#[test]
fn add_watch_to_thread_restricts_the_watch_to_one_thread() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    struct Forwarding(Arc<PerThreadWatch>);
    impl Propagator for Forwarding {
        fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
            self.0.init(init)
        }
        fn propagate(
            &self,
            control: &mut PropagateControl<'_>,
            changes: &[SolverLiteral],
        ) -> Result<()> {
            self.0.propagate(control, changes)
        }
    }

    for _ in 0..5 {
        let shared = Arc::new(PerThreadWatch {
            owner: OnceLock::new(),
            fired: AtomicUsize::new(0),
            foreign: AtomicUsize::new(0),
        });
        let mut ctl = Control::builder()
            .args(["--models=0"])
            .threads(4)
            .build()
            .unwrap();
        ctl.add_base("{ p(1..8) }.").unwrap();
        ctl.ground(&[Part::base()]).unwrap();
        ctl.register_propagator(Forwarding(Arc::clone(&shared)))
            .unwrap();
        assert!(ctl.solve(&[]).unwrap().is_exhausted());

        assert!(
            shared.fired.load(Ordering::SeqCst) > 0,
            "sanity: some watch fired"
        );
        assert_eq!(
            shared.foreign.load(Ordering::SeqCst),
            0,
            "a literal was reported on a thread it was not watched on"
        );
    }
}

// ---------------------------------------------------------------------------
// freeze_literal
//
// Oracle: registering a propagator that maps a fact-derived atom's program
// literal to a solver literal and immediately uses it in `add_clause`,
// neither with nor without `freeze_literal` first, produced an error in this
// session's reproduction (the full
// transcript included a cross-step reuse attempt that also did not
// error). This test therefore only pins what was actually observed: the
// call does not error either way, and the program still solves correctly.
// The stronger claim (an unfrozen literal
// becomes unusable after preprocessing) is flagged as an open item, not
// asserted here without oracle evidence for it.

struct FreezeThenUse {
    freeze: bool,
}

impl Propagator for FreezeThenUse {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "aux")?;
        let slit = init.solver_literal(plit)?;
        if self.freeze {
            init.freeze_literal(slit)?;
        }
        let _ = init.add_clause(&[slit])?;
        Ok(())
    }
}

#[test]
fn freeze_literal_does_not_error_and_the_program_still_solves() {
    for freeze in [true, false] {
        let mut ctl = grounded("aux :- a. a.");
        ctl.register_propagator(FreezeThenUse { freeze }).unwrap();
        let result = ctl.solve(&[]).unwrap();
        assert!(result.is_sat(), "freeze = {freeze}");
    }
}

// ---------------------------------------------------------------------------
// symbolic_atoms()/theory_atoms() inside init return the same domain
// Control::symbolic_atoms()/theory_atoms() report after the search.
//
// Oracle: `sorted(a.symbol for a in init.symbolic_atoms)` inside `init`
// equalled `sorted(a.symbol for a in ctl.symbolic_atoms)` read after
// `solve()` returned, for `a. {b}. c :- a.`.

struct RecordsAtoms {
    symbolic: Mutex<Vec<String>>,
}

impl Propagator for RecordsAtoms {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let mut names: Vec<String> = init
            .symbolic_atoms()?
            .iter()
            .map(|atom| atom.map(|a| a.symbol().to_string()))
            .collect::<Result<_>>()?;
        names.sort();
        *self.symbolic.lock().unwrap() = names;
        Ok(())
    }
}

#[test]
fn symbolic_atoms_inside_init_matches_the_controls_own_view_after_solve() {
    let mut ctl = grounded("a. {b}. c :- a.");
    let recorder = RecordsAtoms {
        symbolic: Mutex::new(Vec::new()),
    };
    // We need to read the recorded names after `register_propagator` moves
    // `recorder` in, so share the inner `Mutex` through an `Arc` instead.
    let shared = Arc::new(recorder);
    struct Forwarding(Arc<RecordsAtoms>);
    impl Propagator for Forwarding {
        fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
            self.0.init(init)
        }
    }
    ctl.register_propagator(Forwarding(Arc::clone(&shared)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();

    let from_init = shared.symbolic.lock().unwrap().clone();
    let mut from_control: Vec<String> = ctl
        .symbolic_atoms()
        .unwrap()
        .iter()
        .map(|atom| atom.map(|a| a.symbol().to_string()))
        .collect::<Result<_>>()
        .unwrap();
    from_control.sort();
    assert_eq!(from_init, from_control);
}

// ---------------------------------------------------------------------------
// theory_atoms() inside init matches Control::theory_atoms()'s own view
// after the search, the same way symbolic_atoms() does above (the
// `#theory` fixture, `api_theory_atoms.rs`'s own `THEORY` constant and
// display convention).
//
// Oracle: pyclingo 5.8.2, the identical `#theory` block plus `&a { 1 }.`:
// exactly one theory atom, displayed `&a{1}`.

const THEORY: &str = "#theory t { term { + : 1, binary, left }; \
                       &a/0 : term, any; &b/0 : term, {=}, term, directive }.";

struct RecordsTheoryAtoms {
    displayed: Mutex<Vec<String>>,
}

impl Propagator for RecordsTheoryAtoms {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let mut displayed: Vec<String> = init
            .theory_atoms()?
            .iter()
            .map(|atom| atom.map(|a| a.to_string()))
            .collect::<Result<_>>()?;
        displayed.sort();
        *self.displayed.lock().unwrap() = displayed;
        Ok(())
    }
}

#[test]
fn theory_atoms_inside_init_match_the_oracle() {
    let program = format!("{THEORY}\n&a {{ 1 }}.");
    let mut ctl = grounded(&program);

    let shared = Arc::new(RecordsTheoryAtoms {
        displayed: Mutex::new(Vec::new()),
    });
    struct Forwarding(Arc<RecordsTheoryAtoms>);
    impl Propagator for Forwarding {
        fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
            self.0.init(init)
        }
    }
    ctl.register_propagator(Forwarding(Arc::clone(&shared)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();

    let from_init = shared.displayed.lock().unwrap().clone();
    assert_eq!(from_init, vec!["&a{1}".to_owned()], "matches the oracle");

    // The control's own view is not compared: clingo resets theory atoms
    // after a solve (see `TheoryAtoms`), so it is empty here.
}

// ---------------------------------------------------------------------------
// check_mode/undo_mode getters and setters round-trip for every value.

struct RoundTrips;
impl Propagator for RoundTrips {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        for mode in [
            CheckMode::Off,
            CheckMode::Total,
            CheckMode::Fixpoint,
            CheckMode::Both,
        ] {
            init.set_check_mode(mode);
            assert_eq!(init.check_mode(), mode);
        }
        for mode in [UndoMode::Default, UndoMode::Always] {
            init.set_undo_mode(mode);
            assert_eq!(init.undo_mode(), mode);
        }
        Ok(())
    }
}

#[test]
fn check_mode_and_undo_mode_round_trip_every_value() {
    let mut ctl = grounded("a.");
    ctl.register_propagator(RoundTrips).unwrap();
    let _ = ctl.solve(&[]).unwrap();
}

// ---------------------------------------------------------------------------
// assignment() inside init: decision level 0, no conflict, not total, but
// NOT necessarily empty (a fact's literal is already assigned).
//
// Oracle: `a. c :- a. {b}.`: `decision_level == 0`, `has_conflict == False`,
// `is_total == False`, `len(assignment) == 2` (the two literals of the two
// facts `a`/`c`, unit-propagated before any decision).

/// Decision level, root level, conflict, size and totality at `init`.
type AssignmentAtInit = (u32, u32, bool, usize, bool);

struct RecordsAssignment(Mutex<Option<AssignmentAtInit>>);
impl Propagator for RecordsAssignment {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let a = init.assignment();
        *self.0.lock().unwrap() = Some((
            a.decision_level(),
            a.root_level(),
            a.has_conflict(),
            a.size(),
            a.is_total(),
        ));
        Ok(())
    }
}

#[test]
fn assignment_at_init_is_level_zero_and_sized_by_variables() {
    let recorder = Arc::new(RecordsAssignment(Mutex::new(None)));
    struct Forwarding(Arc<RecordsAssignment>);
    impl Propagator for Forwarding {
        fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
            self.0.init(init)
        }
    }
    let mut ctl = grounded("a. c :- a. {b}.");
    ctl.register_propagator(Forwarding(Arc::clone(&recorder)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();

    let (decision_level, root_level, has_conflict, size, is_total) =
        recorder.0.lock().unwrap().expect("init ran");
    assert_eq!(decision_level, 0);
    assert_eq!(root_level, 0);
    assert!(!has_conflict);
    assert!(!is_total, "b is still undecided");
    assert_eq!(
        size, 2,
        "the variable space: the fixed true variable (a and c both map to it) and b's variable"
    );
}

// ---------------------------------------------------------------------------
// add_literal: a frozen literal is usable in add_clause within the same
// init; an unfrozen one (freeze: false) is too, when used immediately (no
// error was observed either way in a reproduction).

struct AddsLiteral {
    freeze: bool,
}
impl Propagator for AddsLiteral {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let lit = init.add_literal(self.freeze)?;
        let result = init.add_clause(&[lit])?;
        assert_eq!(result, Flow::Continue);
        Ok(())
    }
}

#[test]
fn add_literal_is_usable_in_add_clause_within_the_same_init_frozen_or_not() {
    for freeze in [true, false] {
        let mut ctl = grounded("a.");
        ctl.register_propagator(AddsLiteral { freeze }).unwrap();
        assert!(ctl.solve(&[]).unwrap().is_sat(), "freeze = {freeze}");
    }
}

// ---------------------------------------------------------------------------
// add_clause/propagate returning Flow::Stop, and the guard against further
// calls afterward.
//
// Oracle: a contradictory pair of unit clauses over a literal obtained from
// `add_literal` made the second `add_clause` return `False` (`Flow::Stop`);
// solving afterward reported UNSAT. A *further* call after that `Stop` did
// not error or crash in clingo itself (it was accepted silently and
// returned `Flow::Stop` again) -- clingox's own guard must therefore refuse
// it first, with `ErrorKind::InvalidInput` (the post-Stop guard).

struct ContradictoryPair;
impl Propagator for ContradictoryPair {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let lit = init.add_literal(true)?;
        let first = init.add_clause(&[lit])?;
        assert_eq!(first, Flow::Continue);
        let second = init.add_clause(&[-lit])?;
        assert_eq!(
            second,
            Flow::Stop,
            "a direct contradiction stops propagation"
        );
        Ok(())
    }
}

#[test]
fn add_clause_returning_stop_makes_the_program_unsatisfiable() {
    let mut ctl = grounded("a.");
    ctl.register_propagator(ContradictoryPair).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_unsat());
}

struct CallsAfterStop;
impl Propagator for CallsAfterStop {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let lit = init.add_literal(true)?;
        let _ = init.add_clause(&[lit])?;
        let stop = init.add_clause(&[-lit])?;
        assert_eq!(stop, Flow::Stop);
        let err = init.add_clause(&[lit]).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "clingox refuses a call after Stop before reaching clingo"
        );
        let err = init.propagate().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        Ok(())
    }
}

#[test]
fn a_further_call_after_stop_is_refused_by_clingox_itself() {
    let mut ctl = grounded("a.");
    ctl.register_propagator(CallsAfterStop).unwrap();
    let _ = ctl.solve(&[]).unwrap();
}

// ---------------------------------------------------------------------------
// A `SolverLiteral` legitimately obtained from a different control's
// grounding is out of range for this one; every `PropagateInit` method that
// takes a literal must validate it first (against `clingo_assignment_
// has_literal` on this init's own assignment) and reject an unknown one
// with `ErrorKind::InvalidInput`, rather than passing it to clingo, which
// segfaults (reproduced directly).
//
// A C-level probe (`clingo_assignment_has_literal` called directly through
// `_lib`/`_ffi` on huge, negative, zero, `i32::MIN` and `i32::MAX` values)
// confirmed the check itself is safe for any `i32`: it returns `False`
// cleanly in every out-of-range case, never crashes. The guard can
// therefore rely on it.

struct RejectsForeignLiteral(SolverLiteral);
impl Propagator for RejectsForeignLiteral {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let foreign = self.0;

        let err = init.add_watch(foreign).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "add_watch");

        let err = init.add_watch_to_thread(foreign, 0).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "add_watch_to_thread");

        let err = init.remove_watch(foreign).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "remove_watch");

        let err = init.remove_watch_from_thread(foreign, 0).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "remove_watch_from_thread"
        );

        let err = init.freeze_literal(foreign).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "freeze_literal");

        let err = init.add_clause(&[foreign]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "add_clause");

        // A valid clause whose *second* literal is the foreign one: the
        // guard must check every literal in the slice, not only the first.
        let own_program_literal = program_literal(init, "b")?;
        let own = init.solver_literal(own_program_literal)?;
        let err = init.add_clause(&[own, foreign]).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "add_clause, foreign literal not in first position"
        );

        // add_weight_constraint: the guard must check both the associated
        // literal and every weighted literal.
        let err = init
            .add_weight_constraint(
                foreign,
                &[(own, 1)],
                1,
                WeightConstraintKind::Equivalence,
                false,
            )
            .unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "add_weight_constraint, foreign associated literal"
        );
        let err = init
            .add_weight_constraint(
                own,
                &[(foreign, 1)],
                1,
                WeightConstraintKind::Equivalence,
                false,
            )
            .unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "add_weight_constraint, foreign weighted literal"
        );

        let err = init.add_minimize(foreign, 1, 0).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "add_minimize");

        Ok(())
    }
}

#[test]
fn every_literal_taking_propagate_init_method_rejects_a_foreign_literal() {
    let foreign = foreign_literal();
    let mut ctl = grounded("b.");
    ctl.register_propagator(RejectsForeignLiteral(foreign))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
}

// ---------------------------------------------------------------------------
// add_weight_constraint: the three WeightConstraintKind variants match the
// corrected entailment direction.
//
// Oracle (`{a;b;c}.`, constraint `{a=1,b=1} >= 1`, literal c):
// - ImplicationLeft (-1): models where the constraint holds but c is false
//   never appear; c can be true while the constraint does not hold.
// - ImplicationRight (1): models where c holds but the constraint does not
//   never appear; the constraint can hold while c is false.
// - Equivalence (0): both directions.

fn weight_constraint_models(kind: WeightConstraintKind) -> Vec<Vec<String>> {
    struct WC(WeightConstraintKind);
    impl Propagator for WC {
        fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
            let pa = program_literal(init, "a")?;
            let pb = program_literal(init, "b")?;
            let pc = program_literal(init, "c")?;
            let la = init.solver_literal(pa)?;
            let lb = init.solver_literal(pb)?;
            let lc = init.solver_literal(pc)?;
            let _ = init.add_weight_constraint(lc, &[(la, 1), (lb, 1)], 1, self.0, false)?;
            Ok(())
        }
    }

    // All models, as in the oracle's `Control(["0"])`: clingo's default of
    // one model would hide the ones the tests look for.
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b;c}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(WC(kind)).unwrap();
    let mut models = Vec::new();
    let _ = ctl
        .for_each_model(&[], |m| {
            let mut syms: Vec<String> = m
                .symbols(clingox::ShowType::SHOWN)?
                .iter()
                .map(ToString::to_string)
                .collect();
            syms.sort();
            models.push(syms);
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .unwrap();
    models.sort();
    models
}

#[test]
fn weight_constraint_implication_left_forces_the_literal_when_the_constraint_holds() {
    let models = weight_constraint_models(WeightConstraintKind::ImplicationLeft);
    // "a" alone and "b" alone (constraint true, c false) must not appear;
    // "c" alone (c true, constraint false) is allowed.
    assert!(!models.contains(&vec!["a".to_owned()]));
    assert!(!models.contains(&vec!["b".to_owned()]));
    assert!(models.contains(&vec!["c".to_owned()]));
}

#[test]
fn weight_constraint_implication_right_forces_the_constraint_when_the_literal_holds() {
    let models = weight_constraint_models(WeightConstraintKind::ImplicationRight);
    // "c" alone (c true, constraint false) must not appear; "a" alone and
    // "b" alone (constraint true, c false) are allowed.
    assert!(!models.contains(&vec!["c".to_owned()]));
    assert!(models.contains(&vec!["a".to_owned()]));
    assert!(models.contains(&vec!["b".to_owned()]));
}

#[test]
fn weight_constraint_equivalence_holds_in_both_directions() {
    let models = weight_constraint_models(WeightConstraintKind::Equivalence);
    assert!(!models.contains(&vec!["a".to_owned()]));
    assert!(!models.contains(&vec!["b".to_owned()]));
    assert!(!models.contains(&vec!["c".to_owned()]));
}

// ---------------------------------------------------------------------------
// add_minimize extends the solver's own minimize constraint.
//
// Oracle: `{a;b}.`, `add_minimize(a, 1, 0)` and `add_minimize(-b, 1, 0)`
// (equivalently, `add_minimize` on the literal that is true exactly when
// `b` is false) made the unique optimal model `{b}` (`a` false, `b` true).

struct Minimizes;
impl Propagator for Minimizes {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let pa = program_literal(init, "a")?;
        let pb = program_literal(init, "b")?;
        let la = init.solver_literal(pa)?;
        let lb = init.solver_literal(pb)?;
        init.add_minimize(la, 1, 0)?;
        init.add_minimize(-lb, 1, 0)?;
        Ok(())
    }
}

#[test]
fn add_minimize_extends_the_minimize_constraint() {
    let mut ctl = grounded("{a;b}.");
    ctl.register_propagator(Minimizes).unwrap();
    let outcome = ctl.solve_optimal().unwrap();
    let clingox::Outcome::Sat(model, _) = outcome else {
        panic!("satisfiable: {outcome:?}");
    };
    let mut syms: Vec<String> = model.symbols().iter().map(ToString::to_string).collect();
    syms.sort();
    assert_eq!(syms, vec!["b".to_owned()]);
}

// ---------------------------------------------------------------------------
// propagate() inside init propagates clauses added so far, visible through
// assignment() before solving proper starts.
//
// Oracle: `{a;b}.` with `add_clause([la])` and `add_clause([-la, lb])`
// followed by `init.propagate()`: `assignment.is_true(la)` was already
// `True` inside `init`, and the unique model was `{a, b}`.

struct PropagatesEarly;
impl Propagator for PropagatesEarly {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let pa = program_literal(init, "a")?;
        let pb = program_literal(init, "b")?;
        let la = init.solver_literal(pa)?;
        let lb = init.solver_literal(pb)?;
        assert_eq!(init.add_clause(&[la])?, Flow::Continue);
        assert_eq!(init.add_clause(&[-la, lb])?, Flow::Continue);
        assert_eq!(init.propagate()?, Flow::Continue);
        Ok(())
    }
}

#[test]
fn propagate_in_init_makes_added_clauses_effective_before_solving() {
    let mut ctl = grounded("{a;b}.");
    ctl.register_propagator(PropagatesEarly).unwrap();
    let mut models = Vec::new();
    let _ = ctl
        .for_each_model(&[], |m| {
            let mut syms: Vec<String> = m
                .symbols(clingox::ShowType::SHOWN)?
                .iter()
                .map(ToString::to_string)
                .collect();
            syms.sort();
            models.push(syms);
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(models, vec![vec!["a".to_owned(), "b".to_owned()]]);
}

// ---------------------------------------------------------------------------
// A panic inside init is caught, resumes on the caller's thread, and poisons
// the control (init follows the ground-callback precedent).

struct PanicsInInit;
impl Propagator for PanicsInInit {
    fn init(&self, _init: &mut PropagateInit<'_>) -> Result<()> {
        panic!("stop");
    }
}

#[test]
fn a_panic_in_init_is_caught_resumes_and_poisons() {
    let mut ctl = grounded("a.");
    ctl.register_propagator(PanicsInInit).unwrap();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(payload.downcast_ref::<&str>().copied(), Some("stop"));

    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
    assert!(format!("{ctl:?}").contains("poisoned"));
}

// ---------------------------------------------------------------------------
// A panic inside undo is caught and resumed on the caller's thread after the
// solve, never unwinding into clingo's C++ frames (`undo` stays infallible,
// returning `()`, exactly like the logger and script `free`, DESIGN S9; a panic
// there is still caught by the trampoline per S8, which applies "regardless of
// the C signature's return type").
//
// `undo`'s own trampoline dispatch is wired, and this test needs no
// `PropagateControl` method at all (its `undo` implementation ignores `control`
// entirely). Forcing `solve.models = 0` guarantees clasp actually backtracks
// past the watched literal at least once (a single-model solve of `1 { a; b }
// 1.` can succeed on the first branch with no backtrack at all, which would
// never call `undo`).

struct PanicsInUndo {
    lit: OnceLock<SolverLiteral>,
}

impl Propagator for PanicsInUndo {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        let _ = self.lit.set(slit);
        Ok(())
    }

    fn undo(&self, _control: &PropagateControl<'_>, _changes: &[SolverLiteral]) {
        panic!("undo panics on purpose");
    }
}

#[test]
fn a_panic_in_undo_is_caught_and_resumes_after_the_solve() {
    let mut ctl = grounded("1 { a; b } 1.");
    ctl.configuration().set("solve.models", "0").unwrap();
    ctl.register_propagator(PanicsInUndo {
        lit: OnceLock::new(),
    })
    .unwrap();

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctl.for_each_model(&[], |_| Ok(std::ops::ControlFlow::Continue(())))
    }));
    let payload = caught.expect_err("the panic reaches the caller, not a solver thread");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("undo panics on purpose")
    );
}

// ---------------------------------------------------------------------------
// An error from init is returned, keeps its own kind, and poisons.

struct ErrorsInInit(ErrorKind);
impl Propagator for ErrorsInInit {
    fn init(&self, _init: &mut PropagateInit<'_>) -> Result<()> {
        Err(clingox::Error::new(self.0, "no"))
    }
}

#[test]
fn an_error_from_init_keeps_its_own_kind_and_poisons() {
    let mut ctl = grounded("a.");
    ctl.register_propagator(ErrorsInInit(ErrorKind::Conversion))
        .unwrap();
    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert!(format!("{ctl:?}").contains("poisoned"));
    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

// ---------------------------------------------------------------------------
// propagate/check: Err is returned by the solve that ran it, with its own
// kind and source; a panic resumes on the caller's thread, not a solver
// thread, once the solve returns; neither poisons the control.
//
// Oracle, both propagate and check, checked directly against clingo 5.8.2:
// an unconditionally-raising propagator raises again on every later solve
// too, which only shows the *same* propagator keeps firing, not whether a
// failure poisons the control -- so the oracle probe (and the tests below)
// use a propagator that fails on its first call only. A further solve,
// where the callback does *not* raise, then completes normally (`SAT`),
// confirming propagate/undo/check/decide follow the measured
// behaviour and do not poison, unlike `init`.
//
// `1 { a; b } 1.` with a watch on `a`'s literal fires `propagate` exactly
// once per `solve()` call (checked directly: 3 calls across 3 solves, one
// each); a plain fact `a.` with `CheckMode::Total` fires `check` at least
// once per `solve()` call in the same way.

#[derive(Debug)]
struct Boom;
impl std::fmt::Display for Boom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("boom")
    }
}
impl std::error::Error for Boom {}

struct ErrorsInPropagate;
impl Propagator for ErrorsInPropagate {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        _control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        Err(clingox::Error::callback(Boom))
    }
}

#[test]
fn an_error_from_propagate_is_returned_with_its_own_kind_and_source() {
    let mut ctl = grounded("1 { a; b } 1.");
    ctl.register_propagator(ErrorsInPropagate).unwrap();
    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
    let source = std::error::Error::source(&err).expect("the user error is the source");
    assert!(source.downcast_ref::<Boom>().is_some());
}

struct PanicsInPropagate {
    lit: OnceLock<SolverLiteral>,
}
impl Propagator for PanicsInPropagate {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        let _ = self.lit.set(slit);
        Ok(())
    }

    fn propagate(
        &self,
        _control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        panic!("propagate panics on purpose");
    }
}

#[test]
fn a_panic_in_propagate_is_caught_and_resumes_on_the_caller_after_the_solve() {
    let mut ctl = grounded("1 { a; b } 1.");
    ctl.register_propagator(PanicsInPropagate {
        lit: OnceLock::new(),
    })
    .unwrap();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller, not a solver thread");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("propagate panics on purpose")
    );
}

/// Fails only on its first call, so a *later* solve can show whether the
/// earlier failure poisoned the control.
struct FailsPropagateOnce {
    calls: Arc<Mutex<u32>>,
}
impl Propagator for FailsPropagateOnce {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        _control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            Err(clingox::Error::callback(Boom))
        } else {
            Ok(())
        }
    }
}

#[test]
fn a_failed_propagate_does_not_poison_the_control() {
    let calls = Arc::new(Mutex::new(0));
    let mut ctl = grounded("1 { a; b } 1.");
    ctl.register_propagator(FailsPropagateOnce {
        calls: Arc::clone(&calls),
    })
    .unwrap();

    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);

    // A later solve, where propagate no longer fails, completes normally:
    // the earlier failure did not poison the control.
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());
    assert_eq!(*calls.lock().unwrap(), 2);
}

struct ErrorsInCheck;
impl Propagator for ErrorsInCheck {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Total);
        Ok(())
    }

    fn check(&self, _control: &mut PropagateControl<'_>) -> Result<()> {
        Err(clingox::Error::callback(Boom))
    }
}

#[test]
fn an_error_from_check_is_returned_with_its_own_kind_and_source() {
    let mut ctl = grounded("a.");
    ctl.register_propagator(ErrorsInCheck).unwrap();
    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
    let source = std::error::Error::source(&err).expect("the user error is the source");
    assert!(source.downcast_ref::<Boom>().is_some());
}

struct PanicsInCheck;
impl Propagator for PanicsInCheck {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Total);
        Ok(())
    }

    fn check(&self, _control: &mut PropagateControl<'_>) -> Result<()> {
        panic!("check panics on purpose");
    }
}

#[test]
fn a_panic_in_check_is_caught_and_resumes_on_the_caller_after_the_solve() {
    let mut ctl = grounded("a.");
    ctl.register_propagator(PanicsInCheck).unwrap();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller, not a solver thread");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("check panics on purpose")
    );
}

/// Fails only on its first call, so a *later* solve can show whether the
/// earlier failure poisoned the control.
struct FailsCheckOnce {
    calls: Arc<Mutex<u32>>,
}
impl Propagator for FailsCheckOnce {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Total);
        Ok(())
    }

    fn check(&self, _control: &mut PropagateControl<'_>) -> Result<()> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            Err(clingox::Error::callback(Boom))
        } else {
            Ok(())
        }
    }
}

#[test]
fn a_failed_check_does_not_poison_the_control() {
    let calls = Arc::new(Mutex::new(0));
    let mut ctl = grounded("a.");
    ctl.register_propagator(FailsCheckOnce {
        calls: Arc::clone(&calls),
    })
    .unwrap();

    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);

    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());
    assert_eq!(*calls.lock().unwrap(), 2);
}

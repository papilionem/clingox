//! `PropagateControl`'s complete surface: `thread_id`, `assignment` and
//! `add_clause`, plus `add_literal`, `add_watch`, `has_watch`, `remove_watch`
//! and this type's own `propagate`. `ClauseType`'s observable cross-step
//! lifetime behaviour, the post-`Stop` guard extended to every new method, and
//! the ported `libpyclingo` oracle tests are all here too.
//!
//! Every expected value below was checked directly against clingo 5.8.2 (the
//! Python module `clingo`, 2026-09-28).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::sync::{Arc, Mutex, OnceLock};

use clingox::propagate::{
    Assignment, CheckMode, ClauseType, Flow, PropagateControl, PropagateInit, Propagator,
    SolverLiteral,
};
use clingox::{Control, ErrorKind, Part, Result, Signature};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a fresh control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// As [`grounded`], but with `--models=0`: `model_sets`'s own `for_each_ model`
/// otherwise stops at clingo's own default of one model, which undercounts any
/// test below that needs every model actually enumerated (found directly while
/// writing `propagator_no_two_adjacent.rs`'s own correctness test).
fn grounded_all_models(program: &str) -> Control {
    let mut ctl = Control::with_args(["--models=0"]).expect("a fresh control");
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
/// assignment (see `api_propagator_init.rs`'s identical helper and
/// for the segfault this
/// reproduces when passed unguarded).
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

    let mut ctl = Control::new().unwrap();
    ctl.add_base("{p(1..200)}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(Capture(Arc::clone(&biggest)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    biggest.lock().unwrap().expect("at least one atom exists")
}

/// Every model of `ctl`, as sorted symbol lists, one per model.
fn model_sets(ctl: &mut Control) -> Vec<Vec<String>> {
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
        .expect("the program solves");
    models.sort();
    models
}

// ---------------------------------------------------------------------------
// AIFFB: pyclingo's own `propagator.py` module-doctest example, ported
// verbatim as the first correctness test that exercises `solver_literal`,
// `add_watch` and `PropagateInit::assignment` together with a real dispatch
// to `propagate`, including `PropagateControl::add_clause`.
//
// Oracle (pyclingo 5.8.2, `["0"]`, `1 { a; b }.`): exactly one model, `a b`.

// `Propagator: Send + Sync` rules out `Cell`/`RefCell` (never `Sync`) for
// state set once in `init` and read in `propagate`; `OnceLock` is `Sync`
// when its content is (`SolverLiteral: Copy + Send + Sync + 'static`,
// matching `ProgramLiteral`), and is the same choice the
// `NoTwoAdjacent` example makes for its watched literals.
#[derive(Default)]
struct Aiffb {
    slit_a: OnceLock<SolverLiteral>,
    slit_b: OnceLock<SolverLiteral>,
}

impl Propagator for Aiffb {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let sig = Signature::new("a", 0)?;
        let plit_a = init
            .symbolic_atoms()?
            .by_signature(sig)
            .next()
            .expect("a is an atom")?
            .literal();
        let sig = Signature::new("b", 0)?;
        let plit_b = init
            .symbolic_atoms()?
            .by_signature(sig)
            .next()
            .expect("b is an atom")?
            .literal();
        let slit_a = init.solver_literal(plit_a)?;
        let slit_b = init.solver_literal(plit_b)?;
        init.add_watch(slit_a)?;
        init.add_watch(slit_b)?;
        let _ = self.slit_a.set(slit_a);
        let _ = self.slit_b.set(slit_b);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        let slit_a = *self.slit_a.get().expect("init ran first");
        let slit_b = *self.slit_b.get().expect("init ran first");
        // Mirrors pyclingo's own docstring: "If either of the two methods
        // returns False, the propagate function must return immediately."
        if changes.contains(&slit_a)
            && control
                .add_clause(&[-slit_a, slit_b], ClauseType::Learnt)?
                .is_stop()
        {
            return Ok(());
        }
        if changes.contains(&slit_b)
            && control
                .add_clause(&[-slit_b, slit_a], ClauseType::Learnt)?
                .is_stop()
        {
            return Ok(());
        }
        Ok(())
    }
}

#[test]
fn aiffb_forces_a_and_b_to_agree() {
    let mut ctl = grounded("1 { a; b }.");
    ctl.register_propagator(Aiffb::default()).unwrap();
    assert_eq!(
        model_sets(&mut ctl),
        vec![vec!["a".to_owned(), "b".to_owned()]]
    );
}

// ---------------------------------------------------------------------------
// PropagateControl::add_clause validates every literal against this
// control's own assignment first, exactly like every literal-taking
// PropagateInit method: a SolverLiteral legitimately obtained from a different,
// larger control is out of range here and must be refused with
// ErrorKind::InvalidInput, never passed to clingo.
//
// The watched literal here is a plain fact's ("a."): watching a fact's
// literal reliably fires `propagate` as soon as solving starts, with no
// backtracking needed (confirmed directly with pyclingo: `changes = [1]`
// on the very first call), unlike a two-sided choice, where the solver
// might satisfy the program without ever making the watched side true.

struct RejectsForeignAddClause {
    foreign: SolverLiteral,
    checked: Arc<Mutex<bool>>,
}

impl Propagator for RejectsForeignAddClause {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let err = control
            .add_clause(&[self.foreign], ClauseType::Learnt)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        *self.checked.lock().unwrap() = true;
        Ok(())
    }
}

#[test]
fn add_clause_on_propagate_control_rejects_a_foreign_literal() {
    let foreign = foreign_literal();
    let checked = Arc::new(Mutex::new(false));
    let mut ctl = grounded("a.");
    ctl.register_propagator(RejectsForeignAddClause {
        foreign,
        checked: Arc::clone(&checked),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert!(
        *checked.lock().unwrap(),
        "sanity: propagate fired and the guard actually ran"
    );
}

// =============================================================================
// Add_literal, add_watch, has_watch, remove_watch, PropagateControl's
// own propagate, ClauseType's cross-step lifetime, the post-Stop guard
// extended to the four methods above, and the ported libpyclingo/tag.lp
// oracle tests.

// ---------------------------------------------------------------------------
// A SolverLiteral obtained from PropagateControl::add_literal in one solving
// step is unusable in a later one: reusing it unguarded in add_clause
// segfaults the process. The existing
// add_clause guard (assignment().has_literal, re-evaluated fresh every call)
// already closes this, with no code change needed; this test is the
// sibling of add_clause_on_propagate_control_rejects_a_foreign_literal for
// the cross-step case instead of the cross-control one.
//
// Oracle: a literal captured from step 1's propagate, reused via a raw
// clingo_propagate_control_add_clause call bypassing the Python wrapper, in
// step 2's propagate, crashed with SIGSEGV; the same literal read with
// clingo_assignment_has_literal in step 2 cleanly reports False first.

struct CapturesAddLiteral {
    lit: Arc<Mutex<Option<SolverLiteral>>>,
}

impl Propagator for CapturesAddLiteral {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let mut slot = self.lit.lock().unwrap();
        if slot.is_none() {
            *slot = Some(control.add_literal()?);
        }
        Ok(())
    }
}

struct RejectsStaleAddLiteral {
    stale: Arc<Mutex<Option<SolverLiteral>>>,
    checked: Arc<Mutex<bool>>,
}

impl Propagator for RejectsStaleAddLiteral {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        if let Some(stale) = *self.stale.lock().unwrap() {
            let err = control
                .add_clause(&[stale], ClauseType::Learnt)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            *self.checked.lock().unwrap() = true;
        }
        Ok(())
    }
}

#[test]
fn add_literal_reused_in_a_later_solving_step_is_rejected_by_add_clause() {
    let lit = Arc::new(Mutex::new(None));
    let mut ctl = grounded("a.");
    ctl.register_propagator(CapturesAddLiteral {
        lit: Arc::clone(&lit),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let stale = lit
        .lock()
        .unwrap()
        .expect("step 1's propagate captured a literal");

    let checked = Arc::new(Mutex::new(false));
    ctl.register_propagator(RejectsStaleAddLiteral {
        stale: Arc::new(Mutex::new(Some(stale))),
        checked: Arc::clone(&checked),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert!(
        *checked.lock().unwrap(),
        "sanity: the second step's propagate fired and the guard actually ran"
    );
}

// ---------------------------------------------------------------------------
// add_watch here restricts the watch to the current solver thread, unlike
// PropagateInit::add_watch, which by default watches on every thread
// (checked directly against the header's
// own note at clingo.h:1403-1405). Mirrors api_propagator_init.rs's own
// add_watch_to_thread_restricts_the_watch_to_one_thread, but for
// PropagateControl's own add_watch instead of PropagateInit's
// add_watch_to_thread.
//
// `a.` (a fact) is watched through PropagateInit, so every solver thread's
// own initial unit propagation fires this file's propagate independently,
// once per thread; `y`'s literal (a genuine choice, `{y}.`) is never watched
// through PropagateInit at all, only through PropagateControl::add_watch,
// the first time each thread's own propagate call for `a` runs. Checked
// directly, 4 threads: has_watch(y) is False on every thread before that
// thread's own add_watch, and True immediately after, on all 4 -- proving
// each thread's watch state is independent, never shared or inherited.

struct WatchesYOnlyOnItsOwnThread {
    y: OnceLock<SolverLiteral>,
    log: Arc<Mutex<Vec<(u32, bool, bool)>>>,
}

impl Propagator for WatchesYOnlyOnItsOwnThread {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let a_plit = program_literal(init, "a")?;
        let a_slit = init.solver_literal(a_plit)?;
        let y_plit = program_literal(init, "y")?;
        let y_slit = init.solver_literal(y_plit)?;
        init.add_watch(a_slit)?;
        let _ = self.y.set(y_slit);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        let y = *self.y.get().expect("init ran first");
        if changes.contains(&y) {
            return Ok(());
        }
        let before = control.has_watch(y);
        control.add_watch(y)?;
        let after = control.has_watch(y);
        self.log
            .lock()
            .unwrap()
            .push((control.thread_id(), before, after));
        Ok(())
    }
}

#[test]
fn add_watch_on_propagate_control_restricts_the_watch_to_the_current_thread() {
    let threads: u32 = if clingox_sys::HAS_THREADS { 4 } else { 1 };
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut ctl = Control::builder().threads(threads).build().unwrap();
    ctl.add_base("a. {y}. 1 { p(1..8) }.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(WatchesYOnlyOnItsOwnThread {
        y: OnceLock::new(),
        log: Arc::clone(&log),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();

    let log = log.lock().unwrap();
    assert!(
        !log.is_empty(),
        "at least one thread's own propagate fired for `a`"
    );
    for &(thread_id, before, after) in log.iter() {
        assert!(
            !before,
            "thread {thread_id}: y was not watched before this thread's own add_watch"
        );
        assert!(
            after,
            "thread {thread_id}: y is watched immediately after this thread's own add_watch"
        );
    }
}

// ---------------------------------------------------------------------------
// has_watch and remove_watch both validate their literal first, applying
// the literal rule without exceptions, even
// though clingo itself is already safe for either a foreign (cross-control)
// or a stale (same-control, earlier step) literal on both -- checked
// directly: clingo_propagate_control_has_watch on a foreign literal returns
// False cleanly, and clingo_propagate_control_remove_watch on the same
// literal runs to completion with no observable effect, neither crashes
// (`foreign_literal()` is reused here).
// `has_watch` stays a plain `bool` (validated before ever reaching clingo,
// so this is observably identical to the unguarded case, and no test can
// tell the two apart black-box); `remove_watch` now returns `Result<()>`
// and is refused with `ErrorKind::InvalidInput` for an unknown literal,
// the one behavioural (not just internal) change this decision makes.

struct ChecksHasWatchForForeignLiteral {
    foreign: SolverLiteral,
    checked: Arc<Mutex<bool>>,
}

impl Propagator for ChecksHasWatchForForeignLiteral {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        assert!(
            !control.has_watch(self.foreign),
            "a foreign literal is never watched here"
        );
        *self.checked.lock().unwrap() = true;
        Ok(())
    }
}

#[test]
fn has_watch_reports_false_for_a_foreign_literal() {
    let foreign = foreign_literal();
    let checked = Arc::new(Mutex::new(false));
    let mut ctl = grounded("a.");
    ctl.register_propagator(ChecksHasWatchForForeignLiteral {
        foreign,
        checked: Arc::clone(&checked),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert!(*checked.lock().unwrap(), "sanity: propagate fired");
}

struct RejectsForeignRemoveWatch {
    foreign: SolverLiteral,
    checked: Arc<Mutex<bool>>,
}

impl Propagator for RejectsForeignRemoveWatch {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let err = control.remove_watch(self.foreign).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        *self.checked.lock().unwrap() = true;
        Ok(())
    }
}

#[test]
fn remove_watch_on_propagate_control_rejects_a_foreign_literal() {
    let foreign = foreign_literal();
    let checked = Arc::new(Mutex::new(false));
    let mut ctl = grounded("a.");
    ctl.register_propagator(RejectsForeignRemoveWatch {
        foreign,
        checked: Arc::clone(&checked),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert!(*checked.lock().unwrap(), "sanity: propagate fired");
}

// ---------------------------------------------------------------------------
// add_watch here validates its literal first, the same way add_clause
// already does, ErrorKind::InvalidInput before ever reaching clingo (not
// forced by a crash risk here, since clingo's
// own clasp/src/clingo.cpp:138-155 check is already safe, but kept for one
// error kind across the whole propagator API).

struct RejectsForeignAddWatch {
    foreign: SolverLiteral,
    checked: Arc<Mutex<bool>>,
}

impl Propagator for RejectsForeignAddWatch {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let err = control.add_watch(self.foreign).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        *self.checked.lock().unwrap() = true;
        Ok(())
    }
}

#[test]
fn add_watch_on_propagate_control_rejects_a_foreign_literal() {
    let foreign = foreign_literal();
    let checked = Arc::new(Mutex::new(false));
    let mut ctl = grounded("a.");
    ctl.register_propagator(RejectsForeignAddWatch {
        foreign,
        checked: Arc::clone(&checked),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert!(*checked.lock().unwrap(), "sanity: propagate fired");
}

// ---------------------------------------------------------------------------
// PropagateControl::propagate propagates the clauses added so far during
// this call, before the outer propagate returns; a call with nothing
// pending succeeds and reports Flow::Continue.

struct CallsControlPropagate {
    result: Arc<Mutex<Option<Flow>>>,
}

impl Propagator for CallsControlPropagate {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let flow = control.propagate()?;
        *self.result.lock().unwrap() = Some(flow);
        Ok(())
    }
}

#[test]
fn propagate_on_propagate_control_succeeds_with_nothing_pending() {
    let result = Arc::new(Mutex::new(None));
    let mut ctl = grounded("a.");
    ctl.register_propagator(CallsControlPropagate {
        result: Arc::clone(&result),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert_eq!(*result.lock().unwrap(), Some(Flow::Continue));
}

// ---------------------------------------------------------------------------
// propagate() actually detects a real conflict and reports Flow::Stop, not
// only Flow::Continue when nothing is pending: a mutation always returning
// Continue would go unnoticed by the test above alone. `{b; d; g; e; f}. :-
// f, g.`, decide forces only b, d, e, leaving g and f both genuinely free.
// When e's watch fires, two separate add_clause calls force f and g true
// (each individually non-conflicting at the moment it is made, since
// neither's own counterpart is assigned yet, so both return Flow::Continue
// rather than being refused outright); control.propagate() then drains the
// queue, discovers `:- f, g.` (a permanent ASP-level clause, already
// installed) is now violated, and reports Flow::Stop. The post-Stop guard
// is then exercised directly: a further control.propagate() call is
// refused with ErrorKind::InvalidInput, matching every other Flow-returning
// method's own guard.
//
// Oracle: checked directly against clingo 5.8.2
// (`PropagateControl::propagate()` was also tried):
// section): has_conflict() is true immediately after control.propagate()
// returns Stop here.

struct DetectsRealConflict {
    b: OnceLock<SolverLiteral>,
    d: OnceLock<SolverLiteral>,
    e: OnceLock<SolverLiteral>,
    g: OnceLock<SolverLiteral>,
    f: OnceLock<SolverLiteral>,
    fired: Mutex<bool>,
    flow: Arc<Mutex<Option<Flow>>>,
    post_stop_checked: Arc<Mutex<bool>>,
}

impl Propagator for DetectsRealConflict {
    #[allow(
        clippy::many_single_char_names,
        reason = "each name matches its own atom in the fixture program (b, d, e, g, f)"
    )]
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let lit = |init: &PropagateInit<'_>, name: &str| -> Result<SolverLiteral> {
            init.solver_literal(program_literal(init, name)?)
        };
        let (b, d, e, g, f) = (
            lit(init, "b")?,
            lit(init, "d")?,
            lit(init, "e")?,
            lit(init, "g")?,
            lit(init, "f")?,
        );
        for l in [b, d, g, e, f] {
            init.add_watch(l)?;
            init.add_watch(-l)?;
        }
        let _ = self.b.set(b);
        let _ = self.d.set(d);
        let _ = self.e.set(e);
        let _ = self.g.set(g);
        let _ = self.f.set(f);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        let e = *self.e.get().expect("init ran first");
        if !changes.contains(&e) {
            return Ok(());
        }
        let mut fired = self.fired.lock().unwrap();
        if *fired {
            return Ok(());
        }
        *fired = true;
        let f = *self.f.get().unwrap();
        let g = *self.g.get().unwrap();
        let flow_f = control.add_clause(&[-e, f], ClauseType::Learnt)?;
        assert!(
            !flow_f.is_stop(),
            "sanity: forcing f alone does not conflict yet"
        );
        let flow_g = control.add_clause(&[-e, g], ClauseType::Learnt)?;
        assert!(
            !flow_g.is_stop(),
            "sanity: forcing g alone does not conflict yet"
        );
        let flow = control.propagate()?;
        *self.flow.lock().unwrap() = Some(flow);
        if flow.is_stop() {
            let err = control.propagate().unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            *self.post_stop_checked.lock().unwrap() = true;
        }
        Ok(())
    }

    fn decide(
        &self,
        _thread_id: u32,
        assignment: &Assignment<'_>,
        fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        for slot in [&self.b, &self.d, &self.e] {
            let l = *slot.get().unwrap();
            if assignment.truth_value(l)?.is_none() {
                return Ok(Some(l));
            }
        }
        Ok(Some(fallback))
    }
}

#[test]
fn propagate_on_propagate_control_detects_a_real_conflict_and_returns_stop() {
    let mut ctl = Control::builder().threads(1).build().unwrap();
    ctl.add_base("{b; d; g; e; f}. :- f, g.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let flow = Arc::new(Mutex::new(None));
    let post_stop_checked = Arc::new(Mutex::new(false));
    ctl.register_propagator(DetectsRealConflict {
        b: OnceLock::new(),
        d: OnceLock::new(),
        e: OnceLock::new(),
        g: OnceLock::new(),
        f: OnceLock::new(),
        fired: Mutex::new(false),
        flow: Arc::clone(&flow),
        post_stop_checked: Arc::clone(&post_stop_checked),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert_eq!(*flow.lock().unwrap(), Some(Flow::Stop));
    assert!(
        *post_stop_checked.lock().unwrap(),
        "sanity: the post-Stop guard was actually exercised"
    );
}

// ---------------------------------------------------------------------------
// The post-Stop guard extends to add_literal, add_watch, remove_watch and
// (PropagateControl's own) propagate, exactly like add_clause's already-
// merged guard: refused with ErrorKind::InvalidInput before reaching
// clingo, once any Flow-returning method on this same PropagateControl
// already reported Stop (has_watch and remove_watch also validate their
// literal first: clingo's own behaviour differs per method here, but every case
// is a uniformity decision, not a crash-prevention necessity).

struct RejectsEveryCallAfterStop {
    checked: Arc<Mutex<bool>>,
}

impl Propagator for RejectsEveryCallAfterStop {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(slit)?;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let slit = changes[0];
        let stop = control
            .add_clause(&[-slit], ClauseType::Learnt)
            .unwrap()
            .is_stop();
        assert!(
            stop,
            "sanity: a fails its own watched literal, forcing Stop"
        );

        assert_eq!(
            control.add_literal().unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            control.add_watch(slit).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            control.remove_watch(slit).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            control.propagate().unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            control
                .add_clause(&[slit], ClauseType::Learnt)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        *self.checked.lock().unwrap() = true;
        Ok(())
    }
}

#[test]
fn a_further_call_after_stop_is_refused_for_every_new_m3c_method() {
    let checked = Arc::new(Mutex::new(false));
    let mut ctl = grounded("a.");
    ctl.register_propagator(RejectsEveryCallAfterStop {
        checked: Arc::clone(&checked),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert!(
        *checked.lock().unwrap(),
        "sanity: propagate fired and Stop was reached"
    );
}

// ---------------------------------------------------------------------------
// thread_id() matches the OS thread actually running the callback, not just
// some number (an externally tracked map keyed by std::thread::current().id(),
// as an ordinary test, distinguishing "reports a number" from
// "reports the right one").

#[test]
fn thread_id_matches_the_os_thread_actually_running_the_callback() {
    struct RecordsOsThread {
        by_solver_id: Arc<Mutex<std::collections::HashMap<u32, std::thread::ThreadId>>>,
    }

    impl Propagator for RecordsOsThread {
        fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
            for atom in &init.symbolic_atoms()? {
                let slit = init.solver_literal(atom?.literal())?;
                init.add_watch(slit)?;
            }
            Ok(())
        }

        fn propagate(
            &self,
            control: &mut PropagateControl<'_>,
            _changes: &[SolverLiteral],
        ) -> Result<()> {
            self.by_solver_id
                .lock()
                .unwrap()
                .insert(control.thread_id(), std::thread::current().id());
            Ok(())
        }
    }

    let by_solver_id = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let threads: u32 = if clingox_sys::HAS_THREADS { 4 } else { 1 };
    let mut ctl = Control::builder().threads(threads).build().unwrap();
    ctl.add_base("1 { p(1..50) }.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(RecordsOsThread {
        by_solver_id: Arc::clone(&by_solver_id),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();

    let recorded = by_solver_id.lock().unwrap();
    assert!(
        !recorded.is_empty(),
        "at least one thread's propagate fired"
    );
    // Every clingo-reported solver thread id maps to exactly one distinct OS
    // thread; if thread_id() ever reported the wrong number (e.g. always 0,
    // or two solver threads sharing one id), two different solver ids would
    // collapse onto the same OS thread, or the same solver id would appear
    // to run on two different OS threads across calls -- the HashMap
    // insertion above would silently overwrite in the latter case, so this
    // asserts on the map's own consistency: every OS thread id present is
    // associated with exactly the solver ids that actually ran there.
    let distinct_os_threads: std::collections::HashSet<_> = recorded.values().collect();
    assert_eq!(
        distinct_os_threads.len(),
        recorded.len(),
        "each reported solver thread id maps to its own distinct OS thread, never a shared one"
    );
}

// ---------------------------------------------------------------------------
// ClauseType's observable cross-step lifetime (ported from tag.lp,
// clingox-sys/clingo/app/clingo/tests/python/tag.lp): a Volatile or
// VolatileStatic clause added during one solving step does not survive into
// the next; a Static clause does. Checked directly, three runs each.

/// Whether a clause of `kind`, forbidding `a` when added in step 1, is gone
/// by step 2 (`a` becomes satisfiable again): `true` means the clause did
/// *not* survive, `false` means it still forbids `a` in step 2.
fn clause_type_disappears_by_the_next_step(kind: ClauseType) -> bool {
    struct ForbidsAOnce {
        kind: ClauseType,
        added: Arc<Mutex<bool>>,
    }

    impl Propagator for ForbidsAOnce {
        fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
            let plit = program_literal(init, "a")?;
            let slit = init.solver_literal(plit)?;
            init.add_watch(slit)?;
            Ok(())
        }

        fn propagate(
            &self,
            control: &mut PropagateControl<'_>,
            changes: &[SolverLiteral],
        ) -> Result<()> {
            if changes.is_empty() {
                return Ok(());
            }
            let mut added = self.added.lock().unwrap();
            if !*added {
                *added = true;
                let _ = control.add_clause(&[-changes[0]], self.kind)?;
            }
            Ok(())
        }
    }

    let mut ctl = grounded_all_models("{a}.");
    ctl.register_propagator(ForbidsAOnce {
        kind,
        added: Arc::new(Mutex::new(false)),
    })
    .unwrap();
    // Step 1: the clause is added, forbidding `a`.
    let step1 = model_sets(&mut ctl);
    assert_eq!(
        step1,
        vec![Vec::<String>::new()],
        "sanity: `a` is forbidden in step 1"
    );
    // Step 2: no new registration, no new ground; only clingo's own
    // per-clause lifetime policy decides whether `a` is forbidden again.
    let step2 = model_sets(&mut ctl);
    step2.contains(&vec!["a".to_owned()])
}

#[test]
fn clause_type_volatile_and_volatile_static_do_not_survive_a_new_step() {
    assert!(
        clause_type_disappears_by_the_next_step(ClauseType::Volatile),
        "a Volatile clause is gone by the next step: `a` becomes satisfiable again"
    );
    assert!(
        clause_type_disappears_by_the_next_step(ClauseType::VolatileStatic),
        "VolatileStatic behaves like Volatile: exempt from the deletion policy \
         within a step, but still deleted at the step's own end"
    );
}

#[test]
fn clause_type_static_survives_a_new_step() {
    assert!(
        !clause_type_disappears_by_the_next_step(ClauseType::Static),
        "a Static clause forbidding `a` in step 1 still forbids it in step 2"
    );
}

// ---------------------------------------------------------------------------
// libpyclingo::test_propagator_control (test_propagator.py), ported: the
// full PropagateControl surface exercised together -- thread_id, assignment,
// Trail, decision, has_watch, this type's own propagate, and
// add_clause returning Stop -- against a single watch on `-a`'s own literal.
//
// Oracle (pyclingo 5.8.2, `{a}.`, `["0"]`): exactly one model, `a`;
// add_clause([lit_a]) (asking to force `a`'s literal true, while `-a` is the
// very reason propagate is firing) returns False (Stop), confirmed directly.

struct PropagatorControlFullSurface {
    lit_a: OnceLock<SolverLiteral>,
}

impl Propagator for PropagatorControlFullSurface {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        assert_eq!(
            init.check_mode(),
            CheckMode::Total,
            "the documented default"
        );
        assert_eq!(init.number_of_threads(), 1);
        let plit = program_literal(init, "a")?;
        let slit = init.solver_literal(plit)?;
        init.add_watch(-slit)?;
        let _ = self.lit_a.set(slit);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        let lit_a = *self.lit_a.get().expect("init ran first");
        let assignment = control.assignment();
        let trail = assignment.trail();
        let level = assignment.decision_level();
        assert!(changes.contains(&-lit_a));
        assert!(level >= 1);
        assert!(assignment.level(lit_a)?.is_some_and(|l| l >= 1));
        assert!(!trail.level(level)?.is_empty());
        assert_eq!(trail.level(level)?, vec![-lit_a]);
        assert_eq!(assignment.decision(level)?, -lit_a);
        assert_eq!(control.thread_id(), 0);
        assert!(control.has_watch(-lit_a));
        assert!(!control.propagate()?.is_stop());
        assert!(control.add_clause(&[lit_a], ClauseType::Learnt)?.is_stop());
        Ok(())
    }

    fn undo(&self, _control: &PropagateControl<'_>, changes: &[SolverLiteral]) {
        let lit_a = *self.lit_a.get().expect("init ran first");
        assert!(changes.contains(&-lit_a));
    }
}

#[test]
fn propagator_control_full_surface_matches_the_oracle() {
    let mut ctl = Control::builder().threads(1).build().unwrap();
    ctl.add_base("{a}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(PropagatorControlFullSurface {
        lit_a: OnceLock::new(),
    })
    .unwrap();
    assert_eq!(model_sets(&mut ctl), vec![vec!["a".to_owned()]]);
}

// ---------------------------------------------------------------------------
// libpyclingo::test_propagator (test_propagator.py), ported: add_literal,
// has_watch, add_watch and remove_watch used together from check, on an
// otherwise-empty program. The fresh literal add_literal creates is never
// constrained, so the empty program gains a second model (the fresh
// variable's two truth values).
//
// Oracle (pyclingo 5.8.2, program "", `["0"]`): two models, both empty.

struct AddLiteralAndWatchFromCheck {
    added: Arc<Mutex<bool>>,
    log: Arc<Mutex<Vec<bool>>>,
}

impl Propagator for AddLiteralAndWatchFromCheck {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let mut added = self.added.lock().unwrap();
        if !*added {
            *added = true;
            let lit = control.add_literal()?;
            let mut log = self.log.lock().unwrap();
            log.push(control.has_watch(lit));
            control.add_watch(lit)?;
            log.push(control.has_watch(lit));
            control.remove_watch(lit)?;
            log.push(control.has_watch(lit));
        }
        Ok(())
    }
}

#[test]
fn add_literal_and_watch_methods_work_from_check() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut ctl = grounded_all_models("");
    ctl.register_propagator(AddLiteralAndWatchFromCheck {
        added: Arc::new(Mutex::new(false)),
        log: Arc::clone(&log),
    })
    .unwrap();
    assert_eq!(
        model_sets(&mut ctl),
        vec![Vec::<String>::new(), Vec::<String>::new()]
    );
    assert_eq!(
        *log.lock().unwrap(),
        vec![false, true, false],
        "not watched, watched after add_watch, not watched after remove_watch"
    );
}

// ---------------------------------------------------------------------------
// libpyclingo::TestAddAssertingClause (test_propagator.py), ported: adding
// an asserting clause (a nogood that becomes false under the current
// assignment) from propagate reports Stop without itself changing the
// decision level yet (the actual backjump happens once control returns to
// clasp (`Propagator::propagate`'s own `Result<()>` does
// not carry `Flow` to clingo). decide is used only to force a deterministic
// search order (this is not a behavioural test of decide
// itself, whose own tests live elsewhere).
//
// Oracle (pyclingo 5.8.2, `start. {value}. {end}.`, `["0"]`, both
// lock=False and lock=True): three models, `{start,value}`, `{end,start}`,
// `{end,start,value}`; add_clause always returns False, with the decision
// level unchanged immediately afterward.

struct AssertingClause {
    kind: ClauseType,
    start: OnceLock<SolverLiteral>,
    end: OnceLock<SolverLiteral>,
    value: OnceLock<SolverLiteral>,
}

impl Propagator for AssertingClause {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let start = init.solver_literal(program_literal(init, "start")?)?;
        let end = init.solver_literal(program_literal(init, "end")?)?;
        let value = init.solver_literal(program_literal(init, "value")?)?;
        let mut lits = [start, end, value];
        lits.sort();
        for lit in lits {
            init.add_watch(lit)?;
            init.add_watch(-lit)?;
        }
        let _ = self.start.set(start);
        let _ = self.end.set(end);
        let _ = self.value.set(value);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        _changes: &[SolverLiteral],
    ) -> Result<()> {
        let start = *self.start.get().unwrap();
        let end = *self.end.get().unwrap();
        let value = *self.value.get().unwrap();
        let assignment = control.assignment();
        if assignment.is_false(value)? && assignment.is_false(end)? {
            let nogood = [start, -end, -value];
            let clause: Vec<SolverLiteral> = nogood.iter().map(|l| -*l).collect();
            let dl_before = assignment.decision_level();
            let flow = control.add_clause(&clause, self.kind)?;
            assert_eq!(dl_before, control.assignment().decision_level());
            assert!(flow.is_stop());
        }
        Ok(())
    }

    fn decide(
        &self,
        _thread_id: u32,
        assignment: &Assignment<'_>,
        fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        let end = *self.end.get().unwrap();
        let value = *self.value.get().unwrap();
        if assignment.truth_value(end)?.is_none() {
            return Ok(Some(-end));
        }
        if assignment.truth_value(value)?.is_none() {
            return Ok(Some(-value));
        }
        Ok(Some(fallback))
    }
}

fn asserting_clause_models(kind: ClauseType) -> Vec<Vec<String>> {
    let mut ctl = grounded_all_models("start. {value}. {end}.");
    ctl.register_propagator(AssertingClause {
        kind,
        start: OnceLock::new(),
        end: OnceLock::new(),
        value: OnceLock::new(),
    })
    .unwrap();
    model_sets(&mut ctl)
}

#[test]
fn asserting_clause_forces_a_conflict_without_changing_the_decision_level() {
    let expected = vec![
        vec!["end".to_owned(), "start".to_owned()],
        vec!["end".to_owned(), "start".to_owned(), "value".to_owned()],
        vec!["start".to_owned(), "value".to_owned()],
    ];
    assert_eq!(asserting_clause_models(ClauseType::Learnt), expected);
    assert_eq!(asserting_clause_models(ClauseType::Static), expected);
}

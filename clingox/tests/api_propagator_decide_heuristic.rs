//! `Propagator::decide`'s own behaviour: steering the search, the
//! fallback chain across two registered propagators in both registration
//! orders (plus the `None`-vs-`Some(fallback)` distinction, below),
//! running on every solver thread at `-t 4`, poisoning (measured, decision
//! 8's `decide` branch), and a panic caught and resumed.
//!
//! `decide`'s *dispatch* (the trampoline that validates a returned literal
//! against `clingo_assignment_has_literal` before handing it to clingo) is
//! part of the trampoline and tested in `api_propagator_decide.rs`;
//! nothing here duplicates that file. This file needs only `Propagator::
//! decide` and `Assignment` (`Model`/`SolveControl` do not appear below).
//!
//! **`Propagator::decide` returns `Result<Option<SolverLiteral>>`**, because
//! a plain `Result<SolverLiteral>` had a real defect
//! had: returning the bare `fallback` literal unchanged (the old trait's
//! "no opinion" default) was indistinguishable, at the C level, from
//! choosing it, so a propagator with no opinion could never actually
//! decline once more than one propagator was registered. `None` is the new
//! default and the real decline; `Some(fallback)` is still a real, distinct
//! choice. The fallback-chain tests below (`decide_falls_through_a_no_op_
//! propagator_registered_first`/`_second`,
//! `deliberately_choosing_the_fallback_literal_still_blocks_a_later_
//! propagator`, `declining_everywhere_uses_clingos_own_choice`) are the
//! regression coverage for this fix.
//!
//! Every expected value below was checked directly against clingo 5.8.2
//! (the Python module `clingo`, 2026-09-28). The search-steering test ports
//! `test_propagator.py::test_heurisitc` (`TestHeuristic`, `NOT_PORTED.md`'s
//! own upstream spelling).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use clingox::propagate::{Assignment, PropagateInit, Propagator, SolverLiteral};
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

/// The shown symbols of the first model found, as sorted text, or `None` if
/// the program is unsatisfiable.
fn first_model(ctl: &mut Control) -> Option<Vec<String>> {
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle.next_model().unwrap()?;
    let mut syms: Vec<String> = model
        .symbols(clingox::ShowType::SHOWN)
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    syms.sort();
    Some(syms)
}

// ---------------------------------------------------------------------------
// decide steers the search: a propagator that forces a's literal true, then
// (once a is no longer free) forces b's literal false, whenever propagation
// reaches a fixpoint. Ported from `TestHeuristic`/`test_heurisitc`
// (`test_propagator.py`).
//
// Oracle (pyclingo 5.8.2, `["1"]`, `{a;b}.`):
// - default heuristic, no propagator at all: the first model is `{}` (both
//   choices false).
// - with `TestHeuristic` registered: the first model is `{a}` — a
//   genuinely different model, proving `decide` changed clasp's own choice,
//   not merely accompanying it.
//
// `assignment.is_free(lit)` (pyclingo) is `assignment.truth_value(lit)? ==
// None` here: `Assignment::truth_value` returns `Result<Option<bool>>`,
// `None` for a literal not yet assigned.
// variant'" rule).

struct ForcesAThenNotB {
    lit_a: std::sync::OnceLock<SolverLiteral>,
    lit_b: std::sync::OnceLock<SolverLiteral>,
}

impl Propagator for ForcesAThenNotB {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit_a = program_literal(init, "a")?;
        let plit_b = program_literal(init, "b")?;
        let _ = self.lit_a.set(init.solver_literal(plit_a)?);
        let _ = self.lit_b.set(init.solver_literal(plit_b)?);
        Ok(())
    }

    fn decide(
        &self,
        _thread_id: u32,
        assignment: &Assignment<'_>,
        _fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        let lit_a = *self.lit_a.get().expect("init ran first");
        let lit_b = *self.lit_b.get().expect("init ran first");
        if assignment.truth_value(lit_a)?.is_none() {
            return Ok(Some(lit_a));
        }
        if assignment.truth_value(lit_b)?.is_none() {
            return Ok(Some(-lit_b));
        }
        // Both already decided: this propagator has nothing left to add,
        // a real decline (`None`), not a choice that merely happens to
        // equal `fallback`.
        Ok(None)
    }
}

impl Default for ForcesAThenNotB {
    fn default() -> Self {
        ForcesAThenNotB {
            lit_a: std::sync::OnceLock::new(),
            lit_b: std::sync::OnceLock::new(),
        }
    }
}

#[test]
fn default_heuristic_with_no_propagator_finds_the_empty_model_first() {
    let mut ctl = Control::with_args(["--models=1"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(first_model(&mut ctl), Some(Vec::<String>::new()));
}

#[test]
fn decide_forcing_a_literal_true_changes_which_model_is_found_first() {
    let mut ctl = Control::with_args(["--models=1"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(ForcesAThenNotB::default()).unwrap();
    assert_eq!(first_model(&mut ctl), Some(vec!["a".to_owned()]));
}

// ---------------------------------------------------------------------------
// decide returning the trait's default (the fallback literal, intended as
// clingox's equivalent of pyclingo's `decide` returning `0`) lets a
// later-registered propagator, or the solver's own heuristic, decide instead —
// in both registration orders. Oracle: `{a;b}.`, `--models=1`, both orders find
// `{a}` (the forcing propagator's choice always wins, regardless of when it was
// registered relative to the no-op one; regardless of when it was registered
// relative to the no-op one).
//
// **This section documents a real defect this test suite found in an earlier
// `decide` trampoline, now fixed.** Checked directly against the vendored
// source: `clingo_propagator_t::decide`'s own header doc says plainly "In case
// multiple propagators are registered, this function can return 0 to let a
// propagator registered later make a decision" (`H:1567-1568`), and
// `ClingoControl::decide` (`clingocontrol.cc:618-626`) implements exactly that:
// it calls every registered propagator's `decide` **in registration order**,
// and returns the **first non-zero** result immediately — `for (auto &heu :
// heus_) { auto ret = heu->decide(...); if (ret != 0) { return ret; } } return
// fallback;`. The earlier trampoline always wrote the returned literal to
// clingo's `*decision` out-pointer, whether the Rust `Propagator::decide`
// implementation genuinely chose a literal or only returned `fallback`
// unchanged (the old trait's own documented "no opinion" default) — and
// `fallback` is never `0`, so clingox could **never** send clingo the `0` that
// means "ask the next propagator." A propagator registered *first* that
// declined therefore silently swallowed every later-registered propagator's own
// heuristic.
//
// **The fix:** `Propagator::decide` now returns `Result<Option<
// SolverLiteral>>`, `None` (the new default) mapped to the raw `0` clingo
// checks for, `Some(literal)` validated and written as before. This is a change
// to `Propagator`'s own signature, chosen over the non-invasive alternative of
// a value-equality check in the trampoline alone, on the grounds that a
// propagator which deliberately picks the fallback literal must still count as
// a real choice, not be silently reinterpreted as a decline). `no_op_then_
// forces_a`/`forces_a_then_no_op` below now both pass in either registration
// order; `fallback_still_blocks_a_later_propagator` and
// `declining_everywhere_uses_clingos_own_choice` are the two new cases pinning
// the distinction between "declining" (`None`) and "choosing the same literal
// fallback would have picked anyway" (`Some(fallback)`), which is exactly the
// distinction the old signature could not express.

struct NoOpDecide;
impl Propagator for NoOpDecide {}

/// Always returns `Some(fallback)`: a deliberate choice, even though its
/// value happens to equal clasp's own pick, and must still block a
/// later-registered propagator the way `NoOpDecide`'s real decline
/// (`None`) does not.
struct AlwaysChoosesFallback;
impl Propagator for AlwaysChoosesFallback {
    fn decide(
        &self,
        _thread_id: u32,
        _assignment: &Assignment<'_>,
        fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        Ok(Some(fallback))
    }
}

struct ForcesA {
    lit_a: std::sync::OnceLock<SolverLiteral>,
}
impl Propagator for ForcesA {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let plit_a = program_literal(init, "a")?;
        let _ = self.lit_a.set(init.solver_literal(plit_a)?);
        Ok(())
    }

    fn decide(
        &self,
        _thread_id: u32,
        assignment: &Assignment<'_>,
        _fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        let lit_a = *self.lit_a.get().expect("init ran first");
        if assignment.truth_value(lit_a)?.is_none() {
            Ok(Some(lit_a))
        } else {
            Ok(None)
        }
    }
}
impl Default for ForcesA {
    fn default() -> Self {
        ForcesA {
            lit_a: std::sync::OnceLock::new(),
        }
    }
}

fn no_op_then_forces_a() -> Vec<String> {
    let mut ctl = Control::with_args(["--models=1"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(NoOpDecide).unwrap();
    ctl.register_propagator(ForcesA::default()).unwrap();
    first_model(&mut ctl).expect("the program is satisfiable")
}

fn forces_a_then_no_op() -> Vec<String> {
    let mut ctl = Control::with_args(["--models=1"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(ForcesA::default()).unwrap();
    ctl.register_propagator(NoOpDecide).unwrap();
    first_model(&mut ctl).expect("the program is satisfiable")
}

#[test]
fn decide_falls_through_a_no_op_propagator_registered_first() {
    assert_eq!(no_op_then_forces_a(), vec!["a".to_owned()]);
}

#[test]
fn decide_falls_through_a_no_op_propagator_registered_second() {
    assert_eq!(forces_a_then_no_op(), vec!["a".to_owned()]);
}

// Oracle (pyclingo 5.8.2, `["1"]`, `{a;b}.`): a propagator that always
// returns `fallback` (a real choice, upstream's own sentinel-free
// semantics: any nonzero return, including one that happens to equal
// `fallback`, is a choice) registered before `ForceA` finds the empty
// model `[]`, the same as the default heuristic with no propagator at
// all — `ForceA` is never consulted, unlike the `None`-declining case
// above.
#[test]
fn deliberately_choosing_the_fallback_literal_still_blocks_a_later_propagator() {
    let mut ctl = Control::with_args(["--models=1"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(AlwaysChoosesFallback).unwrap();
    ctl.register_propagator(ForcesA::default()).unwrap();
    assert_eq!(
        first_model(&mut ctl).expect("the program is satisfiable"),
        Vec::<String>::new(),
        "an explicit Some(fallback) must stop the chain, unlike None"
    );
}

// Oracle: two propagators that always return `0` (`clingo`'s own
// decline): `--models=1` finds `[]`, identical to the no-propagator-at-all
// baseline (`default_heuristic_with_no_propagator_finds_the_empty_model_
// first`, above) — declining everywhere truly falls through to clasp's
// own choice, on every thread's own call, not only a single propagator's.
#[test]
fn declining_everywhere_uses_clingos_own_choice() {
    let mut ctl = Control::with_args(["--models=1"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(NoOpDecide).unwrap();
    ctl.register_propagator(NoOpDecide).unwrap();
    assert_eq!(
        first_model(&mut ctl).expect("the program is satisfiable"),
        Vec::<String>::new()
    );
}

// ---------------------------------------------------------------------------
// decide runs on every solver thread at -t 4: each thread's own calls carry
// that thread's own id (clasp's `s.id()`, the same space `PropagateControl::
// thread_id` and `Model::thread_id` read, as cross-checked in
// `api_model_thread_id.rs`). A symmetric counting program with enough
// choices that clasp's 4 workers each get real search space is needed;
// checked directly against pyclingo with the identical program and thread
// count: all of thread ids {0, 1, 2, 3} appear among `decide`'s calls.

struct RecordsDecideThreads {
    seen: Arc<Mutex<HashSet<u32>>>,
}
impl Propagator for RecordsDecideThreads {
    fn decide(
        &self,
        thread_id: u32,
        _assignment: &Assignment<'_>,
        _fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        self.seen.lock().unwrap().insert(thread_id);
        Ok(None)
    }
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build cannot start threads"
)]
fn decide_runs_on_every_solver_thread_at_four_threads() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let seen = Arc::new(Mutex::new(HashSet::new()));
    let mut ctl = Control::builder()
        .threads(4)
        .args(["--models=0"])
        .build()
        .unwrap();
    ctl.add_base("1 {p(1..8)} 8. :- #count{X: p(X)} != 4.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(RecordsDecideThreads {
        seen: Arc::clone(&seen),
    })
    .unwrap();
    let (result, _models) = ctl.solve_all().unwrap();
    assert!(result.is_exhausted());
    let seen = seen.lock().unwrap();
    assert!(
        seen.len() > 1,
        "expected decide to run on more than one solver thread, saw {seen:?}"
    );
    assert!(
        seen.iter().all(|&id| id < 4),
        "every decide thread id must be below the configured thread count: {seen:?}"
    );
}

// ---------------------------------------------------------------------------
// decide's poisoning behaviour (measured the way propagate's and check's were): a propagator whose
// decide fails on its first call only. The first solve reports the error; a
// second, independent solve on the same control, where decide no longer fails,
// completes normally: decide does not poison, joining propagate/check's own
// already-measured "does not poison" branch. Oracle: raising on the first
// `decide` call and succeeding afterward behaves identically in pyclingo.

#[derive(Debug)]
struct Boom;
impl std::fmt::Display for Boom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("boom")
    }
}
impl std::error::Error for Boom {}

struct FailsDecideOnce {
    calls: Arc<Mutex<u32>>,
}
impl Propagator for FailsDecideOnce {
    fn decide(
        &self,
        _thread_id: u32,
        _assignment: &Assignment<'_>,
        _fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            Err(clingox::Error::callback(Boom))
        } else {
            Ok(None)
        }
    }
}

#[test]
fn a_failed_decide_does_not_poison_the_control() {
    let calls = Arc::new(Mutex::new(0));
    let mut ctl = grounded("{a;b}.");
    ctl.register_propagator(FailsDecideOnce {
        calls: Arc::clone(&calls),
    })
    .unwrap();

    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);

    // A later solve, where decide no longer fails, completes normally: the
    // earlier failure did not poison the control.
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());
    assert!(*calls.lock().unwrap() >= 2);
}

// ---------------------------------------------------------------------------
// A panic inside decide is caught and resumes on the caller's thread once
// the outer solve returns, never unwinding into clingo's C++ frames (S8,
// the same pattern every other Propagator callback already has a test
// for). `{a;b}.` reliably calls decide twice, once per free choice
// (checked directly); panicking on the first call is
// enough to exercise the trampoline's catch.

struct PanicsInDecide;
impl Propagator for PanicsInDecide {
    fn decide(
        &self,
        _thread_id: u32,
        _assignment: &Assignment<'_>,
        _fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        panic!("decide panics on purpose");
    }
}

#[test]
fn a_panic_in_decide_is_caught_and_resumes_on_the_caller_after_the_solve() {
    let mut ctl = grounded("{a;b}.");
    ctl.register_propagator(PanicsInDecide).unwrap();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller, not a solver thread");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("decide panics on purpose")
    );
}

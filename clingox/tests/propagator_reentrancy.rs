//! DESIGN S11's own gate: tests that force backjumps with 1,
//! 2 and 8 threads must pass under `TSan`.
//! `cargo xtask sanitize` runs every `#[test]` in this file under the thread
//! sanitizer (`xtask/src/sanitize.rs`'s `THREAD_TESTS`); nothing here is
//! gated to a sanitized build specifically, so the ordinary `cargo xtask
//! test linux` run also exercises the same functional behaviour.
//!
//! Two kinds of test:
//!
//! - **The forced-backjump test**: a propagator whose `propagate` adds a
//!   clause that immediately conflicts with the current partial assignment,
//!   asserting at a decision level below the current one, forcing clasp to
//!   backtrack past the propagator's own watched literals before `propagate`
//!   returns. **Pins that `undo` never runs while `propagate` is still on
//!   the same thread's own call stack for this scenario**: found, by direct
//!   instrumentation of `clasp/src/clingo.cpp`, to be structurally
//!   impossible through `PropagateControl::add_clause` in this clasp build,
//!   not merely unobserved. Detected directly with a per-thread "currently
//!   inside propagate" flag `undo` checks, not by reading a counter after
//!   the fact.
//! - **The data-race probe**: `PerThread`-shaped state (a `Vec<Mutex<_>>>`
//!   sized from `PropagateInit::number_of_threads`, indexed by
//!   `PropagateControl::thread_id`) read and written from `propagate` on
//!   every thread of an 8-thread solve over a program with enough symmetry
//!   that every thread's propagator instance fires frequently (a
//!   no-two-adjacent constraint over a larger domain than the dedicated
//!   correctness test uses, enumerated fully).
//!
//! Every expected value below was checked directly against clingo 5.8.2 (the
//! Python module `clingo`, 2026-09-28) and, for the forced-backjump test's
//! own finding, against `clasp/src/clingo.cpp` directly.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use clingox::propagate::{
    Assignment, ClauseType, PropagateControl, PropagateInit, Propagator, SolverLiteral,
};
use clingox::{Control, Part, Result, Signature};

fn program_literal(init: &PropagateInit<'_>, name: &str) -> Result<clingox::ProgramLiteral> {
    let sig = Signature::new(name, 0)?;
    Ok(init
        .symbolic_atoms()?
        .by_signature(sig)
        .next()
        .unwrap_or_else(|| panic!("{name} is an atom"))?
        .literal())
}

// =============================================================================
// The forced-backjump test.
//
// `{b; d; e}.`, decide forces b, then d, then e true, one at a time. The first
// time e's watch fires propagate, the propagator adds a unit, volatile clause
// forcing b to the opposite of its own current value (an immediate conflict at
// b's own, lower decision level), which clasp resolves by backtracking past d
// and e, undoing both. Checked directly (a representative Python transcript,
// three runs): a genuine multi-level backjump, never a single-level backtrack,
// confirming the scenario exercises "backtrack past the propagator's own
// watched literals," not chronological backtracking.
//
// The volatile clause and the "fire once per solving step" guard both exist
// only to keep the scenario itself deterministic and non-self-contradictory (an
// unguarded, repeated forcing flips b's own forced polarity back and forth
// across restarts of the search and makes the program UNSAT instead of
// exercising a resolvable conflict, found directly while designing this test);
// they are not part of the reentrancy question under test.

struct ForcedBackjump {
    b: OnceLock<SolverLiteral>,
    d: OnceLock<SolverLiteral>,
    e: OnceLock<SolverLiteral>,
    /// One flag per solver thread: `true` while that thread's own
    /// `Propagator::propagate` call is on the stack.
    in_propagate: OnceLock<Vec<AtomicBool>>,
    /// Incremented by `undo` when it observes its own thread's
    /// `in_propagate` flag still set: this is the reentrancy claim itself.
    /// Shared with the test itself (an `Arc` cloned in before
    /// registration), since `Control` takes ownership of the propagator.
    reentrant_undo_count: Arc<AtomicU32>,
    /// Every undo call, reentrant or not: a sanity floor (a genuine
    /// multi-level backjump undoes more than one change, regardless of
    /// whether it is caught as reentrant).
    total_undo_count: Arc<AtomicU32>,
    fired: Mutex<bool>,
}

impl ForcedBackjump {
    fn new(reentrant_undo_count: Arc<AtomicU32>, total_undo_count: Arc<AtomicU32>) -> Self {
        ForcedBackjump {
            b: OnceLock::new(),
            d: OnceLock::new(),
            e: OnceLock::new(),
            in_propagate: OnceLock::new(),
            reentrant_undo_count,
            total_undo_count,
            fired: Mutex::new(false),
        }
    }
}

impl Propagator for ForcedBackjump {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let b = init.solver_literal(program_literal(init, "b")?)?;
        let d = init.solver_literal(program_literal(init, "d")?)?;
        let e = init.solver_literal(program_literal(init, "e")?)?;
        for lit in [b, d, e] {
            init.add_watch(lit)?;
            init.add_watch(-lit)?;
        }
        let _ = self.b.set(b);
        let _ = self.d.set(d);
        let _ = self.e.set(e);
        let flags = (0..init.number_of_threads())
            .map(|_| AtomicBool::new(false))
            .collect();
        let _ = self.in_propagate.set(flags);
        *self.fired.lock().unwrap() = false;
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        let tid = control.thread_id() as usize;
        let flags = self.in_propagate.get().expect("init ran first");
        flags[tid].store(true, Ordering::SeqCst);

        let e = *self.e.get().expect("init ran first");
        if changes.contains(&e) {
            let mut fired = self.fired.lock().unwrap();
            if !*fired {
                *fired = true;
                let b = *self.b.get().expect("init ran first");
                let assignment = control.assignment();
                let b_is_true = assignment.is_true(b)?;
                let forced = if b_is_true { -b } else { b };
                let _ = control.add_clause(&[forced], ClauseType::Volatile)?;
            }
        }

        flags[tid].store(false, Ordering::SeqCst);
        Ok(())
    }

    fn undo(&self, control: &PropagateControl<'_>, _changes: &[SolverLiteral]) {
        let tid = control.thread_id() as usize;
        self.total_undo_count.fetch_add(1, Ordering::SeqCst);
        let flags = self.in_propagate.get().expect("init ran first");
        if flags[tid].load(Ordering::SeqCst) {
            self.reentrant_undo_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn decide(
        &self,
        _thread_id: u32,
        assignment: &Assignment<'_>,
        fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        // Forces a deterministic search order (b, then d, then e, all true
        // first) so the scenario above reproduces the same way every run,
        // on every thread; not itself a behavioural test of `decide`.
        for slot in [&self.b, &self.d, &self.e] {
            let lit = *slot.get().expect("init ran first");
            if assignment.truth_value(lit)?.is_none() {
                return Ok(Some(lit));
            }
        }
        Ok(Some(fallback))
    }
}

fn run_forced_backjump(threads: u32) -> (u32, u32) {
    // --models=0, matching the oracle scripts exactly (the scenario itself
    // is already forced by `decide` on the way to the very first model, so
    // this is for parity with the validated transcripts, not a correctness
    // requirement the way it is for the data-race probe's own model count).
    let mut ctl = Control::builder()
        .threads(threads)
        .args(["--models=0"])
        .build()
        .unwrap();
    ctl.add_base("{b; d; e}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let reentrant_undo_count = Arc::new(AtomicU32::new(0));
    let total_undo_count = Arc::new(AtomicU32::new(0));
    ctl.register_propagator(ForcedBackjump::new(
        Arc::clone(&reentrant_undo_count),
        Arc::clone(&total_undo_count),
    ))
    .unwrap();
    let mut models = 0;
    let _ = ctl
        .for_each_model(&[], |_m| {
            models += 1;
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(models > 0, "sanity: the program has at least one model");
    (
        total_undo_count.load(Ordering::SeqCst),
        reentrant_undo_count.load(Ordering::SeqCst),
    )
}

/// Pins what clingo 5.8.2 actually does for this scenario: a clause added
/// through `PropagateControl::add_clause`, asserting at a decision level
/// below the current one (this file's own fixture), never makes `undo` run
/// while `propagate` is still on the same thread's own call stack. Found by
/// direct instrumentation of `ClingoPropagator::addClause`
/// (`clasp/src/clingo.cpp`): `ClingoPropagator::Control`'s own constructor
/// unconditionally ORs `state_ctrl` into every `Control` object, including
/// the one handed to `Propagator::propagate`, so `addClause`'s own
/// `state_ctrl` guard always reports `Flow::Stop` before the synchronous
/// `s.undoUntil` branch is ever reached on this path; the actual backjump
/// happens afterward, through clasp's own separate, non-nested conflict
/// machinery. **A failure here means clingo started re-entering `undo`
/// from inside `add_clause`** -- a clingo-side change worth re-examining
/// (clingox's own `&self`/`Send + Sync` design for `Propagator`, and the
/// "never hold your own lock across a call into `PropagateControl`" rule,
/// both stay sound either way: they do not rely on this *not* happening,
/// only on handling it safely if it ever does). Asserted with the total
/// count in the message, since "0 reentrant, N total" (the expected case)
/// and "0 reentrant, 0 total" are different failures (the latter means the
/// scenario itself never triggered a backjump at all, a fixture bug, not a
/// clingo behaviour change).
fn assert_undo_never_reentrant(threads: u32) {
    let (total, reentrant) = run_forced_backjump(threads);
    assert!(
        total > 0,
        "sanity ({threads} threads): at least one undo call happened at all \
         (the backjump itself still needs to happen, just not reentrant)"
    );
    assert_eq!(
        reentrant, 0,
        "{threads} threads: undo ran {reentrant} times while propagate was on the same \
         thread's own call stack, out of {total} undo calls total -- clingo 5.8.2 does not \
         do this for an add_clause-triggered backjump; if this \
         assertion now fails, clingo has started re-entering undo from add_clause, see the \
         oracle before assuming the test is wrong"
    );
}

#[test]
fn undo_never_runs_inside_propagate_after_add_clause_at_one_thread() {
    assert_undo_never_reentrant(1);
}

#[test]
fn undo_never_runs_inside_propagate_after_add_clause_at_two_threads() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    assert_undo_never_reentrant(2);
}

#[test]
fn undo_never_runs_inside_propagate_after_add_clause_at_eight_threads() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    assert_undo_never_reentrant(8);
}

// =============================================================================
// The data-race probe: PerThread-shaped state read and written from
// propagate on every thread, over a no-two-adjacent constraint on a larger
// domain than propagator_no_two_adjacent.rs's own correctness test uses
// (`x(1..12)`, enumerated fully), so every thread's own propagator instance
// fires often. Each thread also exercises add_watch/has_watch/remove_watch
// from inside propagate (all touched concurrently),
// not only add_clause. TSan finding zero warnings here is this file's other
// half of DESIGN S11's own gate.

struct DataRaceProbe {
    lits: OnceLock<Vec<SolverLiteral>>,
    /// One log per solver thread: only that thread's own propagate call
    /// ever locks its own slot, but every thread's slot lives in the same
    /// Vec, exactly the `PerThread` shape DESIGN S11 recommends in place of
    /// the dropped `PerThread<T>`. Shared with the test itself (an
    /// `Arc<OnceLock<_>>` cloned in before registration), since `Control`
    /// takes ownership of the propagator and the only way to read what it
    /// saw afterward is through a handle kept beforehand. Records the OS
    /// thread (`std::thread::current().id()`) that wrote each entry, not
    /// only a count: a shared or off-by-one `thread_id()` index would show
    /// up as two different OS threads writing into the same slot, which a
    /// bare total (an earlier version of this test) could not catch.
    per_thread_log: Arc<OnceLock<Vec<Mutex<Vec<std::thread::ThreadId>>>>>,
}

impl DataRaceProbe {
    fn neighbour(&self, lit: SolverLiteral) -> Option<SolverLiteral> {
        let lits = self.lits.get().expect("init ran first");
        let i = lits.iter().position(|&l| l == lit)?;
        lits.get(i + 1).copied()
    }
}

impl Propagator for DataRaceProbe {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let sig = Signature::new("x", 1)?;
        let mut by_argument = Vec::new();
        for atom in init.symbolic_atoms()?.by_signature(sig) {
            let atom = atom?;
            let n = atom.symbol().arguments().unwrap()[0]
                .as_number()
                .expect("x/1's own argument is a number");
            by_argument.push((n, atom.literal()));
        }
        // Sorted by x(_)'s own argument, not by the program literal's own
        // id (grounding order does not promise the two agree), so
        // `lits[i]`/`lits[i + 1]` are truly adjacent integers.
        by_argument.sort_by_key(|&(n, _)| n);
        let mut lits = Vec::new();
        for (_, plit) in by_argument {
            let slit = init.solver_literal(plit)?;
            init.add_watch(slit)?;
            lits.push(slit);
        }
        let _ = self.lits.set(lits);
        let logs = (0..init.number_of_threads())
            .map(|_| Mutex::new(Vec::new()))
            .collect();
        let _ = self.per_thread_log.set(logs);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        let tid = control.thread_id() as usize;
        let logs = self.per_thread_log.get().expect("init ran first");
        for &lit in changes {
            // Exercise add_watch/has_watch/remove_watch concurrently too,
            // on a harmless, already-watched literal (added through
            // PropagateInit, so this is a genuine round trip, not a no-op:
            // has_watch is true both before and after, since the middle
            // add_watch/remove_watch pair only ever touches this thread's
            // own watch state, never the PropagateInit-level one).
            assert!(control.has_watch(lit));
            control.add_watch(lit)?;
            assert!(control.has_watch(lit));
            control.remove_watch(lit)?;
            control.add_watch(lit)?; // restore PropagateInit's own watch

            if let Some(next) = self.neighbour(lit) {
                let flow = control.add_clause(&[-lit, -next], ClauseType::Learnt)?;
                if flow.is_stop() {
                    return Ok(());
                }
                logs[tid].lock().unwrap().push(std::thread::current().id());
            }
        }
        Ok(())
    }
}

fn run_data_race_probe(threads: u32) -> usize {
    // --models=0: for_each_model still stops at clingo's own default of one
    // model otherwise, undercounting the 377 independent sets this test
    // enumerates fully (found the hard way while writing propagator_no_two_
    // adjacent.rs's own correctness test).
    let mut ctl = Control::builder()
        .threads(threads)
        .args(["--models=0"])
        .build()
        .unwrap();
    ctl.add_base("{x(1..12)}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let per_thread_log = Arc::new(OnceLock::new());
    ctl.register_propagator(DataRaceProbe {
        lits: OnceLock::new(),
        per_thread_log: Arc::clone(&per_thread_log),
    })
    .unwrap();

    let mut models = 0;
    let _ = ctl
        .for_each_model(&[], |_m| {
            models += 1;
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .unwrap();
    // Independent sets of a path of 12 nodes: 377 (checked directly against
    // clingo 5.8.2, the same no-two-adjacent logic as propagator_no_two_
    // adjacent.rs's own correctness test). Not this test's own point (that is
    // TSan finding no warnings), but a cheap sanity floor that the probe's own
    // add_watch/remove_watch round trip did not silently break the constraint
    // it is layered on top of.
    assert_eq!(
        models, 377,
        "sanity: the model count is unaffected by the round trip"
    );

    let logs = per_thread_log.get().expect("init ran");
    let mut seen_os_threads = std::collections::HashSet::new();
    let mut total = 0;
    for (i, log) in logs.iter().enumerate() {
        let entries = log.lock().unwrap();
        total += entries.len();
        if let Some(&first) = entries.first() {
            assert!(
                entries.iter().all(|&id| id == first),
                "slot {i}: every entry was written by the same OS thread \
                 ({first:?}), not a mix (a shared or off-by-one thread_id() \
                 index would show up as two different OS threads writing \
                 into the same slot)"
            );
            assert!(
                seen_os_threads.insert(first),
                "slot {i}'s own OS thread ({first:?}) already wrote into a \
                 different slot: two solver thread ids collapsed onto one \
                 OS thread, or thread_id() misreported"
            );
        }
    }
    total
}

#[test]
fn data_race_probe_at_one_thread() {
    assert!(run_data_race_probe(1) > 0);
}

#[test]
fn data_race_probe_at_two_threads() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    assert!(run_data_race_probe(2) > 0);
}

#[test]
fn data_race_probe_at_eight_threads() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    assert!(run_data_race_probe(8) > 0);
}

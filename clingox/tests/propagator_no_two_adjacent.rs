//! `NoTwoAdjacent`, a worked example, ported nearly
//! verbatim as a first correctness test for `PropagateControl::
//! add_clause`: models of `{ x(1..5) }` under a no-two-adjacent constraint
//! enforced purely through the propagator, no ASP-level constraint at all.
//!
//! `PerThread<T>` was dropped from the crate; this port keeps its
//! own per-thread state in a `Vec<Mutex<Vec<SolverLiteral>>>` sized from
//! `PropagateInit::number_of_threads` and indexed by `PropagateControl::
//! thread_id`, matching DESIGN S11's own recommended shape for state a
//! propagator needs to *mutate* per thread.
//!
//! Oracle (pyclingo 5.8.2, `{x(1..5)}.`, `["0"]`, checked directly): 13
//! models, one per independent set of the path graph on 5 nodes (Fibonacci
//! `F(7) = 13`).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::sync::{Arc, Mutex, OnceLock};

use clingox::propagate::{ClauseType, PropagateControl, PropagateInit, Propagator, SolverLiteral};
use clingox::{Control, Part, Result, Signature};

struct NoTwoAdjacent {
    /// `x(1)..x(5)`'s own solver literals, in argument order; set once in
    /// `init`.
    lits: OnceLock<Vec<SolverLiteral>>,
}

impl NoTwoAdjacent {
    fn neighbour(&self, lit: SolverLiteral) -> Option<SolverLiteral> {
        let lits = self.lits.get().expect("init ran first");
        let i = lits.iter().position(|&l| l == lit)?;
        lits.get(i + 1).copied()
    }
}

impl Propagator for NoTwoAdjacent {
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
        // Sorted by x(_)'s own argument (not by the program literal's own
        // id, which grounding order does not promise matches argument
        // order), so `lits[i]`/`lits[i + 1]` are truly adjacent integers.
        by_argument.sort_by_key(|&(n, _)| n);
        let mut lits = Vec::new();
        for (_, plit) in by_argument {
            let slit = init.solver_literal(plit)?;
            init.add_watch(slit)?;
            lits.push(slit);
        }
        let _ = self.lits.set(lits);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        for &lit in changes {
            if let Some(next) = self.neighbour(lit)
                && control
                    .add_clause(&[-lit, -next], ClauseType::Learnt)?
                    .is_stop()
            {
                return Ok(()); // clingo asked us to stop
            }
        }
        Ok(())
    }
}

#[test]
fn no_two_adjacent_matches_the_independent_sets_of_a_path_of_five() {
    // `--models=0`: for_each_model enumerates every model clingo finds, but
    // still stops at whatever model *count* the control is configured for
    // (1, by clingo's own default); this test wants every independent set,
    // not only the first.
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{x(1..5)}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(NoTwoAdjacent {
        lits: OnceLock::new(),
    })
    .unwrap();

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

    let expected: Vec<Vec<String>> = [
        vec![],
        vec!["x(1)"],
        vec!["x(1)", "x(3)"],
        vec!["x(1)", "x(3)", "x(5)"],
        vec!["x(1)", "x(4)"],
        vec!["x(1)", "x(5)"],
        vec!["x(2)"],
        vec!["x(2)", "x(4)"],
        vec!["x(2)", "x(5)"],
        vec!["x(3)"],
        vec!["x(3)", "x(5)"],
        vec!["x(4)"],
        vec!["x(5)"],
    ]
    .into_iter()
    .map(|m| m.into_iter().map(str::to_owned).collect())
    .collect();

    assert_eq!(
        models.len(),
        13,
        "one model per independent set of a path of 5 (Fibonacci F(7))"
    );
    assert_eq!(models, expected);
}

// ---------------------------------------------------------------------------
// The same constraint again, but with per-thread state mutated from propagate
// (the Vec<Mutex<Vec<SolverLiteral>>> shape DESIGN S11 recommends in place of a
// per-thread wrapper): each thread records, in its own slot, the OS thread
// (std::thread::current().id()) that wrote each entry, not only a count. A
// shared-index or off-by-one bug in PropagateControl::thread_id() (or in how a
// test indexes by it) would show up here as two different OS threads writing
// into the same slot; recording only totals (an earlier version of this test)
// could not catch that, since a total is the same regardless of which slot
// absorbed which thread's entries.

struct NoTwoAdjacentWithPerThreadLog {
    lits: OnceLock<Vec<SolverLiteral>>,
    /// Shared with the test itself for post-solve inspection: `Control`
    /// takes ownership of the propagator, so the only way to read what it
    /// saw afterward is through a handle cloned in before registration.
    per_thread: Arc<OnceLock<Vec<Mutex<Vec<std::thread::ThreadId>>>>>,
}

impl NoTwoAdjacentWithPerThreadLog {
    fn neighbour(&self, lit: SolverLiteral) -> Option<SolverLiteral> {
        let lits = self.lits.get().expect("init ran first");
        let i = lits.iter().position(|&l| l == lit)?;
        lits.get(i + 1).copied()
    }
}

impl Propagator for NoTwoAdjacentWithPerThreadLog {
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
        by_argument.sort_by_key(|&(n, _)| n);
        let mut lits = Vec::new();
        for (_, plit) in by_argument {
            let slit = init.solver_literal(plit)?;
            init.add_watch(slit)?;
            lits.push(slit);
        }
        let _ = self.lits.set(lits);
        let slots = (0..init.number_of_threads())
            .map(|_| Mutex::new(Vec::new()))
            .collect();
        let _ = self.per_thread.set(slots);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        for &lit in changes {
            if let Some(next) = self.neighbour(lit) {
                let flow = control.add_clause(&[-lit, -next], ClauseType::Learnt)?;
                if flow.is_stop() {
                    return Ok(());
                }
                let slots = self.per_thread.get().expect("init ran first");
                slots[control.thread_id() as usize]
                    .lock()
                    .unwrap()
                    .push(std::thread::current().id());
            }
        }
        Ok(())
    }
}

#[test]
fn per_thread_state_stays_private_to_its_own_thread() {
    let threads: u32 = if clingox_sys::HAS_THREADS { 4 } else { 1 };
    // --models=0, like the correctness test: a single default solve can
    // return the trivial empty model without ever making any x(i) true,
    // never exercising the propagator at all; enumerating every model
    // guarantees real exploration.
    let mut ctl = Control::builder()
        .threads(threads)
        .args(["--models=0"])
        .build()
        .unwrap();
    ctl.add_base("{x(1..5)}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let per_thread = Arc::new(OnceLock::new());
    ctl.register_propagator(NoTwoAdjacentWithPerThreadLog {
        lits: OnceLock::new(),
        per_thread: Arc::clone(&per_thread),
    })
    .unwrap();
    let mut models = 0;
    let _ = ctl
        .for_each_model(&[], |_m| {
            models += 1;
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(models, 13, "sanity: same correctness as the dedicated test");

    let slots = per_thread.get().expect("init ran");
    assert_eq!(
        slots.len(),
        threads as usize,
        "one slot per configured solver thread"
    );
    let mut seen_os_threads = std::collections::HashSet::new();
    let mut total = 0;
    for (i, slot) in slots.iter().enumerate() {
        let entries = slot.lock().unwrap();
        total += entries.len();
        if let Some(&first) = entries.first() {
            assert!(
                entries.iter().all(|&id| id == first),
                "slot {i}: every entry was written by the same OS thread \
                 ({first:?}), not a mix (a shared or off-by-one thread_id() \
                 index would show up as two different OS threads writing \
                 into the same slot): {entries:?}"
            );
            assert!(
                seen_os_threads.insert(first),
                "slot {i}'s own OS thread ({first:?}) already wrote into a \
                 different slot: two solver thread ids collapsed onto one \
                 OS thread, or thread_id() misreported"
            );
        }
    }
    assert!(
        total > 0,
        "at least one thread forbade at least one adjacent pair"
    );
}

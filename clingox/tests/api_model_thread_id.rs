//! `Model::thread_id`: matches the thread that actually found the
//! model, at `-t 1` and `-t 4`, cross-checked against `PropagateControl::
//! thread_id` to confirm the two numbering schemes agree.
//!
//! `Model::thread_id` is infallible (`u32`, not `Result<u32>`): see
//! why (`clingo_model_thread_id`'s own C body is a plain field read that cannot
//! throw, the same shape already established for `clingo_model_number` and
//! `PropagateControl::thread_id`). Every expected value below was checked
//! directly against clingo 5.8.2 (the Python module `clingo`,
//! 2026-09-28).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use std::ops::ControlFlow;

use clingox::propagate::{PropagateControl, PropagateInit, Propagator, SolverLiteral};
use clingox::{Control, ExtendableModel, Part, Result, SolveEventHandler, SolveOptions};

// ---------------------------------------------------------------------------
// A single-threaded solve: every model's thread_id is 0. Oracle (pyclingo
// 5.8.2, default thread count, `a.`): `m.thread_id == 0`.

#[test]
fn thread_id_is_zero_on_a_single_threaded_solve() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle.next_model().unwrap().expect("a. has a model");
    assert_eq!(model.thread_id(), 0);
}

// ---------------------------------------------------------------------------
// At -t 4, Model::thread_id matches PropagateControl::thread_id's own
// numbering for the same underlying solver thread: a propagator records,
// per model number, which thread's `propagate` call last ran before that
// model was found, keyed by the model's own running number (`Model::
// number`, unique within one solve); `on_model` then reads `Model::
// thread_id` for the same model number and compares. Both read the
// identical `clingo_id_t` space (`s.id()` in clasp): `model_thread_id` and
// `PropagateControl::thread_id` share the same space.
//
// The program needs enough symmetric choices that clasp's 4 workers each
// find at least one model; checked directly against pyclingo with the
// identical program and thread count: every one of thread ids {0, 1, 2, 3}
// is seen among the models found.

/// Records, for each solver thread number `PropagateControl` reports, the
/// operating-system thread that ran `propagate` with it.
struct RecordsPropagateThreads {
    os_thread_of: Arc<Mutex<HashMap<u32, std::thread::ThreadId>>>,
}
impl Propagator for RecordsPropagateThreads {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        for atom in &init.symbolic_atoms()? {
            let lit = init.solver_literal(atom?.literal())?;
            init.add_watch(lit)?;
        }
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        _changes: &[SolverLiteral],
    ) -> Result<()> {
        let _ = self
            .os_thread_of
            .lock()
            .unwrap()
            .insert(control.thread_id(), std::thread::current().id());
        Ok(())
    }
}

/// Records, for each model, the solver thread number `Model::thread_id`
/// reports and the operating-system thread that delivered it. clingo calls
/// the model event on the solver thread that found the model.
struct RecordsModelThreads {
    seen: Arc<Mutex<Vec<(u32, std::thread::ThreadId)>>>,
}
impl SolveEventHandler for RecordsModelThreads {
    fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
        self.seen
            .lock()
            .unwrap()
            .push((model.thread_id(), std::thread::current().id()));
        Ok(ControlFlow::Continue(()))
    }
}

/// `Model::thread_id` and `PropagateControl::thread_id` number solver
/// threads the same way: the model's number maps to the same
/// operating-system thread that ran `propagate` under that number. The
/// correlation goes through the operating-system thread, so it holds
/// whatever the other solver threads do meanwhile.
#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build cannot start threads"
)]
fn thread_id_matches_propagate_controls_own_numbering_at_four_threads() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let os_thread_of = Arc::new(Mutex::new(HashMap::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut ctl = Control::builder()
        .threads(4)
        .args(["--models=0"])
        .build()
        .unwrap();
    ctl.add_base("1 {p(1..8)} 8. :- #count{X: p(X)} != 4.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(RecordsPropagateThreads {
        os_thread_of: Arc::clone(&os_thread_of),
    })
    .unwrap();
    let _ = ctl
        .solve_with_events(
            SolveOptions::new(),
            RecordsModelThreads {
                seen: Arc::clone(&seen),
            },
        )
        .unwrap();

    let os_thread_of = os_thread_of.lock().unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 70, "C(8,4) models");
    // Which solver threads find models depends on scheduling: on a loaded
    // machine one thread can find all 70 (seen twice under parallel suites).
    // The claim tested here is the numbering, checked for every model below.
    for &(number, os_thread) in seen.iter() {
        assert!(number < 4, "thread number {number} out of range");
        assert_eq!(
            os_thread_of.get(&number),
            Some(&os_thread),
            "solver thread {number}: the model was delivered on a different operating-system \
             thread than the one PropagateControl reported as {number}"
        );
    }
}

// ---------------------------------------------------------------------------
// `Model::thread_id` on a model from `for_each_model` and from a
// `SolveEventHandler`'s `ExtendableModel` reports the same value as one
// from a plain `next_model` loop, for the same single-threaded solve
// (sanity: the accessor works identically across every model-delivery
// path, matching `Model::context`'s own "reachable from every path"
// finding, though `thread_id` needed no cross-path test in the contract:
// added here since it is nearly free once `for_each_model` is exercised
// anyway).

#[test]
fn thread_id_is_zero_from_for_each_model_too() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut seen = false;
    let _ = ctl
        .for_each_model(&[], |m| {
            assert_eq!(m.thread_id(), 0);
            seen = true;
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(seen, "sanity: the model callback ran");
}

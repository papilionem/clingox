//! `Control::register_propagator`/`register_propagator_sequential`: both entry
//! points, several independent propagators, and what clingo does when one
//! registered propagator's `init` fails or panics before another's has run.
//!
//! Every expected value below was checked directly against clingo 5.8.2 (the
//! Python module `clingo`, 2026-09-28), never assumed from `clingo.h`'s prose:
//! Python module `clingo`, 2026-09-28), never assumed from `clingo.h`'s prose.
//!
//! `Propagator`'s methods take `&self` (DESIGN S11), so every propagator here
//! that needs to record something uses interior mutability, never a field
//! mutated through `&mut self`.
//!
//! The smoke/correctness test lives in `api_propagator_control.rs`: it is
//! `PropagateControl` behaviour, not registration, and belongs in the file
//! named for that subject.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use clingox::propagate::{PropagateInit, Propagator};
use clingox::{Control, ErrorKind, Part, Result};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a fresh control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
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

/// A propagator with every method left at its default (a pure no-op).
struct NoOp;
impl Propagator for NoOp {}

// ---------------------------------------------------------------------------
// The smoke test: registering a propagator that does nothing does not change
// which models a program has.
//
// Oracle: pyclingo, `1 { a; b }.` with `ctl.solve(on_model=...)` under `["0"]`,
// registered and unregistered, produced the identical three models
// (`{a}`, `{b}`, `{a b}`).

#[test]
fn register_propagator_with_only_defaults_does_not_change_models() {
    let plain = model_sets(&mut grounded("1 { a; b }."));

    let mut ctl = grounded("1 { a; b }.");
    ctl.register_propagator(NoOp).unwrap();
    let with_noop = model_sets(&mut ctl);

    assert_eq!(with_noop, plain);
}

#[test]
fn register_propagator_sequential_with_only_defaults_does_not_change_models() {
    let plain = model_sets(&mut grounded("1 { a; b }."));

    let mut ctl = grounded("1 { a; b }.");
    ctl.register_propagator_sequential(NoOp).unwrap();
    let with_noop = model_sets(&mut ctl);

    assert_eq!(with_noop, plain);
}

// ---------------------------------------------------------------------------
// register_propagator called twice registers two independent propagators,
// both run.
//
// Oracle: two propagators registered on the same `Control`, each with its
// own `init` that records itself; both recorded, confirming clingo runs
// every registered propagator, not only the first or the last.

struct RecordsInit(Arc<AtomicUsize>);
impl Propagator for RecordsInit {
    fn init(&self, _init: &mut PropagateInit<'_>) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn two_registrations_are_two_independent_propagators_both_run() {
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let mut ctl = grounded("a.");
    ctl.register_propagator(RecordsInit(Arc::clone(&first)))
        .unwrap();
    ctl.register_propagator(RecordsInit(Arc::clone(&second)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(second.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// Multi-shot: a propagator registered once sees `init` on every solving
// step, not only the first.
//
// Oracle: registering one propagator, then calling `solve()` three times (a
// `ground` between the first and second, none between the second and
// third), its `init` was called once per `solve()` call: 1, then 2, then 3.
// "Solving step" therefore means "one `clingo_control_solve`," not "one
// `ground`."

#[test]
fn init_runs_once_per_solving_step_not_once_per_ground() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut ctl = grounded("a.");
    ctl.register_propagator(RecordsInit(Arc::clone(&counter)))
        .unwrap();

    let _ = ctl.solve(&[]).unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 1, "first solve");

    ctl.add("step2", &[], "b.").unwrap();
    ctl.ground(&[Part::new("step2", &[]).unwrap()]).unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "second solve, after a new ground"
    );

    let _ = ctl.solve(&[]).unwrap();
    assert_eq!(
        counter.load(Ordering::SeqCst),
        3,
        "third solve, with no new ground in between: a \"solving step\" is one \
         `solve()` call, not one `ground()` call"
    );
}

// ---------------------------------------------------------------------------
// A failing or panicking `init` on one registered propagator stops the
// remaining registered propagators' `init` from running at all, and does not
// un-run an earlier one that already completed.
//
// Oracle, both orders checked directly:
// - Raiser registered first, Recorder second: Recorder's `init` never runs.
// - Recorder registered first, Raiser second: Recorder's `init` had already
//   run and stays recorded; the exception still stops the solve.
// clingo calls each registered propagator's `init` in registration order,
// and a failure from one is fatal to the whole `init` phase, not only to the
// propagator that raised.

/// Records, through interior mutability, that its `init` ran.
/// `Propagator: Send + Sync` forbids a plain `Cell`/`Rc` here.
struct Recorder(Arc<Mutex<bool>>);
impl Propagator for Recorder {
    fn init(&self, _init: &mut PropagateInit<'_>) -> Result<()> {
        *self.0.lock().unwrap() = true;
        Ok(())
    }
}

struct ErrRaiser;
impl Propagator for ErrRaiser {
    fn init(&self, _init: &mut PropagateInit<'_>) -> Result<()> {
        Err(clingox::Error::new(ErrorKind::Callback, "boom"))
    }
}

struct PanicRaiser;
impl Propagator for PanicRaiser {
    fn init(&self, _init: &mut PropagateInit<'_>) -> Result<()> {
        panic!("boom");
    }
}

#[test]
fn an_erroring_inits_earlier_registered_sibling_already_ran_a_later_one_never_does() {
    let earlier = Arc::new(Mutex::new(false));
    let later = Arc::new(Mutex::new(false));
    let mut ctl = grounded("a.");
    ctl.register_propagator(Recorder(Arc::clone(&earlier)))
        .unwrap();
    ctl.register_propagator(ErrRaiser).unwrap();
    ctl.register_propagator(Recorder(Arc::clone(&later)))
        .unwrap();

    let err = ctl.solve(&[]).unwrap_err();
    // The first call returns the propagator's own error; it poisons, so a
    // later call is refused (as for a ground callback).
    assert_eq!(err.kind(), ErrorKind::Callback, "init keeps its own kind");
    assert_eq!(
        ctl.solve(&[]).unwrap_err().kind(),
        ErrorKind::Poisoned,
        "init poisons"
    );
    assert!(
        *earlier.lock().unwrap(),
        "registered before the failure: already ran"
    );
    assert!(
        !*later.lock().unwrap(),
        "registered after the failure: never runs"
    );
}

#[test]
fn a_panicking_inits_earlier_registered_sibling_already_ran_a_later_one_never_does() {
    let earlier = Arc::new(Mutex::new(false));
    let later = Arc::new(Mutex::new(false));
    let mut ctl = grounded("a.");
    ctl.register_propagator(Recorder(Arc::clone(&earlier)))
        .unwrap();
    ctl.register_propagator(PanicRaiser).unwrap();
    ctl.register_propagator(Recorder(Arc::clone(&later)))
        .unwrap();

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.solve(&[])));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(payload.downcast_ref::<&str>().copied(), Some("boom"));
    assert!(
        *earlier.lock().unwrap(),
        "registered before the panic: already ran"
    );
    assert!(
        !*later.lock().unwrap(),
        "registered after the panic: never runs"
    );

    // Recovery means a new `Control`: every later call on this one refuses.
    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

// ---------------------------------------------------------------------------
// number_of_threads matches the thread count the `Control` was built with.
//
// Oracle: `Control(['-t', str(t)])` for t in 1, 2, 4, 8 reported
// `init.number_of_threads == t` in every case.

struct RecordsThreadCount(Arc<Mutex<Option<u32>>>);
impl Propagator for RecordsThreadCount {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        *self.0.lock().unwrap() = Some(init.number_of_threads());
        Ok(())
    }
}

#[test]
fn number_of_threads_matches_the_configured_thread_count() {
    // A build without threads (the default WebAssembly target) runs one
    // solver thread only.
    let counts: &[u32] = if clingox_sys::HAS_THREADS {
        &[1, 2, 4, 8]
    } else {
        &[1]
    };
    for &threads in counts {
        let seen = Arc::new(Mutex::new(None));
        let mut ctl = Control::builder().threads(threads).build().unwrap();
        ctl.add_base("a.").unwrap();
        ctl.ground(&[Part::base()]).unwrap();
        ctl.register_propagator(RecordsThreadCount(Arc::clone(&seen)))
            .unwrap();
        let _ = ctl.solve(&[]).unwrap();
        assert_eq!(*seen.lock().unwrap(), Some(threads));
    }
}

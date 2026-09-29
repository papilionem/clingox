//! `Assignment`'s accessors: `decision_level`, `root_level`, `has_conflict`,
//! `size`, `is_total`, `has_literal`, `level`, `decision`, `is_fixed`,
//! `is_true`, `is_false`, `truth_value`, `at`. `Trail`'s own tests are in
//! `api_trail.rs`, named for its own subject.
//!
//! Every expected value below was checked directly against clingo 5.8.2
//! (the Python module `clingo`, 2026-09-28), several of them by reading
//! `clasp/src/clingo.cpp`/`clasp/libpotassco/src/clingo.cpp` directly
//! rather than trusting `clingo.h`'s prose alone.
//!
//! Captured readings store `Result<_, ErrorKind>` rather than
//! `clingox::Result<_>`: `Error` is neither `Clone` nor `PartialEq` (it
//! carries a `String` message and no derive), and every assertion here
//! only cares about the error kind, not its message.
//!
//! The fixture program below (`FIXTURE`) is used by most tests here and in
//! `api_trail.rs`, chosen so several confusable values differ at once
//! `decision_level() != root_level()` (4 vs 1); a positive
//! and a negative literal disagree (`a`'s literal is true, its negation is
//! false); a fixed, level-0 literal (`a`, a fact) and a genuinely decided
//! one both exist; and the trail's chronological order differs from
//! `size`/`at`'s ascending order (one literal is implied second
//! chronologically despite being numbered last, `api_trail.rs`).

#![forbid(unsafe_code)]
#![allow(
    clippy::type_complexity,
    reason = "fixtures group several readings into one value a callback can fill"
)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each propagator is defined next to the test that uses it"
)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use clingox::propagate::{CheckMode, PropagateControl, PropagateInit, Propagator, SolverLiteral};
use clingox::{Control, ErrorKind, Part, Result};

type R<T> = std::result::Result<T, ErrorKind>;

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a fresh control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// `a.` (a fact, unified by clasp's own preprocessing with `c`, an equivalent
/// atom), plus three independent choices and one constraint that forces
/// backtracking (checked directly): `decision_level() == 4`, `root_level() ==
/// 1` once the assignment is total, with `a`'s and `c`'s shared literal fixed
/// at level 0 and every other literal genuinely decided or implied above it.
const FIXTURE: &str = "a. c :- a. {b; d; e}. :- b, d.";

/// A `SolverLiteral` legitimately obtained from a much larger, separate
/// control's grounding: valid there, but unknown to a small control's own
/// assignment. Identical in shape to `api_propagator_init.rs`'s own helper
/// (each integration test file is its own compilation unit, so it cannot be
/// shared without a `tests/common` module this crate does not otherwise
/// have); used here to check that an *unknown* literal is a clean error on
/// every read accessor, never a crash, unlike `PropagateInit`'s own
/// mutating methods.
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

// ---------------------------------------------------------------------------
// The main correctness test: every accessor's value at the fixture's total
// assignment, cross-checked against the oracle.

struct Captured {
    decision_level: u32,
    root_level: u32,
    has_conflict: bool,
    is_total: bool,
    size: usize,
    // The positive literal enumeration, `at(0..size)`, always ascending by
    // construction (`clingo_assignment_at` is `offset + 1` in `control.cc`,
    // not a search-order property): `[lit_a, lit_b, lit_d, lit_e, aux]`
    // for this fixture, where `aux` is a fifth solver variable with no
    // named atom of its own (an auxiliary the constraint `:- b, d.`
    // introduces). `SolverLiteral` has no public raw constructor, so this
    // test never names any literal directly by number, only compares
    // publicly-obtained `SolverLiteral` values to one another.
    ascending: Vec<SolverLiteral>,
    // Per-literal readings for a, b, d, e (in that order, `ascending[0..4]`).
    is_true: [R<bool>; 4],
    is_false: [R<bool>; 4],
    is_fixed: [R<bool>; 4],
    truth_value: [R<Option<bool>>; 4],
    level: [R<Option<u32>>; 4],
    neg_a_is_true: R<bool>,
    neg_a_is_false: R<bool>,
    neg_a_truth_value: R<Option<bool>>,
    // decision(level) for level in 0..=decision_level, and
    // level(decision(level)).
    decisions: Vec<SolverLiteral>,
    decision_levels_round_trip: Vec<u32>,
}

struct Recorder(Arc<Mutex<Option<Captured>>>);
impl Propagator for Recorder {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        if !a.is_total() {
            return Ok(());
        }
        let ascending: Vec<SolverLiteral> = (0..a.size()).map(|i| a.at(i).unwrap()).collect();
        let lits = [ascending[0], ascending[1], ascending[2], ascending[3]];

        let is_true = lits.map(|l| a.is_true(l).map_err(|e| e.kind()));
        let is_false = lits.map(|l| a.is_false(l).map_err(|e| e.kind()));
        let is_fixed = lits.map(|l| a.is_fixed(l).map_err(|e| e.kind()));
        let truth_value = lits.map(|l| a.truth_value(l).map_err(|e| e.kind()));
        let level = lits.map(|l| a.level(l).map_err(|e| e.kind()));

        let decisions: Vec<SolverLiteral> = (0..=a.decision_level())
            .map(|lvl| a.decision(lvl).unwrap())
            .collect();
        // A decision literal is always assigned, so `level` never answers
        // `None` here; `level` itself now returns `Result<Option<u32>>`.
        let decision_levels_round_trip = decisions
            .iter()
            .map(|&lit| {
                a.level(lit)
                    .unwrap()
                    .expect("a decision literal is always assigned")
            })
            .collect();

        let neg_a = -lits[0];
        *self.0.lock().unwrap() = Some(Captured {
            decision_level: a.decision_level(),
            root_level: a.root_level(),
            has_conflict: a.has_conflict(),
            is_total: a.is_total(),
            size: a.size(),
            ascending,
            is_true,
            is_false,
            is_fixed,
            truth_value,
            level,
            neg_a_is_true: a.is_true(neg_a).map_err(|e| e.kind()),
            neg_a_is_false: a.is_false(neg_a).map_err(|e| e.kind()),
            neg_a_truth_value: a.truth_value(neg_a).map_err(|e| e.kind()),
            decisions,
            decision_levels_round_trip,
        });
        Ok(())
    }
}

#[test]
fn assignment_accessors_agree_with_the_python_oracle() {
    let captured = Arc::new(Mutex::new(None));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(Recorder(Arc::clone(&captured)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let c = captured
        .lock()
        .unwrap()
        .take()
        .expect("check() ran at least once, total");

    assert_eq!(c.decision_level, 4);
    assert_eq!(c.root_level, 1, "decision_level and root_level must differ");
    assert!(!c.has_conflict);
    assert!(c.is_total);
    assert_eq!(c.size, 5);
    assert_eq!(c.ascending.len(), 5);

    let lit_a = c.ascending[0];
    let lit_b = c.ascending[1];
    let lit_d = c.ascending[2];
    let lit_e = c.ascending[3];
    let aux = c.ascending[4];

    // a: a fact, fixed at level 0, true.
    assert_eq!(c.is_true[0], Ok(true), "a is true");
    assert_eq!(c.is_false[0], Ok(false));
    assert_eq!(c.is_fixed[0], Ok(true), "a is fixed: a level-0 fact");
    assert_eq!(c.truth_value[0], Ok(Some(true)));
    assert_eq!(c.level[0], Ok(Some(0)));

    // -a: the negation disagrees with a on every truth reading.
    assert_eq!(c.neg_a_is_true, Ok(false));
    assert_eq!(c.neg_a_is_false, Ok(true));
    assert_eq!(c.neg_a_truth_value, Ok(Some(false)));

    // b, d, e: all false in this model (the constraint ":- b, d." plus the
    // solver's own choices), none fixed (none is a level-0 fact).
    for i in 1..4 {
        assert_eq!(c.is_true[i], Ok(false), "b/d/e[{i}] is false in this model");
        assert_eq!(c.is_false[i], Ok(true));
        assert_eq!(c.is_fixed[i], Ok(false), "b/d/e[{i}] is decided, not fixed");
        assert_eq!(c.truth_value[i], Ok(Some(false)));
    }
    // b, d, e are assigned at three different levels (2, 3, 4): another
    // confusable pair (a level-shaped value) that must not collapse.
    assert_eq!(
        [c.level[1], c.level[2], c.level[3]],
        [Ok(Some(2)), Ok(Some(3)), Ok(Some(4))]
    );

    // decision(level) round-trips through level() for every level.
    assert_eq!(c.decisions.len(), 5);
    assert_eq!(c.decision_levels_round_trip, vec![0, 1, 2, 3, 4]);
    // decision(0) is always clasp's own "trivially true" sentinel, which
    // this fixture's own fact happens to share a literal with (confirmed
    // separately with a fact-free program too, where no atom shares it).
    assert_eq!(c.decisions[0], lit_a);
    // decision(1) is this fixture's auxiliary literal, not b's own (checked
    // directly: clasp's own heuristic decided the constraint's auxiliary
    // variable before any named atom).
    assert_eq!(c.decisions[1], aux);

    // lit_b/lit_d/lit_e/aux are pairwise distinct and none is lit_a.
    let all = [lit_a, lit_b, lit_d, lit_e, aux];
    for i in 0..all.len() {
        for j in (i + 1)..all.len() {
            assert_ne!(
                all[i], all[j],
                "ascending[{i}] and ascending[{j}] must differ"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// has_literal: true for a literal
// and its negation, false for one legitimately obtained from a different,
// larger control.

struct RecordsHasLiteral {
    foreign: SolverLiteral,
    out: Arc<Mutex<Option<(bool, bool, bool)>>>,
}
impl Propagator for RecordsHasLiteral {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        if !a.is_total() {
            return Ok(());
        }
        let lit_a = a.at(0).unwrap();
        *self.out.lock().unwrap() = Some((
            a.has_literal(lit_a),
            a.has_literal(-lit_a),
            a.has_literal(self.foreign),
        ));
        Ok(())
    }
}

#[test]
fn has_literal_is_true_in_range_false_for_a_foreign_literal() {
    let foreign = foreign_literal();
    let out = Arc::new(Mutex::new(None));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(RecordsHasLiteral {
        foreign,
        out: Arc::clone(&out),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let (has_a, has_neg_a, has_foreign) = out.lock().unwrap().take().expect("check() ran, total");
    assert!(has_a);
    assert!(
        has_neg_a,
        "a literal's negation is equally a member of the assignment"
    );
    assert!(
        !has_foreign,
        "a literal from a different, larger control is out of range here"
    );
}

// ---------------------------------------------------------------------------
// level() on a known but currently unassigned literal succeeds with None, never
// an error. Internally, clasp reports a u32::MAX sentinel for exactly this case
// (checked directly against clasp/src/clingo.cpp), which is why Option is the
// right wrap rather than a speculative one; clingox does not expose the raw
// sentinel.

struct RecordsFreeLevel(Arc<Mutex<Vec<(u32, R<Option<u32>>)>>>);
impl Propagator for RecordsFreeLevel {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Fixpoint);
        Ok(())
    }

    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        // b is always ascending offset 1 in this fixture (checked directly,
        // matching `assignment_accessors_agree_with_the_python_oracle`).
        let lit_b = a.at(1).unwrap();
        self.0
            .lock()
            .unwrap()
            .push((a.decision_level(), a.level(lit_b).map_err(|e| e.kind())));
        Ok(())
    }
}

#[test]
fn level_of_an_unassigned_known_literal_is_none_not_an_error() {
    let readings = Arc::new(Mutex::new(Vec::new()));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(RecordsFreeLevel(Arc::clone(&readings)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let readings = readings.lock().unwrap();
    assert!(!readings.is_empty(), "check() ran at least once");

    // At decision_level 0 (before b is assigned), level(b) succeeds and
    // reports None, not an error.
    let (first_level, first_reading) = readings[0];
    assert_eq!(first_level, 0);
    assert_eq!(first_reading, Ok(None));

    // By the time the assignment is total, b is assigned and level(b) is
    // Some of a real, small level.
    let (_, last) = *readings.last().unwrap();
    assert_eq!(last, Ok(Some(2)));
}

// ---------------------------------------------------------------------------
// truth_value is None while free, Some(false)/Some(true) once assigned: the
// three truth values, distinguished on the same literal across the search
// rather than on three different literals (a stronger claim: it is the
// same accessor changing its answer correctly as the search commits).

struct RecordsFreeTruthValue(Arc<Mutex<Vec<R<Option<bool>>>>>);
impl Propagator for RecordsFreeTruthValue {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Fixpoint);
        Ok(())
    }

    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        let lit_b = a.at(1).unwrap();
        self.0
            .lock()
            .unwrap()
            .push(a.truth_value(lit_b).map_err(|e| e.kind()));
        Ok(())
    }
}

#[test]
fn truth_value_is_none_while_free_then_settles() {
    let readings = Arc::new(Mutex::new(Vec::new()));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(RecordsFreeTruthValue(Arc::clone(&readings)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let readings = readings.lock().unwrap();
    assert_eq!(readings[0], Ok(None), "free before any decision touches b");
    assert_eq!(
        *readings.last().unwrap(),
        Ok(Some(false)),
        "settled to false by the end of this fixture's search (checked against the oracle)"
    );
}

// ---------------------------------------------------------------------------
// An unknown literal is refused with ErrorKind::InvalidInput on every read
// accessor, checked by clingox itself first, never reaching clingo (clingo's
// own Logic error is not let through unchanged).

struct RecordsForeignErrors {
    foreign: SolverLiteral,
    out: Arc<Mutex<Option<(R<Option<u32>>, R<bool>, R<bool>, R<bool>, R<Option<bool>>)>>>,
}
impl Propagator for RecordsForeignErrors {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        if !a.is_total() {
            return Ok(());
        }
        *self.out.lock().unwrap() = Some((
            a.level(self.foreign).map_err(|e| e.kind()),
            a.is_fixed(self.foreign).map_err(|e| e.kind()),
            a.is_true(self.foreign).map_err(|e| e.kind()),
            a.is_false(self.foreign).map_err(|e| e.kind()),
            a.truth_value(self.foreign).map_err(|e| e.kind()),
        ));
        Ok(())
    }
}

#[test]
fn unknown_literal_is_refused_with_invalid_input_on_every_read_accessor() {
    let foreign = foreign_literal();
    let out = Arc::new(Mutex::new(None));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(RecordsForeignErrors {
        foreign,
        out: Arc::clone(&out),
    })
    .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let (level, is_fixed, is_true, is_false, truth_value) =
        out.lock().unwrap().take().expect("check() ran, total");

    assert_eq!(level, Err(ErrorKind::InvalidInput));
    assert_eq!(is_fixed, Err(ErrorKind::InvalidInput));
    assert_eq!(is_true, Err(ErrorKind::InvalidInput));
    assert_eq!(is_false, Err(ErrorKind::InvalidInput));
    assert_eq!(truth_value, Err(ErrorKind::InvalidInput));
}

// ---------------------------------------------------------------------------
// Assignment::at and Assignment::decision both fail with
// ErrorKind::InvalidInput for an out-of-range offset/level, checked by clingox
// itself before calling clingo. Internally, clingo's own two functions fail
// differently (`at`'s own bounds check is a bare `std::runtime_error`,
// `Runtime`; `decision`'s is a `POTASSCO_REQUIRE`, `Logic`), but neither kind
// is observable through the public API any more: clingox's own check runs first
// and always produces `InvalidInput`.

struct RecordsOutOfRangeKinds(Arc<Mutex<Option<(ErrorKind, ErrorKind)>>>);
impl Propagator for RecordsOutOfRangeKinds {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        if !a.is_total() {
            return Ok(());
        }
        let at_kind = a.at(a.size()).unwrap_err().kind();
        let decision_kind = a.decision(a.decision_level() + 1).unwrap_err().kind();
        *self.0.lock().unwrap() = Some((at_kind, decision_kind));
        Ok(())
    }
}

#[test]
fn at_and_decision_out_of_range_are_both_invalid_input() {
    let out = Arc::new(Mutex::new(None));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(RecordsOutOfRangeKinds(Arc::clone(&out)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let (at_kind, decision_kind) = out.lock().unwrap().take().expect("check() ran, total");
    assert_eq!(
        at_kind,
        ErrorKind::InvalidInput,
        "Assignment::at: an out-of-range offset, checked by clingox itself"
    );
    assert_eq!(
        decision_kind,
        ErrorKind::InvalidInput,
        "Assignment::decision: an out-of-range level, checked by clingox itself"
    );

    // The literal-shaped case, separately, since it needs a foreign literal.
    let foreign = foreign_literal();
    let level_out = Arc::new(Mutex::new(None));
    struct RecordsLevelKind {
        foreign: SolverLiteral,
        out: Arc<Mutex<Option<ErrorKind>>>,
    }
    impl Propagator for RecordsLevelKind {
        fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
            let a = control.assignment();
            if !a.is_total() {
                return Ok(());
            }
            *self.out.lock().unwrap() = Some(a.level(self.foreign).unwrap_err().kind());
            Ok(())
        }
    }
    let mut ctl2 = grounded(FIXTURE);
    ctl2.register_propagator(RecordsLevelKind {
        foreign,
        out: Arc::clone(&level_out),
    })
    .unwrap();
    let _ = ctl2.solve(&[]).unwrap();
    assert_eq!(
        level_out
            .lock()
            .unwrap()
            .take()
            .expect("check() ran, total"),
        ErrorKind::InvalidInput,
        "Assignment::level: an unknown literal, checked by clingox itself"
    );
}

// ---------------------------------------------------------------------------
// is_total() is false mid-search and true once the assignment is total, on
// the same solve.

struct RecordsIsTotal(Arc<Mutex<Vec<bool>>>);
impl Propagator for RecordsIsTotal {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Both);
        Ok(())
    }

    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        self.0.lock().unwrap().push(control.assignment().is_total());
        Ok(())
    }
}

#[test]
fn is_total_is_false_mid_search_and_true_once_total() {
    let readings = Arc::new(Mutex::new(Vec::new()));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(RecordsIsTotal(Arc::clone(&readings)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let readings = readings.lock().unwrap();
    assert!(
        readings.len() > 1,
        "CheckMode::Both fires more than once for this fixture"
    );
    assert!(
        readings.iter().any(|&t| !t),
        "at least one mid-search reading is not total"
    );
    assert!(*readings.last().unwrap(), "the final reading is total");
}

// ---------------------------------------------------------------------------
// Each solver thread sees its own Assignment: a TSan-relevant smoke test
// more than a functional one (the deliverable's own "behaviour on threads"
// requirement), gated on `clingox_sys::HAS_THREADS` exactly like
// `api_propagator_registration.rs::number_of_threads_matches_the_
// configured_thread_count`. Behavioural assertions are deliberately
// minimal: the primary property under test is "reading `Assignment`
// concurrently from several solver threads does not race or crash," which
// `cargo xtask test sanitize` (TSan) is the real judge of; this test only
// checks it is possible to observe several threads' own views without a
// panic, and that each one's own `Assignment` is internally consistent.

struct RecordsPerThread(Arc<Mutex<HashSet<u32>>>);
impl Propagator for RecordsPerThread {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        // Self-consistency, not cross-thread comparison: every thread's own
        // assignment must satisfy its own invariants regardless of which
        // thread is running.
        assert!(usize::try_from(a.decision_level()).unwrap() <= a.size() + 1);
        self.0.lock().unwrap().insert(control.thread_id());
        Ok(())
    }
}

#[test]
fn each_thread_gets_its_own_assignment() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let seen = Arc::new(Mutex::new(HashSet::new()));
    let mut ctl = Control::builder().threads(4).build().unwrap();
    ctl.add_base("{p(1..40)}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(RecordsPerThread(Arc::clone(&seen)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let seen = seen.lock().unwrap();
    assert!(!seen.is_empty(), "check() ran on at least one thread");
    assert!(seen.iter().all(|&t| t < 4));
}

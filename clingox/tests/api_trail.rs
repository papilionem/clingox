//! `Trail`, in full: `size`, `begin`, `end`, `at`,
//! the `level(level)` convenience, and `impl IntoIterator for &Trail`.
//! `Assignment`'s own accessors are in `api_assignment.rs`,
//! named for its own subject.
//!
//! Uses the same `FIXTURE` program as `api_assignment.rs` (duplicated: each
//! integration test file is its own compilation unit, so it cannot be
//! shared without a `tests/common` module this crate does not otherwise
//! have), chosen so the trail's chronological order differs from
//! `Assignment::at`'s ascending order: a fifth solver literal (an auxiliary
//! variable the constraint `:- b, d.` introduces, with no named atom of its
//! own) is implied second chronologically despite being numbered last.
//!
//! Every expected value below was checked directly against clingo 5.8.2
//! and against `clasp/src/clingo.cpp`/`clasp/libpotassco/src/clingo.cpp`
//! (checked directly).

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

use std::sync::{Arc, Mutex};

use clingox::propagate::{PropagateControl, Propagator, SolverLiteral};
use clingox::{Control, ErrorKind, Part, Result};

type R<T> = std::result::Result<T, ErrorKind>;

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("a fresh control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// See `api_assignment.rs::FIXTURE` for the full reasoning.
const FIXTURE: &str = "a. c :- a. {b; d; e}. :- b, d.";

// ---------------------------------------------------------------------------
// The main cross-check: trail size, per-level begin/end/at, and how the
// trail relates to Assignment::decision (the first literal at each level,
// per clingo.h's own note: "the first literal with a larger level than the
// previous literals is a decision").

struct Captured {
    trail_size: R<u32>,
    assignment_size: usize,
    is_total: bool,
    // ascending[i] = Assignment::at(i): the fixture's five positive
    // literals in numeric order (a, b, d, e, and a nameless auxiliary).
    ascending: Vec<SolverLiteral>,
    // trail[i] = Trail::at(i): the same five literals' assigned (possibly
    // negative) forms, in chronological order.
    trail: Vec<SolverLiteral>,
    // (begin, end) for level in 0..=4.
    bounds: Vec<(R<u32>, R<u32>)>,
    // Assignment::decision(level) for level in 0..=4, for the
    // decision-is-the-first-trail-entry-at-its-level cross-check.
    decisions: Vec<SolverLiteral>,
}

struct Recorder(Arc<Mutex<Option<Captured>>>);
impl Propagator for Recorder {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        if !a.is_total() {
            return Ok(());
        }
        let t = a.trail();
        let ascending: Vec<SolverLiteral> = (0..a.size()).map(|i| a.at(i).unwrap()).collect();
        let trail_size = t.size().map_err(|e| e.kind());
        let n = t.size().unwrap();
        let trail: Vec<SolverLiteral> = (0..n).map(|i| t.at(i).unwrap()).collect();
        let bounds = (0..=a.decision_level())
            .map(|lvl| {
                (
                    t.begin(lvl).map_err(|e| e.kind()),
                    t.end(lvl).map_err(|e| e.kind()),
                )
            })
            .collect();
        let decisions = (0..=a.decision_level())
            .map(|lvl| a.decision(lvl).unwrap())
            .collect();

        *self.0.lock().unwrap() = Some(Captured {
            trail_size,
            assignment_size: a.size(),
            is_total: a.is_total(),
            ascending,
            trail,
            bounds,
            decisions,
        });
        Ok(())
    }
}

#[test]
fn trail_size_matches_assignment_size_once_total() {
    let captured = Arc::new(Mutex::new(None));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(Recorder(Arc::clone(&captured)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let c = captured.lock().unwrap().take().expect("check() ran, total");
    assert!(c.is_total);
    assert_eq!(c.trail_size, Ok(5));
    assert_eq!(c.trail_size, Ok(u32::try_from(c.assignment_size).unwrap()));
}

#[test]
fn trail_begin_end_bound_exactly_the_literals_at_each_level() {
    let captured = Arc::new(Mutex::new(None));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(Recorder(Arc::clone(&captured)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let c = captured.lock().unwrap().take().expect("check() ran, total");

    assert_eq!(c.bounds.len(), 5);
    let expected_bounds: Vec<(u32, u32)> = vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)];
    for (level, ((begin, end), (want_begin, want_end))) in
        c.bounds.iter().zip(expected_bounds.iter()).enumerate()
    {
        assert_eq!(*begin, Ok(*want_begin), "level {level}'s trail begin");
        assert_eq!(*end, Ok(*want_end), "level {level}'s trail end");
    }

    // Each level's own trail slice, read through the raw offsets, matches
    // the level's own decision at its first offset (clingo.h's own note:
    // "the first literal with a larger level than the previous literals is
    // a decision").
    for level in 0..c.decisions.len() {
        let (begin, _) = c.bounds[level];
        let begin = begin.unwrap();
        assert_eq!(
            c.trail[begin as usize], c.decisions[level],
            "level {level}'s first trail entry is its own decision literal"
        );
    }

    // The trail's signed forms: level 0 and level 1's literals are true
    // (positive, unnegated in the trail); levels 2, 3, 4's are false
    // (negative).
    assert_eq!(
        c.trail[0], c.ascending[0],
        "level 0: a's literal, positive (true)"
    );
    assert_eq!(
        c.trail[1], c.ascending[4],
        "level 1: the auxiliary literal, positive (true)"
    );
    assert_eq!(
        c.trail[2], -c.ascending[1],
        "level 2: b's literal, negative (false)"
    );
    assert_eq!(
        c.trail[3], -c.ascending[2],
        "level 3: d's literal, negative (false)"
    );
    assert_eq!(
        c.trail[4], -c.ascending[3],
        "level 4: e's literal, negative (false)"
    );
}

// ---------------------------------------------------------------------------
// Trail::level(level) (a convenience) matches the same slice
// built manually from begin/end/at.

#[test]
fn trail_level_convenience_matches_manual_begin_end_slicing() {
    let captured: Arc<Mutex<Option<Vec<(u32, Vec<SolverLiteral>, Vec<SolverLiteral>)>>>> =
        Arc::new(Mutex::new(None));
    struct Recorder2(Arc<Mutex<Option<Vec<(u32, Vec<SolverLiteral>, Vec<SolverLiteral>)>>>>);
    impl Propagator for Recorder2 {
        fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
            let a = control.assignment();
            if !a.is_total() {
                return Ok(());
            }
            let t = a.trail();
            let mut rows = Vec::new();
            for level in 0..=a.decision_level() {
                let via_convenience = t.level(level).unwrap();
                let (begin, end) = (t.begin(level).unwrap(), t.end(level).unwrap());
                let via_manual: Vec<SolverLiteral> =
                    (begin..end).map(|o| t.at(o).unwrap()).collect();
                rows.push((level, via_convenience, via_manual));
            }
            *self.0.lock().unwrap() = Some(rows);
            Ok(())
        }
    }
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(Recorder2(Arc::clone(&captured)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let rows = captured.lock().unwrap().take().expect("check() ran, total");
    assert_eq!(rows.len(), 5);
    for (level, via_convenience, via_manual) in rows {
        assert_eq!(
            via_convenience, via_manual,
            "level {level}: convenience vs manual slice"
        );
        assert_eq!(
            via_convenience.len(),
            1,
            "level {level}: exactly one literal in this fixture"
        );
    }
}

// ---------------------------------------------------------------------------
// impl IntoIterator for &Trail yields every literal, in chronological
// order, as Result<SolverLiteral> (Trail::at is itself fallible).

#[test]
fn trail_into_iterator_yields_every_literal_in_chronological_order() {
    let captured: Arc<Mutex<Option<Vec<SolverLiteral>>>> = Arc::new(Mutex::new(None));
    struct Recorder2(Arc<Mutex<Option<Vec<SolverLiteral>>>>);
    impl Propagator for Recorder2 {
        fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
            let a = control.assignment();
            if !a.is_total() {
                return Ok(());
            }
            let t = a.trail();
            let via_iterator: Vec<SolverLiteral> = (&t).into_iter().map(Result::unwrap).collect();
            *self.0.lock().unwrap() = Some(via_iterator);
            Ok(())
        }
    }
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(Recorder2(Arc::clone(&captured)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let via_iterator = captured.lock().unwrap().take().expect("check() ran, total");
    assert_eq!(via_iterator.len(), 5);

    // Cross-checked, on a fresh solve, against a manual per-offset read
    // (the main test above already pins the exact literal identities and
    // signs; this test's own job is only the iterator's own shape).
    let manual_captured: Arc<Mutex<Option<Vec<SolverLiteral>>>> = Arc::new(Mutex::new(None));
    struct ManualReader(Arc<Mutex<Option<Vec<SolverLiteral>>>>);
    impl Propagator for ManualReader {
        fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
            let a = control.assignment();
            if !a.is_total() {
                return Ok(());
            }
            let t = a.trail();
            let n = t.size().unwrap();
            let manual: Vec<SolverLiteral> = (0..n).map(|o| t.at(o).unwrap()).collect();
            *self.0.lock().unwrap() = Some(manual);
            Ok(())
        }
    }
    let mut ctl2 = grounded(FIXTURE);
    ctl2.register_propagator(ManualReader(Arc::clone(&manual_captured)))
        .unwrap();
    let _ = ctl2.solve(&[]).unwrap();
    let manual = manual_captured
        .lock()
        .unwrap()
        .take()
        .expect("check() ran, total");
    assert_eq!(manual, via_iterator);
}

// ---------------------------------------------------------------------------
// The trail's chronological order differs from Assignment's ascending
// (size/at) order: the auxiliary literal is implied second chronologically
// despite being numbered last.

#[test]
fn trail_chronological_order_differs_from_assignment_ascending_order() {
    let captured = Arc::new(Mutex::new(None));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(Recorder(Arc::clone(&captured)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let c = captured.lock().unwrap().take().expect("check() ran, total");

    // Normalize the trail's (possibly negative) literals to their positive
    // form using only public API (`is_positive`/`negate`), to compare
    // "which literal" ignoring truth value.
    let chrono_positive: Vec<SolverLiteral> = c
        .trail
        .iter()
        .map(|&l| if l.is_positive() { l } else { l.negate() })
        .collect();

    assert_ne!(
        chrono_positive, c.ascending,
        "chronological and ascending order must differ for this fixture"
    );
    // The precise claim: the literal implied second chronologically
    // (ascending[4], the auxiliary) is numbered last in ascending order,
    // not second.
    assert_eq!(chrono_positive[1], c.ascending[4]);
    assert_ne!(chrono_positive[1], c.ascending[1]);
}

// ---------------------------------------------------------------------------
// Trail::end and Trail::begin both fail with ErrorKind::InvalidInput for a
// level past the current decision level.
// Trail::end's own case is different in kind from every other validated
// accessor in this module: clasp's own `AbstractAssignment::trailEnd` has
// no bounds check at all for this case (confirmed directly,
// the oracle: `trail_end(level)` returns
// `trail_size()` unconditionally for any `level` at or above the current
// decision level, even an absurd one like `1_000_000`) — clingox invents
// this guard itself, for API uniformity, not because it mirrors an
// existing clasp check the way every other guard in this module does.
// `level == decision_level()` stays valid (the ordinary "end of the
// current level" case, still `Ok(trail_size())`).

struct RecordsEndVsBegin(Arc<Mutex<Option<(R<u32>, R<u32>, R<u32>, R<u32>)>>>);
impl Propagator for RecordsEndVsBegin {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        if !a.is_total() {
            return Ok(());
        }
        let t = a.trail();
        let current = a.decision_level();
        let past = current + 1;
        let far_past = 1_000_000;
        *self.0.lock().unwrap() = Some((
            t.end(current).map_err(|e| e.kind()),
            t.end(past).map_err(|e| e.kind()),
            t.end(far_past).map_err(|e| e.kind()),
            t.begin(past).map_err(|e| e.kind()),
        ));
        Ok(())
    }
}

#[test]
fn trail_end_is_invalid_input_past_the_current_decision_level_like_begin() {
    let out = Arc::new(Mutex::new(None));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(RecordsEndVsBegin(Arc::clone(&out)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let (end_current, end_past, end_far_past, begin_past) =
        out.lock().unwrap().take().expect("check() ran, total");

    assert_eq!(
        end_current,
        Ok(5),
        "end() at the current decision level is the ordinary, valid case"
    );
    assert_eq!(
        end_past,
        Err(ErrorKind::InvalidInput),
        "end() one level past decision_level is refused, checked by clingox itself \
         (clasp's own trailEnd has no bound here at all)"
    );
    assert_eq!(
        end_far_past,
        Err(ErrorKind::InvalidInput),
        "end() for an absurd level is refused the same way"
    );
    assert_eq!(
        begin_past,
        Err(ErrorKind::InvalidInput),
        "begin() rejects the identical out-of-range level, checked by clingox itself"
    );
}

// ---------------------------------------------------------------------------
// Trail::at fails with ErrorKind::InvalidInput for an out-of-range offset,
// checked by clingox itself first, exactly like Assignment::at
// (api_assignment.rs::at_and_decision_out_of_range_are_both_invalid_input),
// per the crate's one-error-kind rule for the whole propagator API.

struct RecordsTrailAtKind(Arc<Mutex<Option<ErrorKind>>>);
impl Propagator for RecordsTrailAtKind {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        if !a.is_total() {
            return Ok(());
        }
        let t = a.trail();
        let n = t.size().unwrap();
        *self.0.lock().unwrap() = Some(t.at(n).unwrap_err().kind());
        Ok(())
    }
}

#[test]
fn trail_at_out_of_range_is_invalid_input() {
    let out = Arc::new(Mutex::new(None));
    let mut ctl = grounded(FIXTURE);
    ctl.register_propagator(RecordsTrailAtKind(Arc::clone(&out)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    assert_eq!(
        out.lock().unwrap().take().expect("check() ran, total"),
        ErrorKind::InvalidInput
    );
}

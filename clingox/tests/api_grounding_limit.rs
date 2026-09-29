//! The grounding-size guard (DESIGN S15): `GroundingLimit` and the
//! `LimitedObserver<O>` combinator.
//!
//! The guard is entirely clingox's own construct (clingo has no equivalent
//! limit), so there is no oracle to check its counting scheme against; what
//! is checked here is the guard's documented behaviour.
//! `max_atoms`'s exact
//! formula: distinct atom ids reported by any callback, each
//! counted once, however many callbacks mention it.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use clingox::backend::Atom;
use clingox::observer::{GroundProgramObserver, GroundingLimit, LimitedObserver};
use clingox::{Control, ErrorKind, Part, ProgramLiteral};

/// An observer that only counts how many times each callback kind ran, to
/// check composability without
/// depending on the exact accounting `LimitedObserver` uses internally.
#[derive(Clone, Default)]
struct Counts {
    rules: Arc<AtomicUsize>,
    output_atoms: Arc<AtomicUsize>,
}

impl GroundProgramObserver for Counts {
    fn rule(
        &mut self,
        _choice: bool,
        _head: &[Atom],
        _body: &[ProgramLiteral],
    ) -> clingox::Result<()> {
        self.rules.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn output_atom(
        &mut self,
        _symbol: clingox::Symbol,
        _atom: Option<Atom>,
    ) -> clingox::Result<()> {
        self.output_atoms.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn facts(n: usize) -> String {
    (b'a'..=b'z')
        .take(n)
        .map(|c| format!("{}.", c as char))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// max_rules: the exact boundary

#[test]
fn a_program_at_the_rule_limit_grounds_and_solves_fine() {
    let mut ctl = Control::new().unwrap();
    let limit = GroundingLimit::new(None, Some(1));
    ctl.register_observer(LimitedObserver::new(Counts::default(), limit), false)
        .unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_program_over_the_rule_limit_fails_grounding_with_grounding_limit() {
    let mut ctl = Control::new().unwrap();
    let limit = GroundingLimit::new(None, Some(1));
    ctl.register_observer(LimitedObserver::new(Counts::default(), limit), false)
        .unwrap();
    ctl.add_base("a. b.").unwrap();
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::GroundingLimit);
}

/// The guard counts a callback before letting it through, so with
/// the guard counts a callback before letting it through, so with
/// `max_rules: Some(1)` and two one-rule facts the second `rule` call is
/// refused before it reaches the wrapped observer, and no later callback
/// (`output_atom`) is observed either.
#[test]
fn the_callback_that_exceeds_the_limit_never_reaches_the_wrapped_observer() {
    let mut ctl = Control::new().unwrap();
    let counts = Counts::default();
    let limit = GroundingLimit::new(None, Some(1));
    ctl.register_observer(LimitedObserver::new(counts.clone(), limit), false)
        .unwrap();
    ctl.add_base("a. b.").unwrap();
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::GroundingLimit);
    // The guard counts a callback before checking it,
    // so the rule that would exceed the limit is refused and never reaches
    // the wrapped observer.
    assert_eq!(
        counts.rules.load(Ordering::SeqCst),
        1,
        "only the rule within the limit is observed"
    );
    assert_eq!(
        counts.output_atoms.load(Ordering::SeqCst),
        0,
        "the callback after the limit is exceeded is refused before it reaches the wrapped observer"
    );
}

// ---------------------------------------------------------------------------
// max_atoms: an exact boundary, and two coarse-grained sanity checks

/// The exact boundary, mirroring the
/// `max_rules` pair above. The fixture is an `observer.lp`-derived
/// program (see `api_observer.rs`'s tests): grounding it uses atoms `1` through
/// `7`, seven distinct ids, counted by hand across
/// every callback that carries one (`rule`'s heads and bodies, `external`,
/// `heuristic`, `acyc_edge`'s condition, `project`, `weight_rule`,
/// `minimize`, `output_atom`, `output_term`'s condition), per the counting
/// formula: atom `1` in particular is reported only once, as a body literal
/// of the very first rule, and nowhere else, which is exactly the case a
/// heads-only or `output_atom`-only count would miss.
const SEVEN_ATOM_PROGRAM: &str = "
1 {a; b}.
#minimize {1:a; 2:b}.
#project a.
#show x : a, b.
#external a.
#heuristic a : b. [1@2,sign]
#edge (a,b) : a, b.

#theory t {
  term   { + : 1, binary, left };
  &a/0 : term, any;
  &b/1 : term, {=}, term, any
}.
a :- &a { 1+2,\"test\": a, b }.
b :- &b(3) { } = 17.
";

#[test]
fn a_program_with_seven_distinct_atoms_is_at_a_limit_of_seven() {
    let mut ctl = Control::new().unwrap();
    let limit = GroundingLimit::new(Some(7), None);
    ctl.register_observer(LimitedObserver::new(Counts::default(), limit), false)
        .unwrap();
    ctl.add_base(SEVEN_ATOM_PROGRAM).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_program_with_seven_distinct_atoms_exceeds_a_limit_of_six() {
    let mut ctl = Control::new().unwrap();
    let limit = GroundingLimit::new(Some(6), None);
    ctl.register_observer(LimitedObserver::new(Counts::default(), limit), false)
        .unwrap();
    ctl.add_base(SEVEN_ATOM_PROGRAM).unwrap();
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::GroundingLimit);
}

#[test]
fn a_program_well_under_the_atom_limit_grounds_fine() {
    let mut ctl = Control::new().unwrap();
    let limit = GroundingLimit::new(Some(10), None);
    ctl.register_observer(LimitedObserver::new(Counts::default(), limit), false)
        .unwrap();
    ctl.add_base(&facts(2)).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_program_well_over_the_atom_limit_fails_grounding_with_grounding_limit() {
    let mut ctl = Control::new().unwrap();
    let limit = GroundingLimit::new(Some(5), None);
    ctl.register_observer(LimitedObserver::new(Counts::default(), limit), false)
        .unwrap();
    ctl.add_base(&facts(20)).unwrap();
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::GroundingLimit);
}

/// Negative control #5, the rules half: a limit on atoms alone never fires
/// for a program with many rules but few atoms sharing them.
#[test]
fn a_rule_only_limit_does_not_fire_on_atom_count_alone() {
    let mut ctl = Control::new().unwrap();
    let limit = GroundingLimit::new(None, Some(1_000));
    ctl.register_observer(LimitedObserver::new(Counts::default(), limit), false)
        .unwrap();
    ctl.add_base(&facts(20)).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

// ---------------------------------------------------------------------------
// Composition: the guard and a user's own observer both see the same calls

#[test]
fn the_guard_composes_with_a_users_own_observer_both_see_the_same_callbacks() {
    let mut ctl = Control::new().unwrap();
    let counts = Counts::default();
    let limit = GroundingLimit::new(Some(100), Some(100));
    ctl.register_observer(LimitedObserver::new(counts.clone(), limit), false)
        .unwrap();
    ctl.add_base("a. b.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(counts.rules.load(Ordering::SeqCst), 2);
    assert_eq!(counts.output_atoms.load(Ordering::SeqCst), 2);
}

// ---------------------------------------------------------------------------
// The control's state after a grounding-limit error: poisoned, like every other
// error that stops grounding partway (reversing DESIGN S3 for grounding):
// clingo keeps whatever it already ground before the guard's `Err` fired, and
// would answer from that truncated program silently.

#[test]
fn a_grounding_limit_error_poisons_the_control() {
    let mut ctl = Control::new().unwrap();
    let limit = GroundingLimit::new(None, Some(1));
    ctl.register_observer(LimitedObserver::new(Counts::default(), limit), false)
        .unwrap();
    ctl.add_base("a. b.").unwrap();
    ctl.add("next", &[], "c.").unwrap();
    ctl.ground(&[Part::base()]).unwrap_err();
    assert!(format!("{ctl:?}").contains("poisoned"));
    let err = ctl.ground(&[Part::new("next", &[]).unwrap()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

// ---------------------------------------------------------------------------
// `ground_with_limit`: the convenience entry point, for the no-user-observer
// case.

#[test]
fn ground_with_limit_is_a_convenience_that_needs_no_observer_of_its_own() {
    let mut ctl = Control::new().unwrap();
    let limit = GroundingLimit::new(None, Some(1));
    ctl.add_base("a.").unwrap();
    ctl.ground_with_limit(&[Part::base()], limit).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn ground_with_limit_reports_grounding_limit_over_the_bound() {
    let mut ctl = Control::new().unwrap();
    let limit = GroundingLimit::new(None, Some(1));
    ctl.add_base("a. b.").unwrap();
    let err = ctl.ground_with_limit(&[Part::base()], limit).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::GroundingLimit);
}

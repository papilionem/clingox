//! Regression test: the grounding-size guard's `check()` looked at the counts
//! as they stood *before* the current callback's own contribution, so the
//! callback that pushed a count over its limit was never itself refused, only
//! the next one -- and when that pushing callback was the last one of the
//! grounding, there was no next callback to refuse. `ground_with_limit` then
//! returned `Ok`, and the limit violation only surfaced as a poisoning error on
//! the *next* unrelated call (decided behaviour 9: "`ground_with_limit` counts
//! before it checks, so exceeding the limit is reported by `ground_with_limit`
//! itself").

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use clingox::observer::GroundingLimit;
use clingox::{Control, ErrorKind, Part};

/// `a :- not b. b :- not a. #show.` grounds two rules (`a :- not b.` and
/// `b :- not a.`); a `max_rules` of 1 is exceeded by the second one, the
/// last callback of the grounding.
#[test]
fn ground_with_limit_over_the_rule_limit_on_the_last_callback_fails_at_once() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- not b. b :- not a. #show.").unwrap();
    let limit = GroundingLimit::new(None, Some(1));

    let grounded = ctl.ground_with_limit(&[Part::base()], limit);
    assert_eq!(
        grounded.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::GroundingLimit),
        "ground_with_limit must itself report exceeding the limit, not a later call: {grounded:?}"
    );

    // The control is poisoned by the limit violation itself: a truncated
    // grounding must not be usable silently.
    assert!(
        format!("{ctl:?}").contains("poisoned"),
        "a limit violation must poison the control at once: {ctl:?}"
    );
}

/// The same limit on `max_atoms`, where the last callback's own atom (not a
/// rule) pushes the count over.
#[test]
fn ground_with_limit_over_the_atom_limit_on_the_last_callback_fails_at_once() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a. b.").unwrap();
    let limit = GroundingLimit::new(Some(1), None);

    let grounded = ctl.ground_with_limit(&[Part::base()], limit);
    assert_eq!(
        grounded.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::GroundingLimit),
        "ground_with_limit must itself report exceeding the limit, not a later call: {grounded:?}"
    );
    assert!(format!("{ctl:?}").contains("poisoned"), "{ctl:?}");
}

/// A grounding that stays within both limits must still succeed, and
/// nothing after it must be poisoned: the fix must not turn the "next
/// callback" case into a false positive on the callback that only reaches
/// the limit exactly.
#[test]
fn ground_with_limit_exactly_at_the_limit_succeeds() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a. b.").unwrap();
    let limit = GroundingLimit::new(Some(2), None);
    ctl.ground_with_limit(&[Part::base()], limit).unwrap();
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

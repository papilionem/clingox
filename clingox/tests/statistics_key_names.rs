//! Regression test: `MutableStatistics::add_map_key` accepted a subkey name
//! that was empty or contained a `.` (the path separator), building an entry
//! `Statistics`'s own dot-separated path syntax can never address again;
//! reading it back through `Statistics::snapshot` (which walks every entry by
//! that same syntax) then failed with `ErrorKind::Unknown`, which poisons the
//! control. `add_map_key` rejects such a name up front with
//! `ErrorKind::InvalidInput` (which does not poison), and a statistics lookup
//! miss during `snapshot` is `ErrorKind::Runtime` (which does not poison
//! either), not `Unknown`.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::float_cmp,
    reason = "the value compared exactly is the one this test itself wrote"
)]

use std::ops::ControlFlow;

use clingox::{
    Control, ErrorKind, MutableStatistics, Part, SolveEventHandler, SolveOptions, StatKind,
};

fn poisoned(ctl: &Control) -> bool {
    format!("{ctl:?}").contains("poisoned")
}

/// Tries `add_map_key("", name, StatKind::Value)` inside a solve-event
/// handler and reports what it returned.
struct TryAddKey<'a> {
    name: &'a str,
    result: &'a mut Option<clingox::Result<()>>,
}

impl SolveEventHandler for TryAddKey<'_> {
    fn on_statistics(
        &mut self,
        step: &mut MutableStatistics<'_>,
        _accumulated: &mut MutableStatistics<'_>,
    ) -> clingox::Result<ControlFlow<()>> {
        *self.result = Some(step.add_map_key("", self.name, StatKind::Value));
        Ok(ControlFlow::Continue(()))
    }
}

fn try_add_key(name: &str) -> (clingox::Result<()>, bool) {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut result = None;
    let _ = ctl
        .solve_with_events(
            SolveOptions::new(),
            TryAddKey {
                name,
                result: &mut result,
            },
        )
        .unwrap();
    (result.expect("on_statistics ran"), poisoned(&ctl))
}

#[test]
fn add_map_key_rejects_a_name_containing_a_dot() {
    let (result, poisoned) = try_add_key("a.b");
    assert_eq!(
        result.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::InvalidInput),
        "a name containing the path separator must be rejected up front: {result:?}"
    );
    assert!(!poisoned, "a rejected name must not poison the control");
}

#[test]
fn add_map_key_rejects_an_empty_name() {
    let (result, poisoned) = try_add_key("");
    assert_eq!(
        result.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::InvalidInput),
        "an empty name must be rejected up front: {result:?}"
    );
    assert!(!poisoned);
}

/// An ordinary name (no dot, not empty) must still be accepted, and the tree
/// it builds must be readable back through `Statistics::snapshot` without
/// poisoning: this is the case that broke silently before, and the
/// direct regression for the fix.
#[test]
fn a_plain_name_is_still_accepted_and_survives_a_snapshot() {
    struct AddPlainKey;
    impl SolveEventHandler for AddPlainKey {
        fn on_statistics(
            &mut self,
            step: &mut MutableStatistics<'_>,
            _accumulated: &mut MutableStatistics<'_>,
        ) -> clingox::Result<ControlFlow<()>> {
            step.add_map_key("", "mine", StatKind::Value)?;
            step.set_value("mine", 1.0)?;
            Ok(ControlFlow::Continue(()))
        }
    }
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let _ = ctl
        .solve_with_events(SolveOptions::new(), AddPlainKey)
        .unwrap();
    assert!(!poisoned(&ctl));

    let stats = ctl.statistics().unwrap();
    assert_eq!(stats.value("user_step.mine").unwrap(), 1.0);
    let snapshot = stats.snapshot();
    assert!(
        snapshot.is_ok(),
        "a tree built only from accepted names must snapshot cleanly: {snapshot:?}"
    );
    assert!(!poisoned(&ctl));
}

/// A plain lookup miss (a path that was never created) must be a
/// non-poisoning `Runtime` error, not `Unknown`, whether read directly or
/// discovered while walking the whole tree in `snapshot`.
#[test]
fn a_statistics_lookup_miss_is_runtime_not_poisoning() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let _ = ctl.solve(&[]).unwrap();

    let stats = ctl.statistics().unwrap();
    let missing = stats.value("user_step.does_not_exist");
    assert_eq!(
        missing.as_ref().map_err(clingox::Error::kind),
        Err(ErrorKind::Runtime)
    );
    assert!(!poisoned(&ctl), "a lookup miss must not poison: {ctl:?}");

    let snapshot = stats.snapshot();
    assert!(snapshot.is_ok(), "{snapshot:?}");
    assert!(!poisoned(&ctl));
}

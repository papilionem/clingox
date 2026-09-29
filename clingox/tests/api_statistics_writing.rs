//! `MutableStatistics`'s write side: `set_value`, `push_array`, `add_map_key`,
//! reached through `SolveEventHandler::on_statistics`.
//!
//! Every expected value is checked directly against clingo 5.8.2's C-level
//! statistics functions (`clingo._internal._lib.clingo_statistics_*`, bypassing
//! pyclingo's own `StatisticsMap.__setitem__` sugar, which builds a whole
//! subtree from one Python assignment and supports an "update function"
//! convenience `MutableStatistics` does not have): `add_map_key`/
//! `push_array`/`value_set` calls one at a time, exactly as this test drives
//! them, produce `user_step.test = {a: 1.0, b: [10.0, 20.0], c: {d: 3.0}}`,
//! confirmed with the installed clingo 5.8.2 (checked directly). This is a
//! primitive-only equivalent of `test_conf.py::test_user_stats`, not a
//! byte-for-byte port: that fixture's own values also exercise pyclingo's
//! update-function sugar, which `MutableStatistics` does not expose (a
//! discrepancy in the fixture's coverage, not a bug).
//!
//! U19 (`UPSTREAM-ISSUES.md`): clasp's statistics registry was unsafe to extend
//! concurrently before patch U19, and user statistics are the first feature
//! that calls the same registration path (`clingo_statistics_map_add_subkey`/
//! `array_push`) for *user*-defined statistics rather than clasp's own kinds.
//! The multi-thread tests here are the required evidence that the patch also
//! covers this new caller, not only clasp's internal use of the same registry
//! (`patch_u19_statistics_registry.rs` already covers that half); run them
//! under `ThreadSanitizer`.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::float_cmp,
    reason = "the values compared exactly are the ones the test wrote"
)]
#![allow(
    clippy::items_after_statements,
    reason = "each handler is defined next to the test that uses it"
)]

use std::ops::ControlFlow;
use std::sync::Barrier;
use std::thread;

use clingox::{Control, ErrorKind, MutableStatistics, Part, Result as ClingoxResult, StatKind};

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).unwrap();
    ctl.add_base(program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl
}

/// Builds `user_step.test = {a: 1.0, b: [10.0, 20.0], c: {d: 3.0}}` and the
/// same tree under `user_accu.test`, one primitive call at a time, matching
/// the oracle transcript in the file-level doc comment.
struct BuildUserStats;

fn build_test_tree(stats: &mut MutableStatistics<'_>) -> ClingoxResult<()> {
    stats.add_map_key("", "test", StatKind::Map)?;
    stats.add_map_key("test", "a", StatKind::Value)?;
    stats.set_value("test.a", 1.0)?;
    stats.add_map_key("test", "b", StatKind::Array)?;
    // `push_array` returns the new element's index; writing through it
    // fails if the index is wrong.
    let first = stats.push_array("test.b", StatKind::Value)?;
    stats.set_value(&format!("test.b.{first}"), 10.0)?;
    let second = stats.push_array("test.b", StatKind::Value)?;
    stats.set_value(&format!("test.b.{second}"), 20.0)?;
    stats.add_map_key("test", "c", StatKind::Map)?;
    stats.add_map_key("test.c", "d", StatKind::Value)?;
    stats.set_value("test.c.d", 3.0)?;
    Ok(())
}

impl clingox::SolveEventHandler for BuildUserStats {
    fn on_statistics(
        &mut self,
        step: &mut MutableStatistics<'_>,
        accumulated: &mut MutableStatistics<'_>,
    ) -> ClingoxResult<ControlFlow<()>> {
        build_test_tree(step)?;
        build_test_tree(accumulated)?;
        Ok(ControlFlow::Continue(()))
    }
}

#[test]
fn set_value_push_array_and_add_map_key_build_a_tree_visible_after_the_solve() {
    let mut ctl = grounded(&["--stats=2"], "{a;b}.");
    let result = ctl
        .solve_with_events(clingox::SolveOptions::new(), BuildUserStats)
        .unwrap();
    assert!(result.is_sat());

    let stats = ctl.statistics().unwrap();
    for root in ["user_step", "user_accu"] {
        assert_eq!(
            stats.value(&format!("{root}.test.a")).unwrap(),
            1.0,
            "{root}"
        );
        assert_eq!(
            stats.keys(&format!("{root}.test")).unwrap(),
            vec!["a", "b", "c"],
            "{root}"
        );
        assert_eq!(
            stats.value(&format!("{root}.test.b.0")).unwrap(),
            10.0,
            "{root}"
        );
        assert_eq!(
            stats.value(&format!("{root}.test.b.1")).unwrap(),
            20.0,
            "{root}"
        );
        assert_eq!(
            stats.value(&format!("{root}.test.c.d")).unwrap(),
            3.0,
            "{root}"
        );
    }
}

// ---------------------------------------------------------------------------
// Writing the wrong kind of entry is a runtime error that does not poison,
// mirroring the read side's own
// `reading_a_map_as_a_value_is_an_error_that_does_not_poison` (checked at
// the C level: `clingo_statistics_value_set` on a map-typed key returns a
// *logic* error, `type error`, which would poison if clingox let it
// through unchecked, per S1/S3; `MutableStatistics` must reject it first).
// ---------------------------------------------------------------------------

#[test]
fn setting_a_value_on_a_map_entry_is_a_runtime_error_that_does_not_poison() {
    let seen: std::sync::Arc<std::sync::Mutex<Option<clingox::Error>>> = std::sync::Arc::default();

    struct Shared(std::sync::Arc<std::sync::Mutex<Option<clingox::Error>>>);
    impl clingox::SolveEventHandler for Shared {
        fn on_statistics(
            &mut self,
            step: &mut MutableStatistics<'_>,
            _accumulated: &mut MutableStatistics<'_>,
        ) -> ClingoxResult<ControlFlow<()>> {
            step.add_map_key("", "test", StatKind::Map)?;
            *self.0.lock().unwrap() = step.set_value("test", 5.0).err();
            Ok(ControlFlow::Continue(()))
        }
    }

    let mut ctl = grounded(&["--stats=2"], "a.");
    assert!(
        ctl.solve_with_events(clingox::SolveOptions::new(), Shared(seen.clone()))
            .unwrap()
            .is_sat()
    );
    let err = seen
        .lock()
        .unwrap()
        .take()
        .expect("set_value on a map fails");
    assert_eq!(err.kind(), ErrorKind::Runtime);
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn pushing_to_a_value_entry_is_a_runtime_error() {
    struct PushOnValue;
    impl clingox::SolveEventHandler for PushOnValue {
        fn on_statistics(
            &mut self,
            step: &mut MutableStatistics<'_>,
            _accumulated: &mut MutableStatistics<'_>,
        ) -> ClingoxResult<ControlFlow<()>> {
            step.add_map_key("", "solo", StatKind::Value)?;
            step.set_value("solo", 1.0)?;
            let err = step.push_array("solo", StatKind::Value).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Runtime);
            Ok(ControlFlow::Continue(()))
        }
    }

    let mut ctl = grounded(&["--stats=2"], "a.");
    assert!(
        ctl.solve_with_events(clingox::SolveOptions::new(), PushOnValue)
            .unwrap()
            .is_sat()
    );
}

// ---------------------------------------------------------------------------
// U19: registering user statistics from several solver threads at once.
// `on_statistics` fires on the caller's thread for a blocking solve (no
// cross-thread question there); with several solver threads and a plain
// `--stats` build this is the path that shares clasp's statistics registry
// the way clasp's own kinds already did before U19. Functional value
// checks are the same as the single-threaded test above; the real
// evidence is a clean `cargo xtask sanitize` run, not these assertions.
// ---------------------------------------------------------------------------

fn build_user_stats_with_threads(threads: usize) {
    let mut ctl = Control::builder()
        .threads(u32::try_from(threads).unwrap())
        .args(["--stats=2", "--models=0"])
        .build()
        .unwrap();
    ctl.add_base("{a;b;c}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let result = ctl
        .solve_with_events(clingox::SolveOptions::new(), BuildUserStats)
        .unwrap();
    assert!(result.is_exhausted());
    let stats = ctl.statistics().unwrap();
    assert_eq!(stats.value("user_accu.test.a").unwrap(), 1.0);
    assert_eq!(stats.value("user_accu.test.c.d").unwrap(), 3.0);
}

#[test]
fn registering_user_statistics_with_one_solver_thread() {
    build_user_stats_with_threads(1);
}

#[test]
fn registering_user_statistics_with_two_solver_threads() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    build_user_stats_with_threads(2);
}

#[test]
fn registering_user_statistics_with_eight_solver_threads() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    build_user_stats_with_threads(8);
}

/// Several controls, each with several solver threads, all building and
/// reading `user_step`/`user_accu` statistics at once: the same "many
/// threads register a new statistics kind for the first time simultaneously"
/// shape `patch_u19_statistics_registry.rs` already exercises for clasp's
/// own kinds, repeated here for the map/array/value subkeys
/// `MutableStatistics`'s write functions register (`clingo_statistics_map_
/// add_subkey`/`array_push`, the exact functions U19 patches). This needs
/// `ThreadSanitizer` to be meaningful; functionally it only checks that every
/// thread's own tree is intact afterward.
#[test]
fn many_controls_register_user_statistics_from_many_threads_at_once() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    const WORKERS: usize = 8;
    let start = Barrier::new(WORKERS);
    thread::scope(|scope| {
        for worker in 0..WORKERS {
            let start = &start;
            scope.spawn(move || {
                let threads = 1 + (worker % 3);
                let mut ctl = Control::builder()
                    .threads(u32::try_from(threads).unwrap())
                    .args(["--stats=2"])
                    .build()
                    .unwrap();
                ctl.add_base("{a;b}.").unwrap();
                ctl.ground(&[Part::base()]).unwrap();
                start.wait();
                let result = ctl
                    .solve_with_events(clingox::SolveOptions::new(), BuildUserStats)
                    .unwrap();
                assert!(result.is_sat());
                let stats = ctl.statistics().unwrap();
                assert_eq!(stats.value("user_step.test.a").unwrap(), 1.0);
            });
        }
    });
}

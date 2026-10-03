//! Reading clingo's statistics by path, and owned snapshots of them.
//!
//! Keys and values were checked against clingo 5.8.2 through its C functions
//! (`clingo_statistics_*`, reached from the Python module): pyclingo's own
//! statistics view showed the values of the previous step. clingo reports an
//! unknown key and reading a map as a value as *logic* errors, which would
//! poison; clingox checks the path first and reports a runtime error instead.
//!
//! On WASM, CPU times are 0: Emscripten has no real `getrusage`, and the
//! vendored build does not call it (U14). No test expects a positive CPU time.

#![forbid(unsafe_code)]
#![allow(
    clippy::float_cmp,
    reason = "the values compared exactly are counts, which an f64 holds exactly"
)]

use clingox::{Control, ErrorKind, Part, StatsTree};

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// `{a;b}.` solved once with `--models=0`: four models, two atoms, one rule.
fn solved_four_models() -> Control {
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    let result = ctl.solve(&[]).expect("the program solves");
    assert!(result.is_exhausted());
    ctl
}

/// An optimisation example: clingo 5.8.2 finds two models,
/// the second optimal with cost 0.
const OPTIMISATION: &str = "{ a; b; c }. :~ a. [1] :~ b. [2] :~ not c. [3]";

fn value(ctl: &Control, path: &str) -> f64 {
    ctl.statistics()
        .expect("statistics can be read")
        .value(path)
        .expect("the path names a value")
}

#[test]
fn value_reads_the_summary_by_path() {
    let ctl = solved_four_models();
    assert_eq!(value(&ctl, "summary.models.enumerated"), 4.0);
    assert_eq!(value(&ctl, "summary.models.optimal"), 0.0);
    assert_eq!(value(&ctl, "summary.exhausted"), 1.0);
    assert_eq!(value(&ctl, "summary.result"), 1.0);
    assert_eq!(value(&ctl, "summary.call"), 0.0, "calls count from 0");
}

#[test]
fn value_reads_the_problem_size() {
    let ctl = solved_four_models();
    assert_eq!(value(&ctl, "problem.lp.atoms"), 2.0);
    assert_eq!(value(&ctl, "problem.lp.rules"), 1.0);
}

#[test]
fn times_are_durations_after_a_solve() {
    // Before the first solve `summary.times.total` holds a wall-clock
    // timestamp; after it, a duration in seconds.
    let ctl = solved_four_models();
    let total = value(&ctl, "summary.times.total");
    assert!(
        total.is_finite() && (0.0..3600.0).contains(&total),
        "{total}"
    );
    let cpu = value(&ctl, "summary.times.cpu");
    assert!(cpu.is_finite() && cpu >= 0.0, "{cpu}");
}

/// The documented WebAssembly value (U14): exactly 0, with the patch (clasp
/// does not read the clock there) and without it (Emscripten's stub returns the
/// same time on every call, and the value is a difference of two readings).
/// This pins the behaviour; `cargo xtask test wasm` is what checks the patch.
#[cfg(target_os = "emscripten")]
#[test]
fn cpu_time_is_zero_on_webassembly() {
    let ctl = solved_four_models();
    assert_eq!(value(&ctl, "summary.times.cpu"), 0.0);
}

#[test]
fn statistics_follow_each_solve_call() {
    let mut ctl = solved_four_models();
    let assumption = (clingox::Symbol::function("a", &[]).unwrap(), true);
    assert!(ctl.solve(&[assumption.into()]).unwrap().is_exhausted());
    assert_eq!(value(&ctl, "summary.call"), 1.0);
    assert_eq!(value(&ctl, "summary.models.enumerated"), 2.0);
}

#[test]
fn array_elements_are_read_by_index() {
    let mut ctl = grounded(&[], OPTIMISATION);
    assert!(ctl.solve(&[]).unwrap().is_sat());
    assert_eq!(value(&ctl, "summary.costs.0"), 0.0);
    assert_eq!(value(&ctl, "summary.models.enumerated"), 2.0);
    assert_eq!(value(&ctl, "summary.models.optimal"), 1.0);
    let stats = ctl.statistics().unwrap();
    assert_eq!(stats.keys("summary.costs").unwrap(), vec!["0"]);
}

#[test]
fn keys_list_maps_in_clingo_order() {
    let ctl = solved_four_models();
    let stats = ctl.statistics().unwrap();
    assert_eq!(
        stats.keys("").unwrap(),
        vec!["problem", "solving", "summary", "user_step", "user_accu"]
    );
    assert_eq!(
        stats.keys("summary").unwrap(),
        vec![
            "call",
            "result",
            "signal",
            "exhausted",
            "costs",
            "lower",
            "concurrency",
            "winner",
            "times",
            "models",
        ]
    );
    assert_eq!(
        stats.keys("summary.models").unwrap(),
        vec!["enumerated", "optimal"]
    );
    assert!(
        stats.keys("summary.costs").unwrap().is_empty(),
        "no optimisation, no costs"
    );
    assert!(
        stats.keys("summary.call").unwrap().is_empty(),
        "a value has no keys"
    );
}

#[test]
fn options_change_the_keys() {
    let mut ctl = grounded(&["--stats"], "a.");
    assert!(ctl.solve(&[]).unwrap().is_sat());
    let stats = ctl.statistics().unwrap();
    assert_eq!(
        stats.keys("").unwrap(),
        vec![
            "problem",
            "solving",
            "summary",
            "accu",
            "user_step",
            "user_accu"
        ]
    );
}

#[test]
fn statistics_can_be_read_before_solving() {
    let ctl = grounded(&[], "a.");
    let stats = ctl.statistics().unwrap();
    assert_eq!(
        stats.keys("").unwrap(),
        vec!["problem", "solving", "summary"]
    );
    assert_eq!(stats.value("summary.models.enumerated").unwrap(), 0.0);
}

#[test]
fn an_unknown_statistics_path_is_an_error_that_does_not_poison() {
    let mut ctl = solved_four_models();
    {
        let stats = ctl.statistics().unwrap();
        for path in ["nosuch", "summary.nosuch", "summary.costs.5"] {
            let err = stats.value(path).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Runtime, "{path}");
            assert!(err.to_string().contains(path), "{err}");
            let err = stats.keys(path).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Runtime, "{path}");
        }
    }
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn reading_a_map_as_a_value_is_an_error_that_does_not_poison() {
    let mut ctl = solved_four_models();
    {
        let stats = ctl.statistics().unwrap();
        let err = stats.value("summary").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime);
        let err = stats.value("summary.costs").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime, "an array is not a value");
        let err = stats.value("").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime);
    }
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_snapshot_holds_the_same_values() {
    let ctl = solved_four_models();
    let stats = ctl.statistics().unwrap();
    let tree = stats.snapshot().unwrap();
    for path in [
        "summary.models.enumerated",
        "summary.exhausted",
        "summary.times.total",
        "problem.lp.atoms",
    ] {
        assert_eq!(tree.value(path), Some(stats.value(path).unwrap()), "{path}");
    }
    assert_eq!(tree.value("summary"), None, "a map is not a value");
    assert_eq!(tree.value("summary.nosuch"), None);
    assert!(tree.get("nosuch").is_none());
    assert_eq!(tree.get(""), Some(&tree));
}

#[test]
fn snapshot_keeps_clingo_order() {
    let mut ctl = grounded(&[], OPTIMISATION);
    assert!(ctl.solve(&[]).unwrap().is_sat());
    let tree = ctl.statistics().unwrap().snapshot().unwrap();

    match tree.get("summary.models") {
        Some(StatsTree::Map(entries)) => {
            let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
            assert_eq!(names, vec!["enumerated", "optimal"]);
            assert_eq!(entries[0].1, StatsTree::Value(2.0));
        }
        other => panic!("summary.models is a map: {other:?}"),
    }
    match tree.get("summary.costs") {
        Some(StatsTree::Array(costs)) => assert_eq!(costs, &vec![StatsTree::Value(0.0)]),
        other => panic!("summary.costs is an array: {other:?}"),
    }
    match &tree {
        StatsTree::Map(entries) => {
            let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
            assert_eq!(
                names,
                vec!["problem", "solving", "summary", "user_step", "user_accu"]
            );
        }
        other => panic!("the root is a map: {other:?}"),
    }
}

#[test]
fn a_snapshot_outlives_later_solves() {
    let mut ctl = solved_four_models();
    let tree = ctl.statistics().unwrap().snapshot().unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
    assert_eq!(value(&ctl, "summary.call"), 1.0);
    assert_eq!(tree.value("summary.call"), Some(0.0));
}

#[test]
fn statistics_refuse_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let err = ctl.statistics().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

#[test]
fn a_forgotten_search_is_closed_before_statistics_are_read() {
    // DESIGN S4: `&self` entry points finish a leftover search first, since
    // clingo.h forbids reading statistics during a search.
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);
    let enumerated = value(&ctl, "summary.models.enumerated");
    assert!(enumerated >= 1.0, "{enumerated}");
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
}

#[test]
fn stats_trees_can_leave_the_thread() {
    fn tree_traits<T: Send + Sync + Clone + PartialEq + std::fmt::Debug + 'static>() {}
    tree_traits::<StatsTree>();
    let ctl = solved_four_models();
    let text = format!("{:?}", ctl.statistics().unwrap());
    assert!(text.contains("Statistics"), "{text}");
}

// ---------------------------------------------------------------------------
// the boundaries and texts mutation testing found untested

#[test]
fn an_array_index_equal_to_the_length_is_a_runtime_error_that_does_not_poison() {
    let mut ctl = grounded(&[], OPTIMISATION);
    assert!(ctl.solve(&[]).unwrap().is_sat());
    {
        let stats = ctl.statistics().unwrap();
        assert_eq!(stats.keys("summary.costs").unwrap().len(), 1);
        assert_eq!(stats.value("summary.costs.0").unwrap(), 0.0);
        let err = stats.value("summary.costs.1").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime, "{err}");
        assert!(err.to_string().contains("no entry `1`"), "{err}");
        let err = stats.keys("summary.costs.1").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime, "{err}");
    }
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn reading_a_map_or_an_array_as_a_value_says_which_it_is() {
    let ctl = solved_four_models();
    let stats = ctl.statistics().unwrap();
    let err = stats.value("summary").unwrap_err();
    assert!(err.to_string().contains("is a map, not a value"), "{err}");
    let err = stats.value("summary.costs").unwrap_err();
    assert!(
        err.to_string().contains("is an array, not a value"),
        "{err}"
    );
}

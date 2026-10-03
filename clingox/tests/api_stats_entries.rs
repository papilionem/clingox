//! Statistics entries: `Statistics::root`, `Statistics::entry`, the
//! `StatsEntry` cursor, and the same two methods on `MutableStatistics`.
//!
//! Expected values come from pyclingo 5.8.2 read at C level
//! (a walk of every node for the node counts, and reads of single entries
//! for names, sizes and costs); the path methods and
//! `Statistics::snapshot` are the second and third oracles, since an entry must
//! read what the path reads and what the snapshot copied.
//! Tests that ask for several threads return early on a build without threads.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "the helper functions of an integration test are test code, which clippy's `allow-unwrap-in-tests` does not cover"
)]
#![allow(
    clippy::float_cmp,
    reason = "the values compared exactly are counts, or the numbers a test wrote itself"
)]

use std::ops::ControlFlow;

use clingox::{
    Control, ErrorKind, MutableStatistics, Part, PathSegment, SolveEventHandler, SolveOptions,
    StatKind, Statistics, StatsEntry, StatsTree,
};

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// `{a;b}.` solved once with `--models=0`: four models.
fn solved_four_models() -> Control {
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    assert!(ctl.solve(&[]).expect("the program solves").is_exhausted());
    ctl
}

/// One model whose cost is 3 at priority 3, 5 at priority 2 and 7 at
/// priority 1, so the three entries of `summary.costs` differ (probe F3).
const THREE_LEVELS: &str = "x. y. z. :~ x. [3@3] :~ y. [5@2] :~ z. [7@1]";

fn solved_three_levels() -> Control {
    let mut ctl = grounded(&[], THREE_LEVELS);
    assert!(ctl.solve(&[]).expect("the program solves").is_sat());
    ctl
}

fn kind_of<T: std::fmt::Debug>(result: clingox::Result<T>) -> ErrorKind {
    result.expect_err("the call must fail").kind()
}

fn poisoned(ctl: &Control) -> bool {
    format!("{ctl:?}").contains("poisoned")
}

/// One node of the tree: its path and, for a value, the bits of its number
/// (bits, so that a NaN or a signed zero compares as clingo reported it).
type Node = (String, Option<u64>);

fn join(path: &str, part: &dyn std::fmt::Display) -> String {
    if path.is_empty() {
        part.to_string()
    } else {
        format!("{path}.{part}")
    }
}

/// A depth-first walk through the cursor. It also checks, at every node, that
/// `keys` and `len` agree with `children`.
fn walk_by_entries(entry: StatsEntry<'_>, path: &str, out: &mut Vec<Node>) {
    let kind = entry.kind().unwrap();
    if kind == StatKind::Value {
        out.push((path.to_owned(), Some(entry.value().unwrap().to_bits())));
    } else {
        out.push((path.to_owned(), None));
    }
    let children: Vec<(PathSegment, StatsEntry<'_>)> =
        entry.children().unwrap().map(Result::unwrap).collect();
    assert_eq!(entry.len().unwrap(), children.len(), "len of `{path}`");
    let segments: Vec<String> = children.iter().map(|(s, _)| s.to_string()).collect();
    assert_eq!(entry.keys().unwrap(), segments, "keys of `{path}`");
    match kind {
        StatKind::Map => assert!(
            children
                .iter()
                .all(|(segment, _)| matches!(segment, PathSegment::Name(_))),
            "a map lists names: `{path}`"
        ),
        StatKind::Array => assert!(
            children
                .iter()
                .enumerate()
                .all(|(i, (segment, _))| *segment == PathSegment::Index(i)),
            "an array lists indices in order: `{path}`"
        ),
        _ => assert!(children.is_empty(), "a value has no children: `{path}`"),
    }
    for (segment, child) in children {
        walk_by_entries(child, &join(path, &segment), out);
    }
}

/// The same walk through `value` and `keys` by path.
fn walk_by_paths(stats: &Statistics<'_>, path: &str, out: &mut Vec<Node>) {
    match stats.value(path) {
        Ok(value) => out.push((path.to_owned(), Some(value.to_bits()))),
        Err(err) => {
            assert_eq!(err.kind(), ErrorKind::Runtime, "`{path}`: {err}");
            out.push((path.to_owned(), None));
            for key in stats.keys(path).unwrap() {
                walk_by_paths(stats, &join(path, &key), out);
            }
        }
    }
}

/// The same walk through the owned snapshot.
fn walk_snapshot(tree: &StatsTree, path: &str, out: &mut Vec<Node>) {
    match tree {
        StatsTree::Value(value) => out.push((path.to_owned(), Some(value.to_bits()))),
        StatsTree::Array(elements) => {
            out.push((path.to_owned(), None));
            for (index, element) in elements.iter().enumerate() {
                walk_snapshot(element, &join(path, &index), out);
            }
        }
        StatsTree::Map(entries) => {
            out.push((path.to_owned(), None));
            for (name, entry) in entries {
                walk_snapshot(entry, &join(path, name), out);
            }
        }
        other => panic!("a tree this test does not know: {other:?}"),
    }
}

/// The three walks of a control's statistics, which must agree.
fn three_walks(ctl: &Control) -> (Vec<Node>, Vec<Node>, Vec<Node>) {
    let stats = ctl.statistics().unwrap();
    let mut by_entries = Vec::new();
    walk_by_entries(stats.root(), "", &mut by_entries);
    let mut by_paths = Vec::new();
    walk_by_paths(&stats, "", &mut by_paths);
    let mut by_snapshot = Vec::new();
    walk_snapshot(&stats.snapshot().unwrap(), "", &mut by_snapshot);
    (by_entries, by_paths, by_snapshot)
}

fn counts(nodes: &[Node]) -> (usize, usize) {
    (
        nodes.len(),
        nodes.iter().filter(|(_, v)| v.is_some()).count(),
    )
}

fn names(entry: StatsEntry<'_>) -> Vec<String> {
    entry
        .children()
        .unwrap()
        .map(|child| match child.unwrap().0 {
            PathSegment::Name(name) => name,
            other => panic!("a map lists names: {other:?}"),
        })
        .collect()
}

#[test]
fn root_children_after_a_solve_follow_clingo_order() {
    let ctl = solved_four_models();
    let stats = ctl.statistics().unwrap();
    let root = stats.root();
    assert_eq!(root.kind().unwrap(), StatKind::Map);
    // Probe K2 and F2; `stats.rs` documents the same list.
    let expected = ["problem", "solving", "summary", "user_step", "user_accu"];
    assert_eq!(names(root), expected);
    assert_eq!(root.keys().unwrap(), expected);
    assert_eq!(stats.keys("").unwrap(), expected);
}

#[test]
fn an_entry_reads_what_value_reads() {
    let ctl = solved_four_models();
    let stats = ctl.statistics().unwrap();
    let enumerated = stats.entry("summary.models.enumerated").unwrap();
    assert_eq!(enumerated.kind().unwrap(), StatKind::Value);
    assert_eq!(enumerated.value().unwrap(), 4.0);
    assert_eq!(
        enumerated.value().unwrap(),
        stats.value("summary.models.enumerated").unwrap()
    );
    // A neighbour with another number, so a mix-up of keys shows.
    let optimal = stats.entry("summary.models.optimal").unwrap();
    assert_eq!(optimal.value().unwrap(), 0.0);
    assert_eq!(
        stats.entry("summary.exhausted").unwrap().value().unwrap(),
        1.0
    );
}

#[test]
fn walking_by_entries_matches_the_path_api_and_the_snapshot() {
    let mut ctl = grounded(&["--stats", "--models=0"], "p(1..50). {q(1..8)}.");
    assert!(ctl.solve(&[]).unwrap().is_exhausted());
    let (by_entries, by_paths, by_snapshot) = three_walks(&ctl);
    assert_eq!(by_entries, by_paths);
    assert_eq!(by_entries, by_snapshot);
    // The counts of pyclingo's walk (probe K2): 187 nodes, 164 values.
    assert_eq!(counts(&by_entries), (187, 164));
    assert_eq!(by_entries[0].0, "");
    assert!(
        by_entries
            .iter()
            .any(|(path, _)| path == "summary.models.enumerated")
    );
    let enumerated = by_entries
        .iter()
        .find(|(path, _)| path == "summary.models.enumerated")
        .and_then(|(_, bits)| *bits)
        .map(f64::from_bits);
    assert_eq!(enumerated, Some(256.0));
    assert!(ctl.solve(&[]).unwrap().is_exhausted());
}

#[test]
fn walking_the_default_statistics_matches_the_path_api_and_the_snapshot() {
    let ctl = solved_four_models();
    let (by_entries, by_paths, by_snapshot) = three_walks(&ctl);
    assert_eq!(by_entries, by_paths);
    assert_eq!(by_entries, by_snapshot);
    // Probe K2: 101 nodes, 87 values.
    assert_eq!(counts(&by_entries), (101, 87));
}

#[test]
fn walking_per_thread_statistics() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let mut ctl = grounded(&["--stats=2", "-t", "2", "--models=0"], "{a;b;c}.");
    assert!(ctl.solve(&[]).unwrap().is_exhausted());
    let (by_entries, by_paths, by_snapshot) = three_walks(&ctl);
    assert_eq!(by_entries, by_paths);
    assert_eq!(by_entries, by_snapshot);
    // Probe K2: 353 nodes, 316 values, eight models.
    assert_eq!(counts(&by_entries), (353, 316));
    let enumerated = by_entries
        .iter()
        .find(|(path, _)| path == "summary.models.enumerated")
        .and_then(|(_, bits)| *bits)
        .map(f64::from_bits);
    assert_eq!(enumerated, Some(8.0));
}

#[test]
fn array_children_are_indices() {
    let ctl = solved_three_levels();
    let stats = ctl.statistics().unwrap();
    let costs = stats.entry("summary.costs").unwrap();
    assert_eq!(costs.kind().unwrap(), StatKind::Array);
    let found: Vec<(PathSegment, f64)> = costs
        .children()
        .unwrap()
        .map(|child| {
            let (segment, child) = child.unwrap();
            (segment, child.value().unwrap())
        })
        .collect();
    // The cost of each priority level, highest first (probe F3).
    assert_eq!(
        found,
        [
            (PathSegment::Index(0), 3.0),
            (PathSegment::Index(1), 5.0),
            (PathSegment::Index(2), 7.0),
        ]
    );
    for (segment, value) in &found {
        let path = format!("summary.costs.{segment}");
        assert_eq!(stats.value(&path).unwrap(), *value, "{path}");
        assert_eq!(
            stats.entry(&path).unwrap().value().unwrap(),
            *value,
            "{path}"
        );
    }
    assert_eq!(costs.keys().unwrap(), ["0", "1", "2"]);
    assert_eq!(costs.len().unwrap(), 3);
    // The same elements by relative path.
    assert_eq!(costs.entry("2").unwrap().value().unwrap(), 7.0);
    assert_eq!(
        stats
            .entry("summary")
            .unwrap()
            .entry("costs.1")
            .unwrap()
            .value()
            .unwrap(),
        5.0
    );
    let (by_entries, by_paths, by_snapshot) = three_walks(&ctl);
    assert_eq!(by_entries, by_paths);
    assert_eq!(by_entries, by_snapshot);
    // Probe F3.
    assert_eq!(counts(&by_entries), (107, 93));
}

#[test]
fn statistics_entries_before_the_first_solve() {
    let ctl = grounded(&[], "a.");
    let stats = ctl.statistics().unwrap();
    let root = stats.root();
    // Probe S0 and F1.
    assert_eq!(names(root), ["problem", "solving", "summary"]);
    assert_eq!(root.len().unwrap(), 3);
    assert_eq!(stats.entry("summary.call").unwrap().value().unwrap(), 0.0);
    assert_eq!(
        stats
            .entry("summary.models.enumerated")
            .unwrap()
            .value()
            .unwrap(),
        0.0
    );
    // `summary.costs` is an array with no element yet (probe F1).
    let costs = stats.entry("summary.costs").unwrap();
    assert_eq!(costs.kind().unwrap(), StatKind::Array);
    assert_eq!(costs.len().unwrap(), 0);
    assert_eq!(costs.children().unwrap().count(), 0);
    assert!(costs.keys().unwrap().is_empty(), "no element, no key");
    let (by_entries, by_paths, by_snapshot) = three_walks(&ctl);
    assert_eq!(by_entries, by_paths);
    assert_eq!(by_entries, by_snapshot);
}

#[test]
fn a_map_read_as_a_value_is_a_runtime_error_that_does_not_poison() {
    let mut ctl = solved_three_levels();
    {
        let stats = ctl.statistics().unwrap();
        // clingo would raise a logic error, "type error" (probe S6), which
        // poisons the control; the cursor checks the kind first.
        for path in ["", "summary", "summary.models", "summary.costs"] {
            let entry = stats.entry(path).unwrap();
            let err = entry.value().unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Runtime, "`{path}`: {err}");
        }
        let err = stats.entry("summary").unwrap().value().unwrap_err();
        assert!(err.to_string().contains("is a map, not a value"), "{err}");
        let err = stats.entry("summary.costs").unwrap().value().unwrap_err();
        assert!(
            err.to_string().contains("is an array, not a value"),
            "{err}"
        );
        // The entries still read after the failures.
        assert_eq!(
            stats.entry("summary.costs.0").unwrap().value().unwrap(),
            3.0
        );
    }
    assert!(!poisoned(&ctl), "{ctl:?}");
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn an_unknown_relative_path_is_a_runtime_error_that_does_not_poison() {
    let mut ctl = solved_three_levels();
    {
        let stats = ctl.statistics().unwrap();
        let summary = stats.entry("summary").unwrap();
        for path in ["nosuch", "models.nosuch", "nosuch.models", "costs.x"] {
            let err = summary.entry(path).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Runtime, "{path}: {err}");
        }
        // An index equal to the length (mirrors `api_statistics.rs`), and one
        // far past it.
        let costs = stats.entry("summary.costs").unwrap();
        assert_eq!(costs.len().unwrap(), 3);
        let err = costs.entry("3").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime, "{err}");
        assert!(err.to_string().contains("no entry `3`"), "{err}");
        assert_eq!(kind_of(costs.entry("1000000")), ErrorKind::Runtime);
        assert_eq!(kind_of(costs.entry("-1")), ErrorKind::Runtime);
        // A step below a value, and below an array's element, which is one.
        let call = stats.entry("summary.call").unwrap();
        assert_eq!(kind_of(call.entry("x")), ErrorKind::Runtime);
        assert_eq!(kind_of(costs.entry("0.x")), ErrorKind::Runtime);
        // The same from the view.
        for path in [
            "nosuch",
            "summary.nosuch",
            "summary.costs.3",
            "summary.call.x",
        ] {
            let err = stats.entry(path).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Runtime, "{path}: {err}");
            assert!(err.to_string().contains(path), "{err}");
        }
        // A NUL cannot be part of a name: it is a missing one.
        assert_eq!(kind_of(summary.entry("mod\0els")), ErrorKind::Runtime);
        assert_eq!(kind_of(stats.entry("sum\0mary")), ErrorKind::Runtime);
        // The entries still read after the failures.
        assert_eq!(costs.entry("2").unwrap().value().unwrap(), 7.0);
        assert_eq!(
            summary.entry("models.enumerated").unwrap().value().unwrap(),
            1.0
        );
    }
    assert!(!poisoned(&ctl), "{ctl:?}");
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_relative_path_resolves_like_the_full_path() {
    let ctl = solved_four_models();
    let stats = ctl.statistics().unwrap();
    let full = stats.entry("summary.models.enumerated").unwrap();
    assert_eq!(full.value().unwrap(), 4.0);
    let relative = stats
        .entry("summary")
        .unwrap()
        .entry("models.enumerated")
        .unwrap();
    assert_eq!(relative.value().unwrap(), full.value().unwrap());
    let stepwise = stats
        .entry("summary")
        .unwrap()
        .entry("models")
        .unwrap()
        .entry("enumerated")
        .unwrap();
    assert_eq!(stepwise.value().unwrap(), 4.0);
    // From the root it is the full path.
    let from_root = stats.root().entry("summary.models.enumerated").unwrap();
    assert_eq!(from_root.value().unwrap(), 4.0);
    // The empty path is the entry itself.
    let models = stats.entry("summary.models").unwrap();
    let itself = models.entry("").unwrap();
    assert_eq!(itself.kind().unwrap(), StatKind::Map);
    assert_eq!(itself.keys().unwrap(), models.keys().unwrap());
    assert_eq!(stats.root().entry("").unwrap().len().unwrap(), 5);
    // The path is relative to the entry: `models` is not below `models`, and
    // `summary` is not below the root's `summary`.
    assert_eq!(kind_of(models.entry("models")), ErrorKind::Runtime);
    assert_eq!(kind_of(models.entry("summary.models")), ErrorKind::Runtime);
    assert_eq!(
        kind_of(stats.entry("summary").unwrap().entry("summary")),
        ErrorKind::Runtime
    );
    assert_eq!(kind_of(stats.root().entry("models")), ErrorKind::Runtime);
    assert_eq!(
        kind_of(stats.root().entry("enumerated")),
        ErrorKind::Runtime
    );
}

#[test]
fn len_counts_children() {
    let ctl = solved_three_levels();
    let stats = ctl.statistics().unwrap();
    // A map: its entries (probe F2 lists ten under `summary`).
    assert_eq!(stats.entry("summary.models").unwrap().len().unwrap(), 2);
    assert_eq!(stats.entry("summary").unwrap().len().unwrap(), 10);
    assert_eq!(stats.root().len().unwrap(), 5);
    // An array: its elements.
    assert_eq!(stats.entry("summary.costs").unwrap().len().unwrap(), 3);
    // A value has none, and no error.
    for path in [
        "summary.call",
        "summary.costs.0",
        "summary.models.enumerated",
    ] {
        let value = stats.entry(path).unwrap();
        assert_eq!(value.len().unwrap(), 0, "{path}");
        assert_eq!(value.children().unwrap().count(), 0, "{path}");
        assert!(value.keys().unwrap().is_empty(), "{path}");
    }
}

#[test]
fn kind_reports_values_arrays_and_maps() {
    let ctl = solved_three_levels();
    let stats = ctl.statistics().unwrap();
    assert_eq!(stats.root().kind().unwrap(), StatKind::Map);
    assert_eq!(
        stats.entry("summary").unwrap().kind().unwrap(),
        StatKind::Map
    );
    assert_eq!(
        stats.entry("summary.models").unwrap().kind().unwrap(),
        StatKind::Map
    );
    assert_eq!(
        stats.entry("summary.call").unwrap().kind().unwrap(),
        StatKind::Value
    );
    assert_eq!(
        stats
            .entry("summary.models.enumerated")
            .unwrap()
            .kind()
            .unwrap(),
        StatKind::Value
    );
    assert_eq!(
        stats.entry("summary.costs").unwrap().kind().unwrap(),
        StatKind::Array
    );
    assert_eq!(
        stats.entry("summary.lower").unwrap().kind().unwrap(),
        StatKind::Array
    );
    assert_eq!(
        stats.entry("summary.costs.1").unwrap().kind().unwrap(),
        StatKind::Value
    );
}

#[test]
fn the_children_iterator_is_fused() {
    let ctl = solved_four_models();
    let stats = ctl.statistics().unwrap();
    let mut children = stats.entry("summary.models").unwrap().children().unwrap();
    assert_eq!(
        children.next().unwrap().unwrap().0,
        PathSegment::Name("enumerated".to_owned())
    );
    assert_eq!(
        children.next().unwrap().unwrap().0,
        PathSegment::Name("optimal".to_owned())
    );
    for _ in 0..3 {
        assert!(children.next().is_none());
    }
    let mut empty = stats.entry("summary.call").unwrap().children().unwrap();
    for _ in 0..3 {
        assert!(empty.next().is_none());
    }
}

#[test]
fn entries_follow_each_solve_call() {
    let mut ctl = solved_four_models();
    {
        let stats = ctl.statistics().unwrap();
        assert_eq!(stats.entry("summary.call").unwrap().value().unwrap(), 0.0);
        assert_eq!(
            stats
                .entry("summary.models.enumerated")
                .unwrap()
                .value()
                .unwrap(),
            4.0
        );
    }
    let assumption = (clingox::Symbol::function("a", &[]).unwrap(), true);
    assert!(ctl.solve(&[assumption.into()]).unwrap().is_exhausted());
    let stats = ctl.statistics().unwrap();
    assert_eq!(stats.entry("summary.call").unwrap().value().unwrap(), 1.0);
    assert_eq!(
        stats
            .entry("summary.models.enumerated")
            .unwrap()
            .value()
            .unwrap(),
        2.0
    );
}

#[test]
fn a_forgotten_search_is_closed_before_an_entry_is_read() {
    // DESIGN S4: a view closes a leftover search first, since clingo.h
    // forbids reading statistics during a search.
    let mut ctl = grounded(&["--models=0"], "{a;b}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);
    let enumerated = ctl
        .statistics()
        .unwrap()
        .entry("summary.models.enumerated")
        .unwrap()
        .value()
        .unwrap();
    assert!(enumerated >= 1.0, "{enumerated}");
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
}

#[test]
fn entries_have_the_lifetime_of_the_view_and_outlive_each_other() {
    // `Statistics::root` and `entry` return the view's own lifetime, and a
    // child has the lifetime of the entry's borrow, not of the entry.
    fn root_of<'c>(stats: &Statistics<'c>) -> StatsEntry<'c> {
        stats.root()
    }
    fn entry_of<'c>(stats: &Statistics<'c>, path: &str) -> StatsEntry<'c> {
        stats.entry(path).unwrap()
    }
    fn first_child(entry: StatsEntry<'_>) -> StatsEntry<'_> {
        entry.children().unwrap().next().unwrap().unwrap().1
    }
    fn relative(entry: StatsEntry<'_>) -> StatsEntry<'_> {
        entry.entry("models").unwrap()
    }
    let ctl = solved_four_models();
    let root = {
        let stats = ctl.statistics().unwrap();
        root_of(&stats)
    };
    let summary = {
        let stats = ctl.statistics().unwrap();
        entry_of(&stats, "summary")
    };
    // The view is gone; the entries read.
    assert_eq!(root.kind().unwrap(), StatKind::Map);
    let problem = first_child(root);
    assert_eq!(
        problem.keys().unwrap(),
        ctl.statistics().unwrap().keys("problem").unwrap()
    );
    let models = relative(summary);
    assert_eq!(models.entry("enumerated").unwrap().value().unwrap(), 4.0);
    // Entries are `Copy`.
    let copy = models;
    assert_eq!(copy.len().unwrap(), models.len().unwrap());
}

#[test]
fn stats_entries_are_debug() {
    let ctl = solved_three_levels();
    let stats = ctl.statistics().unwrap();
    let value = format!("{:?}", stats.entry("summary.costs.1").unwrap());
    assert!(value.contains("StatsEntry"), "{value}");
    assert!(value.contains("kind: Value"), "{value}");
    assert!(value.contains("value: 5.0"), "{value}");
    let map = format!("{:?}", stats.entry("summary.models").unwrap());
    assert!(map.contains("kind: Map"), "{map}");
    assert!(map.contains("len: 2"), "{map}");
    assert!(!map.contains("value"), "a map has no value: {map}");
    let array = format!("{:?}", stats.entry("summary.costs").unwrap());
    assert!(array.contains("kind: Array"), "{array}");
    assert!(array.contains("len: 3"), "{array}");
    let children = format!("{:?}", stats.root().children().unwrap());
    assert!(children.contains("StatsChildren"), "{children}");
    assert!(children.contains(".."), "{children}");
}

/// What a handler read through entries inside `on_statistics`.
#[derive(Debug, Default, PartialEq)]
struct Seen {
    step_value: Option<f64>,
    step_kind: Option<StatKind>,
    step_len: Option<usize>,
    step_names: Vec<String>,
    accumulated_value: Option<f64>,
    after_write: Option<f64>,
    unknown: Option<ErrorKind>,
}

struct ReadThroughEntries<'a> {
    seen: &'a mut Seen,
}

impl SolveEventHandler for ReadThroughEntries<'_> {
    fn on_statistics(
        &mut self,
        step: &mut MutableStatistics<'_>,
        accumulated: &mut MutableStatistics<'_>,
    ) -> clingox::Result<ControlFlow<()>> {
        step.add_map_key("", "mine", StatKind::Value)?;
        step.set_value("mine", 1.0)?;
        accumulated.add_map_key("", "total", StatKind::Value)?;
        accumulated.set_value("total", 5.0)?;
        {
            let mine = step.root().entry("mine")?;
            self.seen.step_value = Some(mine.value()?);
            self.seen.step_kind = Some(mine.kind()?);
            self.seen.step_len = Some(mine.len()?);
            self.seen.unknown = step.entry("nosuch").err().map(|e| e.kind());
            self.seen.step_names = step
                .root()
                .children()?
                .map(|child| child.map(|(segment, _)| segment.to_string()))
                .collect::<clingox::Result<_>>()?;
            let total = accumulated.entry("total")?;
            self.seen.accumulated_value = Some(total.value()?);
        }
        // The entries are gone, so the write is allowed again.
        step.set_value("mine", 2.0)?;
        self.seen.after_write = Some(step.root().entry("mine")?.value()?);
        Ok(ControlFlow::Continue(()))
    }
}

#[test]
fn an_entry_reads_user_statistics_in_the_callback() {
    let mut ctl = grounded(&[], "a.");
    let mut seen = Seen::default();
    let _ = ctl
        .solve_with_events(SolveOptions::new(), ReadThroughEntries { seen: &mut seen })
        .unwrap();
    assert_eq!(seen.step_value, Some(1.0));
    assert_eq!(seen.step_kind, Some(StatKind::Value));
    assert_eq!(seen.step_len, Some(0));
    assert_eq!(seen.accumulated_value, Some(5.0));
    assert_eq!(seen.after_write, Some(2.0));
    assert_eq!(seen.unknown, Some(ErrorKind::Runtime));
    // `step` is the `user_step` map, which starts empty: it lists only the
    // key the handler added.
    assert_eq!(seen.step_names, ["mine"]);
    assert!(!poisoned(&ctl), "{ctl:?}");
    // What the handler wrote last is what the view reads afterwards.
    let stats = ctl.statistics().unwrap();
    assert_eq!(stats.entry("user_step.mine").unwrap().value().unwrap(), 2.0);
}

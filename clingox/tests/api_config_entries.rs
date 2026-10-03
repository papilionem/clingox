//! Configuration entries: `Configuration::root`, `Configuration::entry` and the
//! `ConfigEntry` cursor, which holds clingo's key for one entry.
//!
//! Expected values come from pyclingo 5.8.2 read at C level
//! (a walk of every node for the node counts, and reads of single entries
//! for the names, kinds and values); the path methods are
//! the second oracle, since an entry must read what the path reads.
//! Tests that ask for several solver configurations return early on a build
//! without threads, which cannot have them.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "the helper functions of an integration test are test code, which clippy's `allow-unwrap-in-tests` does not cover"
)]

use clingox::{ConfigEntry, ConfigKind, Configuration, Control, ErrorKind, Part, PathSegment};

fn kind_of<T: std::fmt::Debug>(result: clingox::Result<T>) -> ErrorKind {
    result.expect_err("the call must fail").kind()
}

/// The control still answers and still solves, so nothing poisoned it.
fn assert_usable(ctl: &mut Control) {
    assert_eq!(
        ctl.configuration()
            .get("solve.models")
            .expect("the control is not poisoned")
            .as_deref(),
        Some("-1")
    );
    ctl.add_base("a.").expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    let (result, models) = ctl.solve_all().expect("the control solves");
    assert!(result.is_sat());
    assert_eq!(models.len(), 1);
}

/// What one node of the tree reads, by whichever way of reading it.
#[derive(Debug, PartialEq, Eq)]
struct Node {
    path: String,
    kind: ConfigKind,
    value: Option<String>,
    description: String,
}

fn join(path: &str, part: &dyn std::fmt::Display) -> String {
    if path.is_empty() {
        part.to_string()
    } else {
        format!("{path}.{part}")
    }
}

/// A depth-first walk through the cursor, as the rustdoc of `children` shows.
fn walk_by_entries(entry: ConfigEntry<'_>, path: &str, out: &mut Vec<Node>) {
    out.push(Node {
        path: path.to_owned(),
        kind: entry.kind().unwrap(),
        value: entry.value().unwrap(),
        description: entry.description().unwrap(),
    });
    for child in entry.children().unwrap() {
        let (segment, child) = child.unwrap();
        walk_by_entries(child, &join(path, &segment), out);
    }
}

/// The same walk through the path methods, written as in
/// `guide/src/how-to/configuration.md`: `kind`, then `keys`, `len` and
/// `element` or `get`, with `description` for every node.
fn walk_by_paths(conf: &Configuration<'_>, path: &str, out: &mut Vec<Node>) {
    let kind = conf.kind(path).unwrap();
    out.push(Node {
        path: path.to_owned(),
        kind,
        value: conf.get(path).unwrap(),
        description: conf.description(path).unwrap(),
    });
    match kind {
        ConfigKind::Value => {}
        ConfigKind::Array | ConfigKind::ArrayMap => {
            for index in 0..conf.len(path).unwrap() {
                let element = conf.element(path, index).unwrap();
                walk_by_paths(conf, &element, out);
            }
        }
        ConfigKind::Map => {
            for key in conf.keys(path).unwrap() {
                walk_by_paths(conf, &join(path, &key), out);
            }
        }
        other => panic!("a kind this test does not know: {other:?}"),
    }
}

/// Both walks of the whole tree, which must agree.
fn both_walks(ctl: &mut Control) -> (Vec<Node>, Vec<Node>) {
    let conf = ctl.configuration();
    let mut by_entries = Vec::new();
    walk_by_entries(conf.root().unwrap(), "", &mut by_entries);
    let mut by_paths = Vec::new();
    walk_by_paths(&conf, "", &mut by_paths);
    (by_entries, by_paths)
}

fn names(entry: ConfigEntry<'_>) -> Vec<String> {
    entry
        .children()
        .unwrap()
        .map(|child| match child.unwrap().0 {
            PathSegment::Name(name) => name,
            other => panic!("a map lists names: {other:?}"),
        })
        .collect()
}

const ROOT_NAMES: [&str; 11] = [
    "tester",
    "solve",
    "asp",
    "solver",
    "configuration",
    "share",
    "learn_explicit",
    "sat_prepro",
    "stats",
    "parse_ext",
    "parse_maxsat",
];

/// The names under `solve` in a build with threads. A build without threads
/// (the default WebAssembly target) has no `THREAD_ONLY` options.
const SOLVE_NAMES: [&str; 10] = [
    "solve_limit",
    "parallel_mode",
    "global_restarts",
    "distribute",
    "integrate",
    "enum_mode",
    "project",
    "models",
    "opt_mode",
    "opt_stop",
];

/// The options of `SOLVE_NAMES` that only a build with threads has; all four
/// are assigned by default (pyclingo 5.8.2: `1,compete`, `no`,
/// `conflict,global,4,4194303`, `gp,1024,all`).
const THREAD_ONLY: [&str; 4] = [
    "parallel_mode",
    "global_restarts",
    "distribute",
    "integrate",
];

/// `SOLVE_NAMES` as this build has them.
fn solve_names() -> Vec<&'static str> {
    SOLVE_NAMES
        .into_iter()
        .filter(|name| clingox_sys::HAS_THREADS || !THREAD_ONLY.contains(name))
        .collect()
}

#[test]
fn root_is_the_options_map() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let root = conf.root().unwrap();
    assert_eq!(root.kind().unwrap(), ConfigKind::Map);
    assert_eq!(root.description().unwrap(), "Options");
    assert_eq!(root.value().unwrap(), None);
    assert_eq!(root.keys().unwrap(), conf.keys("").unwrap());
}

#[test]
fn an_entry_reads_what_get_reads() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let models = conf.entry("solve.models").unwrap();
    assert_eq!(models.value().unwrap().as_deref(), Some("-1"));
    assert_eq!(models.value().unwrap(), conf.get("solve.models").unwrap());
    assert_eq!(models.kind().unwrap(), ConfigKind::Value);
    assert_eq!(
        models.description().unwrap(),
        "Compute at most %A models (0 for all)\n"
    );
    // A value that differs from `solve.models`, so a mix-up of keys shows.
    let seed = conf.entry("solver.seed").unwrap();
    assert_eq!(seed.value().unwrap().as_deref(), Some("1"));
    assert_eq!(seed.value().unwrap(), conf.get("solver.seed").unwrap());
}

#[test]
fn root_children_follow_clingo_order() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let root = conf.root().unwrap();
    assert_eq!(names(root), ROOT_NAMES);
    assert_eq!(conf.keys("").unwrap(), ROOT_NAMES);
    // Each child is the entry its name leads to.
    let kinds: Vec<(String, ConfigKind)> = root
        .children()
        .unwrap()
        .map(|child| {
            let (segment, child) = child.unwrap();
            (segment.to_string(), child.kind().unwrap())
        })
        .collect();
    for (name, kind) in &kinds {
        assert_eq!(*kind, conf.kind(name).unwrap(), "{name}");
    }
    assert_eq!(kinds[0], ("tester".to_owned(), ConfigKind::Map));
    assert_eq!(kinds[3], ("solver".to_owned(), ConfigKind::ArrayMap));
    assert_eq!(kinds[4], ("configuration".to_owned(), ConfigKind::Value));
    // The order below the root is clingo's too.
    assert_eq!(names(conf.entry("solve").unwrap()), solve_names());
}

#[test]
fn walking_by_entries_matches_the_path_api() {
    let mut ctl = Control::new().unwrap();
    let (by_entries, by_paths) = both_walks(&mut ctl);
    assert_eq!(by_entries.len(), by_paths.len());
    for (entry, path) in by_entries.iter().zip(&by_paths) {
        assert_eq!(entry, path);
    }
    // The counts of pyclingo's walk (probe K1): 84 nodes, 77 of them values,
    // 73 of those assigned. A build without threads lacks the four assigned
    // `THREAD_ONLY` values.
    let missing = if clingox_sys::HAS_THREADS {
        0
    } else {
        THREAD_ONLY.len()
    };
    assert_eq!(by_entries.len(), 84 - missing);
    let values: Vec<&Node> = by_entries
        .iter()
        .filter(|node| node.kind == ConfigKind::Value)
        .collect();
    assert_eq!(values.len(), 77 - missing);
    assert_eq!(
        values.iter().filter(|node| node.value.is_some()).count(),
        73 - missing
    );
    // The walk starts at the root and visits the element of `solver` once.
    assert_eq!(by_entries[0].path, "");
    assert_eq!(by_entries[1].path, "tester");
    assert_eq!(
        by_entries
            .iter()
            .filter(|node| node.path == "solver.0")
            .count(),
        1
    );
    assert!(by_entries.iter().any(|node| node.path == "solver.0.seed"));
    assert_usable(&mut ctl);
}

#[test]
fn walking_with_three_solvers_reaches_every_solver() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let mut ctl = Control::with_args(["-t", "3"]).unwrap();
    let (by_entries, by_paths) = both_walks(&mut ctl);
    assert_eq!(by_entries, by_paths);
    // The counts of pyclingo's walk with `-t 3` (probe K1).
    assert_eq!(by_entries.len(), 182);
    let values: Vec<&Node> = by_entries
        .iter()
        .filter(|node| node.kind == ConfigKind::Value)
        .collect();
    assert_eq!(values.len(), 173);
    assert_eq!(
        values.iter().filter(|node| node.value.is_some()).count(),
        169
    );
    let conf = ctl.configuration();
    let solver = conf.entry("solver").unwrap();
    assert_eq!(solver.kind().unwrap(), ConfigKind::ArrayMap);
    let indices: Vec<PathSegment> = solver
        .children()
        .unwrap()
        .map(|child| child.unwrap().0)
        .collect();
    assert_eq!(
        indices,
        [
            PathSegment::Index(0),
            PathSegment::Index(1),
            PathSegment::Index(2)
        ]
    );
    for path in ["solver.0.seed", "solver.1.seed", "solver.2.seed"] {
        assert!(by_entries.iter().any(|node| node.path == path), "{path}");
    }
}

#[test]
fn the_elements_of_three_solvers_read_their_own_options() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let mut ctl = Control::with_args(["-t", "3"]).unwrap();
    let mut conf = ctl.configuration();
    // Make the elements differ, so a mix-up of indices shows (probe E8).
    conf.set("solver.1.seed", "7").unwrap();
    conf.set("solver.2.seed", "11").unwrap();
    let solver = conf.entry("solver").unwrap();
    assert_eq!(solver.len().unwrap(), 3);
    let expected = [Some("1"), Some("7"), Some("11")];
    for (index, want) in expected.iter().enumerate() {
        let element = solver.element(index).unwrap();
        assert_eq!(element.kind().unwrap(), ConfigKind::Map);
        let seed = element.entry("seed").unwrap();
        assert_eq!(seed.value().unwrap().as_deref(), *want, "element {index}");
        assert_eq!(
            seed.value().unwrap(),
            conf.get(&format!("solver.{index}.seed")).unwrap()
        );
    }
    // The children of the array are the same entries as its elements.
    for (index, child) in solver.children().unwrap().enumerate() {
        let (segment, child) = child.unwrap();
        assert_eq!(segment, PathSegment::Index(index));
        assert_eq!(
            child.entry("seed").unwrap().value().unwrap().as_deref(),
            expected[index]
        );
    }
}

#[test]
fn array_map_children_are_its_elements() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let solver = conf.entry("solver").unwrap();
    assert_eq!(solver.kind().unwrap(), ConfigKind::ArrayMap);
    // Only the element: the options of its map side are the element's
    // (probe C3), so a walk through the map side would visit them twice.
    let children: Vec<PathSegment> = solver
        .children()
        .unwrap()
        .map(|child| child.unwrap().0)
        .collect();
    assert_eq!(children, [PathSegment::Index(0)]);
    // The map side stays reachable by name and by `keys`, and answers as the
    // element does.
    let by_name = solver.entry("seed").unwrap();
    let by_element = solver.element(0).unwrap().entry("seed").unwrap();
    assert_eq!(by_name.value().unwrap().as_deref(), Some("1"));
    assert_eq!(by_name.value().unwrap(), by_element.value().unwrap());
    let keys = solver.keys().unwrap();
    assert_eq!(keys, conf.keys("solver").unwrap());
    assert_eq!(keys.len(), 48);
    assert_eq!(keys[0], "opt_strategy");
    assert_eq!(keys, solver.element(0).unwrap().keys().unwrap());
    // The element itself is a map.
    assert_eq!(solver.element(0).unwrap().kind().unwrap(), ConfigKind::Map);
    assert_eq!(solver.len().unwrap(), 1);
}

#[test]
fn an_element_past_the_end_is_invalid_input_and_does_not_poison() {
    let mut ctl = Control::new().unwrap();
    {
        let conf = ctl.configuration();
        let solver = conf.entry("solver").unwrap();
        assert_eq!(solver.len().unwrap(), 1);
        // clingo itself would return an entry for 1 (probe C8); the cursor
        // checks the size first, as `Configuration::element` does.
        for index in [1, 2, 5, 63, 64, usize::MAX / 2, usize::MAX] {
            assert_eq!(
                kind_of(solver.element(index)),
                ErrorKind::InvalidInput,
                "index {index}"
            );
        }
        // An array with no element yet has no valid index at all.
        let tester = conf.entry("tester.solver").unwrap();
        assert_eq!(tester.len().unwrap(), 0);
        assert_eq!(kind_of(tester.element(0)), ErrorKind::InvalidInput);
        let err = solver.element(1).unwrap_err();
        assert!(err.to_string().contains('1'), "{err}");
        // The valid index still works after the failures.
        assert_eq!(solver.element(0).unwrap().kind().unwrap(), ConfigKind::Map);
    }
    assert_usable(&mut ctl);
}

#[test]
fn an_element_past_the_end_of_three_solvers_is_invalid_input() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let mut ctl = Control::with_args(["-t", "3"]).unwrap();
    {
        let conf = ctl.configuration();
        let solver = conf.entry("solver").unwrap();
        assert_eq!(solver.len().unwrap(), 3);
        // clingo accepts 3 and 4 and wraps a huge offset to element 0.
        for index in [3, 4, 100, usize::MAX / 2, usize::MAX] {
            assert_eq!(
                kind_of(solver.element(index)),
                ErrorKind::InvalidInput,
                "index {index}"
            );
        }
        assert!(solver.element(2).is_ok());
    }
    assert_eq!(ctl.configuration().len("solver").unwrap(), 3);
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert_eq!(models.len(), 1);
}

#[test]
fn len_and_element_refuse_an_entry_that_is_not_an_array() {
    let mut ctl = Control::new().unwrap();
    {
        let conf = ctl.configuration();
        // Maps, an element of an array and values, reached by path or by
        // element; none is an array.
        for path in [
            "",
            "solve",
            "solve.models",
            "solver.0",
            "solver.0.seed",
            "solver.seed",
            "asp",
        ] {
            let entry = conf.entry(path).unwrap();
            assert_eq!(
                kind_of(entry.len()),
                ErrorKind::InvalidInput,
                "len of `{path}`"
            );
            assert_eq!(
                kind_of(entry.element(0)),
                ErrorKind::InvalidInput,
                "element of `{path}`"
            );
        }
        // The arrays do answer.
        assert_eq!(conf.entry("solver").unwrap().len().unwrap(), 1);
        assert_eq!(conf.entry("tester.solver").unwrap().len().unwrap(), 0);
    }
    assert_usable(&mut ctl);
}

#[test]
fn value_is_none_for_a_map_and_for_an_unassigned_option() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    assert_eq!(conf.entry("solve").unwrap().value().unwrap(), None);
    assert_eq!(conf.entry("solver").unwrap().value().unwrap(), None);
    // An option clingo leaves unassigned (probe C7).
    let heuristic = conf.entry("tester.solver.heuristic").unwrap();
    assert_eq!(heuristic.kind().unwrap(), ConfigKind::Value);
    assert_eq!(heuristic.value().unwrap(), None);
    assert_eq!(conf.get("tester.solver.heuristic").unwrap(), None);
}

#[test]
fn an_entry_reads_the_tester_options_a_set_creates() {
    let mut ctl = Control::new().unwrap();
    let mut conf = ctl.configuration();
    assert_eq!(
        conf.entry("tester.solver.heuristic")
            .unwrap()
            .value()
            .unwrap(),
        None
    );
    // The first `set` of any tester option creates the tester configuration,
    // with every option assigned its default (probe E7).
    conf.set("tester.solver.opt_strategy", "bb").unwrap();
    let heuristic = conf.entry("tester.solver.heuristic").unwrap();
    assert_eq!(heuristic.value().unwrap().as_deref(), Some("auto,0"));
    assert_eq!(
        conf.entry("tester.solver.opt_strategy")
            .unwrap()
            .value()
            .unwrap()
            .as_deref(),
        Some("bb,lin")
    );
    assert_eq!(conf.entry("tester.solver").unwrap().len().unwrap(), 1);
}

#[test]
fn a_relative_path_resolves_like_the_full_path() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let full = conf.entry("solve.models").unwrap();
    let relative = conf.entry("solve").unwrap().entry("models").unwrap();
    assert_eq!(relative.value().unwrap().as_deref(), Some("-1"));
    assert_eq!(relative.value().unwrap(), full.value().unwrap());
    assert_eq!(relative.description().unwrap(), full.description().unwrap());
    // From the root it is the full path.
    let from_root = conf.root().unwrap().entry("solve.models").unwrap();
    assert_eq!(from_root.value().unwrap(), full.value().unwrap());
    // The empty path is the entry itself.
    let solve = conf.entry("solve").unwrap();
    let itself = solve.entry("").unwrap();
    assert_eq!(itself.kind().unwrap(), ConfigKind::Map);
    assert_eq!(itself.description().unwrap(), "Solve Options");
    assert_eq!(itself.keys().unwrap(), solve.keys().unwrap());
    let root_itself = conf.root().unwrap().entry("").unwrap();
    assert_eq!(root_itself.description().unwrap(), "Options");
    // Dotted relative paths, numbers on arrays, and a name on `solver`
    // through its element 0.
    let heuristic = conf
        .entry("tester")
        .unwrap()
        .entry("solver.heuristic")
        .unwrap();
    assert_eq!(heuristic.kind().unwrap(), ConfigKind::Value);
    let seed = conf.entry("solver").unwrap().entry("0.seed").unwrap();
    assert_eq!(seed.value().unwrap().as_deref(), Some("1"));
    // The path is relative to the entry: a name that is not below it does not
    // resolve from there, though it would from the root.
    assert_eq!(
        kind_of(conf.entry("solve").unwrap().entry("solve")),
        ErrorKind::Runtime
    );
    assert_eq!(
        kind_of(conf.root().unwrap().entry("models")),
        ErrorKind::Runtime
    );
    assert_eq!(
        kind_of(conf.entry("solve").unwrap().entry("solve.models")),
        ErrorKind::Runtime
    );
}

#[test]
fn a_solver_number_past_the_end_resolves_as_the_path_methods_resolve_it() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    assert_eq!(conf.len("solver").unwrap(), 1);
    // clingo reads these through the solver it wraps to (probe E5), and
    // `has_key` documents them as present.
    for path in ["solver.5.seed", "solver.63.seed", "solver.64.seed"] {
        let entry = conf.entry(path).unwrap();
        assert_eq!(entry.value().unwrap(), conf.get(path).unwrap(), "{path}");
        assert_eq!(entry.value().unwrap().as_deref(), Some("1"), "{path}");
    }
    let past = conf.entry("solver").unwrap().entry("5.seed").unwrap();
    assert_eq!(past.value().unwrap().as_deref(), Some("1"));
}

#[test]
fn an_unknown_relative_path_is_a_runtime_error_that_does_not_poison() {
    let mut ctl = Control::new().unwrap();
    {
        let conf = ctl.configuration();
        let solve = conf.entry("solve").unwrap();
        for path in ["nosuch", "models.nosuch", "nosuch.models"] {
            let err = solve.entry(path).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Runtime, "{path}: {err}");
            assert!(
                err.to_string().contains(path),
                "the message names the input: {err}"
            );
        }
        // Below a value there is nothing to find (probe E6).
        let models = conf.entry("solve.models").unwrap();
        assert_eq!(kind_of(models.entry("x")), ErrorKind::Runtime);
        // The same from the view.
        for path in ["nosuch", "solver.0.nosuch", "solver.x", "solve.models.x"] {
            let err = conf.entry(path).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Runtime, "{path}: {err}");
            assert!(err.to_string().contains(path), "{err}");
        }
        // The entry still reads after the failures.
        assert_eq!(solve.keys().unwrap(), solve_names());
    }
    assert_usable(&mut ctl);
}

#[test]
fn a_nul_in_a_relative_path_is_rejected() {
    let mut ctl = Control::new().unwrap();
    {
        let conf = ctl.configuration();
        let solve = conf.entry("solve").unwrap();
        assert_eq!(kind_of(solve.entry("mod\0els")), ErrorKind::Nul);
        assert_eq!(kind_of(solve.entry("\0")), ErrorKind::Nul);
        assert_eq!(kind_of(conf.entry("solve\0")), ErrorKind::Nul);
        assert_eq!(
            kind_of(conf.root().unwrap().entry("solve.mod\0els")),
            ErrorKind::Nul
        );
        assert_eq!(
            solve.entry("models").unwrap().value().unwrap().as_deref(),
            Some("-1")
        );
    }
    assert_usable(&mut ctl);
}

#[test]
fn a_value_has_no_children_and_no_keys() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    for path in ["solve.models", "solver.0.seed", "tester.solver.heuristic"] {
        let value = conf.entry(path).unwrap();
        assert_eq!(value.children().unwrap().count(), 0, "{path}");
        assert!(value.keys().unwrap().is_empty(), "{path}");
    }
}

#[test]
fn the_children_iterator_is_fused() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let mut children = conf.entry("solve").unwrap().children().unwrap();
    for _ in 0..solve_names().len() {
        assert!(children.next().unwrap().is_ok());
    }
    for _ in 0..3 {
        assert!(children.next().is_none());
    }
    let mut empty = conf.entry("solve.models").unwrap().children().unwrap();
    for _ in 0..3 {
        assert!(empty.next().is_none());
    }
    // An array with no element yields nothing either.
    assert_eq!(
        conf.entry("tester.solver")
            .unwrap()
            .children()
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn entries_see_values_set_before_they_were_made() {
    let mut ctl = Control::new().unwrap();
    let mut conf = ctl.configuration();
    conf.set("solve.models", "0").unwrap();
    let entry = conf.entry("solve.models").unwrap();
    assert_eq!(entry.value().unwrap().as_deref(), Some("0"));
    assert_eq!(entry.value().unwrap(), conf.get("solve.models").unwrap());
    let walked = conf
        .root()
        .unwrap()
        .entry("solve")
        .unwrap()
        .children()
        .unwrap()
        .map(Result::unwrap)
        .find(|(segment, _)| *segment == PathSegment::Name("models".to_owned()))
        .expect("`models` is a child of `solve`");
    assert_eq!(walked.1.value().unwrap().as_deref(), Some("0"));
}

#[test]
fn a_child_and_a_copy_outlive_the_entry_they_came_from() {
    fn first_child(entry: ConfigEntry<'_>) -> ConfigEntry<'_> {
        entry.children().unwrap().next().unwrap().unwrap().1
    }
    fn element_of(entry: ConfigEntry<'_>) -> ConfigEntry<'_> {
        entry.element(0).unwrap()
    }
    fn relative(entry: ConfigEntry<'_>) -> ConfigEntry<'_> {
        entry.entry("models").unwrap()
    }
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let root = conf.root().unwrap();
    let copy = root;
    // The first child of the root is `tester`.
    let tester = first_child(root);
    assert_eq!(tester.description().unwrap(), "Tester Options");
    assert_eq!(copy.kind().unwrap(), root.kind().unwrap());
    let element = element_of(conf.entry("solver").unwrap());
    assert_eq!(element.kind().unwrap(), ConfigKind::Map);
    let models = relative(conf.entry("solve").unwrap());
    assert_eq!(models.value().unwrap().as_deref(), Some("-1"));
}

#[test]
fn the_view_and_the_entry_refuse_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let conf = ctl.configuration();
    assert_eq!(kind_of(conf.root()), ErrorKind::Poisoned);
    assert_eq!(kind_of(conf.entry("solve")), ErrorKind::Poisoned);
    assert_eq!(kind_of(conf.entry("")), ErrorKind::Poisoned);
}

#[test]
fn a_forgotten_search_is_closed_before_an_entry_is_made() {
    // DESIGN S4: a view closes a leftover search first.
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);
    {
        let conf = ctl.configuration();
        let models = conf.entry("solve.models").unwrap();
        assert_eq!(models.value().unwrap().as_deref(), Some("0"));
    }
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
}

#[test]
fn config_entries_are_debug() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let models = format!("{:?}", conf.entry("solve.models").unwrap());
    assert!(models.contains("ConfigEntry"), "{models}");
    assert!(models.contains("kind: Value"), "{models}");
    assert!(models.contains("value: Some(\"-1\")"), "{models}");
    let unassigned = format!("{:?}", conf.entry("tester.solver.heuristic").unwrap());
    assert!(unassigned.contains("kind: Value"), "{unassigned}");
    assert!(unassigned.contains("value: None"), "{unassigned}");
    let solve = format!("{:?}", conf.entry("solve").unwrap());
    assert!(solve.contains("kind: Map"), "{solve}");
    assert!(!solve.contains("value"), "a map has no value: {solve}");
    let solver = format!("{:?}", conf.entry("solver").unwrap());
    assert!(solver.contains("kind: ArrayMap"), "{solver}");
    let children = format!("{:?}", conf.root().unwrap().children().unwrap());
    assert!(children.contains("ConfigChildren"), "{children}");
    assert!(children.contains(".."), "{children}");
}

#[test]
fn path_segments_join_into_paths_the_path_methods_accept() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let solver = conf.entry("solver").unwrap();
    let (index, element) = solver.children().unwrap().next().unwrap().unwrap();
    assert_eq!(index, PathSegment::Index(0));
    assert_eq!(index.to_string(), "0");
    let (name, _) = element.children().unwrap().next().unwrap().unwrap();
    assert_eq!(name, PathSegment::Name("opt_strategy".to_owned()));
    assert_eq!(name.to_string(), "opt_strategy");
    let path = [PathSegment::Name("solver".to_owned()), index, name]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(".");
    assert_eq!(path, "solver.0.opt_strategy");
    assert!(conf.get(&path).unwrap().is_some());
    assert_eq!(
        conf.entry(&path).unwrap().value().unwrap(),
        conf.get(&path).unwrap()
    );
}

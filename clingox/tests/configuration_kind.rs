//! `Configuration::kind` (smoke finding B2): what kind of entry a
//! configuration path names, so a caller can walk the tree.
//!
//! Oracle: pyclingo 5.8.2 at the C level, `clingo_configuration_type` (a
//! bit set: value 1, array 2, map 4), walking from the root with
//! `clingo_configuration_map_at` and `clingo_configuration_array_at`:
//!
//! | path | bits | kind |
//! |---|---|---|
//! | `""`, `solve`, `asp`, `solver.0` | 4 | map |
//! | `solve.models`, `solve.opt_mode`, `solver.0.heuristic`, `asp.eq`, `configuration` | 1 | value |
//! | `solver` | 6 | array and map |
//!
//! `solver` is an array of per-thread maps that is also a map, so it is its own
//! kind. An unknown path is a Runtime error (code 1), as `get`, `len` and
//! `description` report it. The names are the ones of `smoke-notes.md`:
//! `ConfigKind::{Value, Array, Map, ArrayMap}`.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use clingox::{ConfigKind, Control, ErrorKind};

#[test]
fn every_kind_of_entry_is_told_apart() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let cases = [
        ("", ConfigKind::Map),
        ("solve", ConfigKind::Map),
        ("asp", ConfigKind::Map),
        ("solver.0", ConfigKind::Map),
        ("solve.models", ConfigKind::Value),
        ("solve.opt_mode", ConfigKind::Value),
        ("solve.solve_limit", ConfigKind::Value),
        ("solver.0.heuristic", ConfigKind::Value),
        ("solver.0.restarts", ConfigKind::Value),
        ("asp.eq", ConfigKind::Value),
        ("configuration", ConfigKind::Value),
        ("solver", ConfigKind::ArrayMap),
    ];
    for (path, expected) in cases {
        assert_eq!(conf.kind(path).unwrap(), expected, "{path:?}");
    }
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn the_solver_array_stays_an_array_and_map_with_several_threads() {
    let mut ctl = Control::with_args(["-t", "2"]).unwrap();
    let conf = ctl.configuration();
    assert_eq!(conf.kind("solver").unwrap(), ConfigKind::ArrayMap);
    assert_eq!(conf.kind("solver.0").unwrap(), ConfigKind::Map);
    assert_eq!(conf.kind("solver.1").unwrap(), ConfigKind::Map);
    assert_eq!(conf.kind("solver.1.heuristic").unwrap(), ConfigKind::Value);
}

#[test]
fn an_unknown_path_is_a_runtime_error() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    for path in ["nosuch", "solve.nosuch", "solver.0.nosuch", "solver.x"] {
        let error = conf.kind(path).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Runtime, "{path}");
    }
    let error = conf.kind("nosuch").unwrap_err();
    assert!(error.to_string().contains("nosuch"), "{error}");
}

#[test]
fn a_nul_in_the_path_is_a_nul_error_and_the_control_lives() {
    let mut ctl = Control::new().unwrap();
    assert_eq!(
        ctl.configuration().kind("solve\0").unwrap_err().kind(),
        ErrorKind::Nul
    );
    assert_eq!(
        ctl.configuration().get("solve.models").unwrap().as_deref(),
        Some("-1")
    );
}

/// The kinds are enough to walk the whole tree, which is what the query is for.
#[test]
fn a_walk_by_kind_reaches_every_value_once() {
    fn walk(conf: &clingox::Configuration<'_>, path: &str, values: &mut Vec<String>) {
        match conf.kind(path).unwrap() {
            ConfigKind::Value => values.push(path.to_owned()),
            ConfigKind::Map | ConfigKind::ArrayMap => {
                for key in conf.keys(path).unwrap() {
                    let child = if path.is_empty() {
                        key
                    } else {
                        format!("{path}.{key}")
                    };
                    walk(conf, &child, values);
                }
            }
            other => panic!("{path}: unexpected {other:?}"),
        }
    }
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let mut values = Vec::new();
    walk(&conf, "", &mut values);
    assert!(values.iter().any(|v| v == "solve.models"), "{values:?}");
    assert!(values.iter().any(|v| v == "asp.eq"), "{values:?}");
    let mut sorted = values.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), values.len(), "no value is visited twice");
}

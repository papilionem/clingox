//! Reading and changing clingo's configuration by path, and the builder's typed
//! shortcuts `threads` and `seed`.
//!
//! Keys, values and error codes were checked against clingo 5.8.2: values
//! through the Python module, error codes through its C functions
//! (`clingo_configuration_*`). An unknown key and a rejected value are both
//! runtime errors in clingo, so none of the errors here poisons.

#![forbid(unsafe_code)]

use clingox::{Control, ErrorKind, Part};

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn value(ctl: &mut Control, path: &str) -> Option<String> {
    ctl.configuration()
        .get(path)
        .expect("the path exists in clingo's configuration")
}

fn model_count(ctl: &mut Control) -> usize {
    let mut count = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            count += 1;
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .expect("the program solves");
    assert!(result.is_sat());
    count
}

// ---------------------------------------------------------------------------
// Configuration

#[test]
fn get_reads_a_value_by_path() {
    let mut ctl = Control::new().unwrap();
    assert_eq!(value(&mut ctl, "solve.models").as_deref(), Some("-1"));
    assert_eq!(value(&mut ctl, "solve.opt_mode").as_deref(), Some("opt"));
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "clasp has no parallel_mode option without threads"
)]
fn get_reads_the_parallel_mode() {
    let mut ctl = Control::new().unwrap();
    assert_eq!(
        value(&mut ctl, "solve.parallel_mode").as_deref(),
        Some("1,compete")
    );
}

#[test]
fn paths_select_array_elements_by_number() {
    let mut ctl = Control::new().unwrap();
    assert_eq!(value(&mut ctl, "solver.0.seed").as_deref(), Some("1"));
    // clingo resolves a name on an array through its first element.
    assert_eq!(value(&mut ctl, "solver.seed").as_deref(), Some("1"));
}

#[test]
fn get_returns_none_for_a_map() {
    let mut ctl = Control::new().unwrap();
    assert_eq!(value(&mut ctl, "solve"), None);
    assert_eq!(value(&mut ctl, ""), None);
}

#[test]
fn get_returns_none_for_an_unassigned_option() {
    // clingo leaves the tester's options unassigned; pyclingo reads them as
    // None.
    let mut ctl = Control::new().unwrap();
    assert_eq!(value(&mut ctl, "tester.solver.heuristic"), None);
}

#[test]
fn set_changes_a_value_for_the_next_solve() {
    let mut ctl = grounded(&[], "{a;b}.");
    assert_eq!(model_count(&mut ctl), 1, "the default model limit is one");
    ctl.configuration().set("solve.models", "0").unwrap();
    assert_eq!(value(&mut ctl, "solve.models").as_deref(), Some("0"));
    assert_eq!(model_count(&mut ctl), 4);
    ctl.configuration().set("solve.models", "2").unwrap();
    assert_eq!(model_count(&mut ctl), 2);
}

#[test]
fn values_set_by_arguments_can_be_read() {
    let mut ctl = Control::with_args(["--models=0", "--opt-mode=optN"]).unwrap();
    assert_eq!(value(&mut ctl, "solve.models").as_deref(), Some("0"));
    assert_eq!(value(&mut ctl, "solve.opt_mode").as_deref(), Some("optN"));
}

#[test]
fn keys_lists_a_map_in_clingo_order() {
    let mut ctl = Control::new().unwrap();
    let mut conf = ctl.configuration();
    let mut solve_keys = vec![
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
    // clasp defines these four only when built with threads
    // (clasp/clasp/cli/clasp_cli_options.inl:519), which the default WASM
    // build is not.
    if cfg!(all(target_family = "wasm", not(target_feature = "atomics"))) {
        solve_keys.retain(|key| {
            ![
                "parallel_mode",
                "global_restarts",
                "distribute",
                "integrate",
            ]
            .contains(key)
        });
    }
    assert_eq!(conf.keys("solve").unwrap(), solve_keys);
    assert_eq!(
        conf.keys("").unwrap(),
        vec![
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
        ]
    );
    assert!(
        conf.keys("solve.models").unwrap().is_empty(),
        "a value has no keys"
    );
    // The view is usable for writing too while it lives.
    conf.set("solve.models", "0").unwrap();
    assert_eq!(conf.get("solve.models").unwrap().as_deref(), Some("0"));
}

#[test]
fn an_unknown_path_is_a_runtime_error_that_does_not_poison() {
    let mut ctl = grounded(&[], "a.");
    let err = ctl.configuration().get("solve.nosuch").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
    assert!(err.to_string().contains("solve.nosuch"), "{err}");
    let err = ctl.configuration().set("nosuch", "1").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
    let err = ctl.configuration().keys("nosuch.at.all").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);

    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_rejected_value_is_a_runtime_error_that_does_not_poison() {
    let mut ctl = grounded(&[], "a.");
    let err = ctl.configuration().set("solve.models", "abc").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
    assert!(err.to_string().contains("solve.models"), "{err}");
    let err = ctl.configuration().set("solve", "1").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime, "a map takes no value");

    assert_eq!(value(&mut ctl, "solve.models").as_deref(), Some("-1"));
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_nul_in_a_path_or_value_is_rejected() {
    let mut ctl = Control::new().unwrap();
    let err = ctl.configuration().get("solve\0.models").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    let err = ctl.configuration().set("solve.models", "0\0").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    assert_eq!(value(&mut ctl, "solve.models").as_deref(), Some("-1"));
}

#[test]
fn configuration_refuses_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let err = ctl.configuration().get("solve.models").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
    let err = ctl.configuration().set("solve.models", "0").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
    let err = ctl.configuration().keys("solve").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

#[test]
fn the_configuration_view_is_debug() {
    let mut ctl = Control::new().unwrap();
    let text = format!("{:?}", ctl.configuration());
    assert!(text.contains("Configuration"), "{text}");
}

// ---------------------------------------------------------------------------
// Typed shortcuts on the builder

#[test]
fn builder_arguments_accumulate() {
    let mut ctl = Control::builder()
        .args(["--models=0"])
        .args(["--opt-mode=optN"])
        .build()
        .unwrap();
    assert_eq!(value(&mut ctl, "solve.models").as_deref(), Some("0"));
    assert_eq!(value(&mut ctl, "solve.opt_mode").as_deref(), Some("optN"));
}

#[test]
fn seed_sets_the_solver_seed() {
    let mut ctl = Control::builder().seed(42).build().unwrap();
    assert_eq!(value(&mut ctl, "solver.seed").as_deref(), Some("42"));
    let mut ctl = Control::builder().seed(0).build().unwrap();
    assert_eq!(value(&mut ctl, "solver.seed").as_deref(), Some("0"));
    // clingo prints the largest seed as `umax`.
    let mut ctl = Control::builder().seed(u32::MAX).build().unwrap();
    assert_eq!(value(&mut ctl, "solver.seed").as_deref(), Some("umax"));
}

#[test]
fn seed_and_the_seed_argument_together_are_rejected() {
    // clingo refuses an option given twice ("multiple occurrences").
    let err = Control::builder()
        .seed(1)
        .args(["--seed=2"])
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Logic);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build has no threads and no parallel_mode option"
)]
fn threads_sets_the_parallel_mode() {
    let mut ctl = Control::builder().threads(4).build().unwrap();
    assert_eq!(
        value(&mut ctl, "solve.parallel_mode").as_deref(),
        Some("4,compete")
    );
    let mut ctl = Control::builder().threads(1).build().unwrap();
    assert_eq!(
        value(&mut ctl, "solve.parallel_mode").as_deref(),
        Some("1,compete")
    );
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build has no threads and no parallel_mode option"
)]
fn threads_out_of_clingo_range_are_rejected() {
    // clingo accepts 1 to 64 solver threads. clingox checks the typed
    // shortcut itself, so these are its own `InvalidInput`, not clingo's
    // `Logic`.
    for n in [0, 65] {
        let err = Control::builder().threads(n).build().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "threads({n})");
    }
    let err = Control::builder()
        .threads(2)
        .args(["--parallel-mode=4"])
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Logic);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn a_control_with_several_threads_solves() {
    let mut ctl = Control::builder()
        .threads(4)
        .args(["--models=0"])
        .build()
        .unwrap();
    ctl.add_base("{a;b;c}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_exhausted());
    assert_eq!(models.len(), 8);
}

#[test]
#[cfg_attr(
    not(all(target_family = "wasm", not(target_feature = "atomics"))),
    ignore = "only a build without threads lacks parallel solving"
)]
fn more_than_one_thread_is_unsupported_without_threads() {
    let err = Control::builder().threads(2).build().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    // One thread is what such a build has, so asking for it is fine.
    let mut ctl = Control::builder().threads(1).build().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

// ---------------------------------------------------------------------------
// a value has no keys

#[test]
fn a_value_has_no_keys() {
    let mut ctl = Control::new().unwrap();
    {
        let config = ctl.configuration();
        assert!(config.keys("solve.models").unwrap().is_empty());
        assert!(config.keys("solver.0.seed").unwrap().is_empty());
        // `solver` is an array and also a map: clingo resolves its names
        // through the first element (Python clingo 5.8.2 lists 48 keys there).
        assert_eq!(
            config.keys("solver").unwrap(),
            config.keys("solver.0").unwrap()
        );
    }
    assert!(!format!("{ctl:?}").contains("poisoned"));
}

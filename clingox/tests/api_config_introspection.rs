//! Configuration introspection: `description`, `len`, `element` and `has_key`.
//!
//! Oracle: pyclingo 5.8.2's `Configuration` (`len`, `[]`, `description`,
//! `keys`), with error codes read from the C functions. Sizes and texts below
//! are the measured values. Tests that ask for several solver configurations
//! are ignored on a WASM build without threads, which cannot have them.

#![forbid(unsafe_code)]

use clingox::{Control, ErrorKind, Part};

/// A control asking for `threads` solver configurations.
fn with_threads(threads: &str) -> Control {
    Control::with_args(["-t", threads]).expect("the arguments are valid")
}

fn kind<T: std::fmt::Debug>(result: clingox::Result<T>) -> ErrorKind {
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

const SEED_TEXT: &str = "Set random number generator's seed to %A";

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn solver_array_has_one_element_per_thread() {
    let mut ctl = with_threads("3");
    let conf = ctl.configuration();
    assert_eq!(conf.len("solver").unwrap(), 3);
    for index in 0..3 {
        assert_eq!(
            conf.element("solver", index).unwrap(),
            format!("solver.{index}")
        );
    }
    let first = conf.keys("solver.0").unwrap();
    assert_eq!(first.len(), 48);
    assert_eq!(
        first[..4],
        [
            "opt_strategy",
            "opt_usc_shrink",
            "opt_heuristic",
            "restart_on_model"
        ]
    );
    for index in 0..3 {
        let path = conf.element("solver", index).unwrap();
        assert_eq!(conf.keys(&path).unwrap(), first, "{path}");
    }
    // The tester array is empty until one of its options is set.
    assert_eq!(conf.len("tester.solver").unwrap(), 0);
}

#[test]
fn descriptions_are_clingos_text_for_every_kind_of_entry() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    // The root, which pyclingo cannot ask for.
    assert_eq!(conf.description("").unwrap(), "Options");
    // Maps and arrays.
    assert_eq!(conf.description("solve").unwrap(), "Solve Options");
    assert_eq!(conf.description("asp").unwrap(), "Asp Options");
    assert_eq!(conf.description("tester").unwrap(), "Tester Options");
    assert_eq!(conf.description("solver").unwrap(), "Solver Options");
    assert_eq!(conf.description("solver.0").unwrap(), "Solver Options");
    assert_eq!(conf.description("tester.solver").unwrap(), "Solver Options");
    // Values, reached through the array or through its first element.
    assert_eq!(conf.description("solver.seed").unwrap(), SEED_TEXT);
    assert_eq!(conf.description("solver.0.seed").unwrap(), SEED_TEXT);
    // Returned verbatim, including a trailing newline and the `%A` marker.
    assert_eq!(
        conf.description("solve.models").unwrap(),
        "Compute at most %A models (0 for all)\n"
    );
    // The same for an option without a value.
    assert_eq!(conf.get("tester.solver.heuristic").unwrap(), None);
    assert!(
        conf.description("tester.solver.heuristic")
            .unwrap()
            .starts_with("Configure decision heuristic\n      %A: {Berkmin|Vmtf|")
    );
}

#[test]
fn descriptions_of_elements_past_the_end_exist_too() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    assert_eq!(conf.len("solver").unwrap(), 1);
    assert_eq!(conf.description("solver.2.seed").unwrap(), SEED_TEXT);
}

#[test]
fn has_key_reports_the_sub_entries_of_a_map() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    assert!(conf.has_key("", "solve").unwrap());
    assert!(!conf.has_key("", "nosuch").unwrap());
    assert!(conf.has_key("solve", "models").unwrap());
    assert!(!conf.has_key("solve", "nosuch").unwrap());
    // Case matters, and so does every character.
    assert!(!conf.has_key("solve", "MODELS").unwrap());
    assert!(!conf.has_key("solve", "models ").unwrap());
    assert!(conf.has_key("solver.0", "seed").unwrap());
    assert!(!conf.has_key("solver.0", "nosuchkey").unwrap());
    assert!(conf.has_key("tester", "solver").unwrap());
    // Several levels at once, by joining with a period.
    assert!(conf.has_key("", "solver.0.seed").unwrap());
    assert!(conf.has_key("tester", "solver.seed").unwrap());
    // `solver` is an array and a map at once, so it answers as a map.
    assert!(conf.has_key("solver", "seed").unwrap());
    assert!(conf.has_key("solver", "0").unwrap());
}

#[test]
fn has_key_is_not_bounded_by_the_array_size() {
    // clingo answers true for an element that `set` would create, while
    // `element` refuses it: the two differ on purpose.
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    assert_eq!(conf.len("solver").unwrap(), 1);
    assert!(conf.has_key("", "solver.5.seed").unwrap());
    assert_eq!(kind(conf.element("solver", 5)), ErrorKind::InvalidInput);
}

#[test]
fn has_key_on_a_value_is_invalid_input() {
    let mut ctl = Control::new().unwrap();
    assert_eq!(
        kind(ctl.configuration().has_key("solver.0.seed", "x")),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(ctl.configuration().has_key("solve.models", "")),
        ErrorKind::InvalidInput
    );
    assert_usable(&mut ctl);
}

#[test]
fn len_and_element_refuse_an_entry_that_is_not_an_array() {
    let mut ctl = Control::new().unwrap();
    for path in [
        "",
        "solve",
        "solver.0",
        "solver.0.seed",
        "solver.seed",
        "asp",
    ] {
        assert_eq!(
            kind(ctl.configuration().len(path)),
            ErrorKind::InvalidInput,
            "len of `{path}`"
        );
        assert_eq!(
            kind(ctl.configuration().element(path, 0)),
            ErrorKind::InvalidInput,
            "element of `{path}`"
        );
    }
    assert_usable(&mut ctl);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn an_index_past_the_end_is_invalid_input_and_does_not_poison() {
    let mut ctl = with_threads("3");
    // clingo itself accepts 3 and 4 and wraps a huge offset to element 0, so
    // only clingox's own check keeps these from returning a path.
    for index in [3, 4, 100, usize::MAX / 2, usize::MAX] {
        assert_eq!(
            kind(ctl.configuration().element("solver", index)),
            ErrorKind::InvalidInput,
            "index {index}"
        );
    }
    // An array with no element yet has no valid index at all.
    assert_eq!(
        kind(ctl.configuration().element("tester.solver", 0)),
        ErrorKind::InvalidInput
    );
    let err = ctl.configuration().element("solver", 3).unwrap_err();
    assert!(err.to_string().contains("solver"), "{err}");
    assert_eq!(ctl.configuration().len("solver").unwrap(), 3);
    // The control is not poisoned: it still solves with three solvers.
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert_eq!(models.len(), 1);
}

#[test]
fn an_unknown_path_is_a_runtime_error() {
    let mut ctl = Control::new().unwrap();
    for path in ["nosuch", "solver.0.nosuch", "solver.x"] {
        assert_eq!(
            kind(ctl.configuration().description(path)),
            ErrorKind::Runtime
        );
        assert_eq!(kind(ctl.configuration().len(path)), ErrorKind::Runtime);
        assert_eq!(
            kind(ctl.configuration().element(path, 0)),
            ErrorKind::Runtime
        );
        assert_eq!(
            kind(ctl.configuration().has_key(path, "seed")),
            ErrorKind::Runtime
        );
    }
    let err = ctl.configuration().description("nosuch").unwrap_err();
    assert!(err.to_string().contains("nosuch"), "{err}");
    assert_usable(&mut ctl);
}

#[test]
fn a_nul_in_a_path_or_key_is_rejected() {
    let mut ctl = Control::new().unwrap();
    assert_eq!(
        kind(ctl.configuration().description("solve\0")),
        ErrorKind::Nul
    );
    assert_eq!(kind(ctl.configuration().len("solver\0")), ErrorKind::Nul);
    assert_eq!(
        kind(ctl.configuration().element("solver\0", 0)),
        ErrorKind::Nul
    );
    assert_eq!(
        kind(ctl.configuration().has_key("solve\0", "models")),
        ErrorKind::Nul
    );
    assert_eq!(
        kind(ctl.configuration().has_key("solve", "mod\0els")),
        ErrorKind::Nul
    );
    assert_usable(&mut ctl);
}

#[test]
fn the_introspection_methods_refuse_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let mut conf = ctl.configuration();
    assert_eq!(kind(conf.description("solve")), ErrorKind::Poisoned);
    assert_eq!(kind(conf.len("solver")), ErrorKind::Poisoned);
    assert_eq!(kind(conf.element("solver", 0)), ErrorKind::Poisoned);
    assert_eq!(kind(conf.has_key("", "solve")), ErrorKind::Poisoned);
    assert_eq!(kind(conf.set("solve.models", "0")), ErrorKind::Poisoned);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn an_element_path_reads_the_same_value_as_the_numbered_path() {
    let mut ctl = with_threads("3");
    let mut conf = ctl.configuration();
    // Make the elements differ, so a mix-up of indices shows.
    conf.set("solver.1.seed", "7").unwrap();
    conf.set("solver.2.seed", "11").unwrap();
    let expected = [Some("1"), Some("7"), Some("11")];
    assert_eq!(conf.len("solver").unwrap(), expected.len());
    for (index, want) in expected.iter().enumerate() {
        let path = conf.element("solver", index).unwrap();
        let by_element = conf.get(&format!("{path}.seed")).unwrap();
        assert_eq!(
            by_element,
            conf.get(&format!("solver.{index}.seed")).unwrap()
        );
        assert_eq!(by_element.as_deref(), *want, "{path}");
        assert_eq!(
            conf.description(&format!("{path}.seed")).unwrap(),
            SEED_TEXT
        );
        assert!(conf.has_key(&path, "seed").unwrap());
    }
}

#[test]
fn the_array_size_follows_the_arguments() {
    let mut ctl = Control::new().unwrap();
    assert_eq!(ctl.configuration().len("solver").unwrap(), 1);
    assert_eq!(ctl.configuration().len("tester.solver").unwrap(), 0);
    let mut ctl = Control::new().unwrap();
    assert_eq!(ctl.configuration().len("solver").unwrap(), 1);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn the_array_size_follows_parallel_arguments() {
    // `--parallel-mode=3,split` runs three threads in one solver
    // configuration.
    let mut ctl = Control::with_args(["--parallel-mode=3,split"]).unwrap();
    assert_eq!(ctl.configuration().len("solver").unwrap(), 1);
    let mut ctl = Control::with_args(["--configuration=many", "-t", "4"]).unwrap();
    assert_eq!(ctl.configuration().len("solver").unwrap(), 4);
    assert_eq!(
        ctl.configuration().element("solver", 3).unwrap(),
        "solver.3"
    );
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn setting_an_element_past_the_end_grows_the_array() {
    let mut ctl = with_threads("2");
    let mut conf = ctl.configuration();
    assert_eq!(conf.len("solver").unwrap(), 2);
    assert_eq!(kind(conf.element("solver", 5)), ErrorKind::InvalidInput);
    // Setting an element of the array, not naming it, is what grows it.
    conf.set("solver.1.seed", "7").unwrap();
    assert_eq!(conf.len("solver").unwrap(), 2);
    conf.set("solver.5.seed", "9").unwrap();
    assert_eq!(conf.len("solver").unwrap(), 6);
    assert_eq!(conf.element("solver", 5).unwrap(), "solver.5");
    assert_eq!(conf.get("solver.5.seed").unwrap().as_deref(), Some("9"));
    // The tester array is separate and starts at the first `tester` option.
    assert_eq!(conf.len("tester.solver").unwrap(), 0);
    conf.set("tester.solver.seed", "4").unwrap();
    assert_eq!(conf.len("tester.solver").unwrap(), 1);
    assert_eq!(conf.element("tester.solver", 0).unwrap(), "tester.solver.0");
    assert_eq!(conf.len("solver").unwrap(), 6);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build is single-threaded"
)]
fn introspection_works_after_a_solve() {
    let mut ctl = Control::with_args(["-t", "2", "--models=0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_exhausted());
    assert_eq!(models.len(), 4);
    let conf = ctl.configuration();
    assert_eq!(conf.len("solver").unwrap(), 2);
    assert_eq!(conf.element("solver", 1).unwrap(), "solver.1");
    assert_eq!(conf.description("solver.1.seed").unwrap(), SEED_TEXT);
    assert!(conf.has_key("solver.1", "seed").unwrap());
}

#[test]
fn element_checks_that_the_path_it_builds_resolves_to_the_element() {
    // In clingo `solver.` and `solver..` both name the array (map_at drops
    // the empty names), so `len` accepts them. But `solver...0` does not
    // resolve while `solver..0` does, so the path built from the second
    // base would fail later: `element` refuses it, and (pyclingo probe, notes
    // Q14) keeps the tolerant base unchanged.
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    let conf = ctl.configuration();
    assert_eq!(conf.len("solver..").unwrap(), 1);
    assert_eq!(kind(conf.element("solver..", 0)), ErrorKind::InvalidInput);
    assert_eq!(conf.len("solver.").unwrap(), 1);
    let path = conf.element("solver.", 0).unwrap();
    assert_eq!(path, "solver..0");
    assert_eq!(
        conf.get(&format!("{path}.seed")).unwrap(),
        conf.get("solver.0.seed").unwrap()
    );
    // The ordinary cases are unchanged.
    assert_eq!(conf.element("solver", 0).unwrap(), "solver.0");
    assert_eq!(
        conf.element("tester.solver.", 0).map_err(|e| e.kind()),
        Err(ErrorKind::InvalidInput)
    );
}

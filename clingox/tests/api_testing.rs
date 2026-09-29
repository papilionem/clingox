//! Testing helpers: `clingox::testing::parse_answer` and `assert_models!`.
//!
//! The models of each program were checked against the Python module `clingo`
//! 5.8.2.

#![forbid(unsafe_code)]

use std::any::Any;
use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

use clingox::testing::{assert_models, parse_answer};
use clingox::{Control, ErrorKind, OwnedModel, Part, Symbol};

fn term(text: &str) -> Symbol {
    text.parse().expect("the test term parses")
}

fn all_models(program: &str) -> Vec<OwnedModel> {
    let mut ctl = Control::new().expect("a control can be created");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    let (result, models) = ctl.solve_all().expect("the search succeeds");
    assert!(result.is_exhausted(), "{result}");
    models
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else {
        String::from("<a panic payload that is not text>")
    }
}

/// Runs `f`, which must panic, and returns the panic message.
fn panics_with(f: impl FnOnce()) -> String {
    let payload = panic::catch_unwind(AssertUnwindSafe(f)).expect_err("the assertion must fail");
    panic_message(payload.as_ref())
}

// ---------------------------------------------------------------------------
// parse_answer

#[test]
fn parse_answer_splits_an_answer_line_into_terms() {
    assert_eq!(
        parse_answer("a p(1) -q").unwrap(),
        vec![term("a"), term("p(1)"), term("-q")]
    );
    // The order of the line is kept, and nothing is removed.
    assert_eq!(
        parse_answer("b a b").unwrap(),
        vec![term("b"), term("a"), term("b")]
    );
    assert_eq!(
        parse_answer("  42\t\"x\"  #sup (1,2)\n").unwrap(),
        vec![
            Symbol::number(42),
            Symbol::string("x").unwrap(),
            Symbol::supremum(),
            term("(1,2)")
        ]
    );
}

#[test]
fn parse_answer_keeps_spaces_inside_strings_and_parentheses() {
    assert_eq!(
        parse_answer(r#"p(1, 2) s("a b") t("x\" y)") u((1, 2), f( g ))"#).unwrap(),
        vec![
            term("p(1,2)"),
            Symbol::function("s", &[Symbol::string("a b").unwrap()]).unwrap(),
            Symbol::function("t", &[Symbol::string("x\" y)").unwrap()]).unwrap(),
            term("u((1,2),f(g))"),
        ]
    );
}

#[test]
fn parse_answer_of_an_empty_line_is_empty() {
    assert_eq!(parse_answer("").unwrap(), Vec::<Symbol>::new());
    assert_eq!(parse_answer(" \t\n").unwrap(), Vec::<Symbol>::new());
}

#[test]
fn parse_answer_rejects_what_clingo_cannot_parse() {
    for line in ["p(", "a p(1", "p)", "X", "a \"open", "p(1,)", "a :- b"] {
        let err = parse_answer(line).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Parse, "{line:?}: {err}");
    }
}

#[test]
fn parse_answer_reads_back_what_a_model_prints() {
    // Shown: `7 -r q(-3) s((1,2)) p(1,"a b")`.
    let models = all_models(r#"p(1,"a b"). q(-3). -r. s((1,2)). #show 7."#);
    assert_eq!(models.len(), 1);
    let line = models[0]
        .symbols()
        .iter()
        .map(Symbol::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(parse_answer(&line).unwrap(), models[0].symbols());
}

// ---------------------------------------------------------------------------
// assert_models!

#[test]
fn the_sketch_example_passes() {
    let models = all_models("a :- not b. b :- not a.");
    assert_models!(models, ["a", "b"]);
    assert_eq!(models[0].symbols(), parse_answer("a").unwrap().as_slice());
}

#[test]
fn assert_models_ignores_the_order_of_models_and_symbols() {
    let models = all_models("{a; b}. p(1) :- a, b.");
    assert_models!(models, ["p(1) b a", "", "b", "a"]);
    assert_models!(models, ["a", "a b p(1)", "b", ""]);
}

#[test]
fn assert_models_accepts_an_empty_list_for_no_models() {
    let models = all_models("a. :- a.");
    assert!(models.is_empty());
    assert_models!(models, []);
}

#[test]
fn assert_models_borrows_the_models() {
    let models = all_models("a.");
    assert_models!(models, ["a"]);
    assert_models!(&models, ["a"]);
    assert_models!(models.as_slice(), ["a"]);
    let expected = String::from("a");
    assert_models!(models, [expected.as_str()]);
    assert_eq!(models.len(), 1);
}

#[test]
fn assert_models_reports_missing_and_unexpected_models() {
    let models = all_models("{a; b}. :- a, b.");
    // The models are {}, {a} and {b}.
    let message = panics_with(|| assert_models!(models, ["a", "c", "", "b d"]));
    assert!(
        message.contains("models differ (expected 4, found 3)"),
        "{message}"
    );
    assert!(message.contains("missing: c"), "{message}");
    assert!(message.contains("missing: b d"), "{message}");
    assert!(message.contains("unexpected: b"), "{message}");
    assert!(!message.contains("missing: a"), "{message}");
    assert!(!message.contains("unexpected: a"), "{message}");
}

#[test]
fn assert_models_shows_an_empty_model() {
    let models = all_models("{a}.");
    let message = panics_with(|| assert_models!(models, ["a"]));
    assert!(message.contains("unexpected: (empty)"), "{message}");
    let message = panics_with(|| assert_models!(models, ["a", "", ""]));
    assert!(message.contains("missing: (empty)"), "{message}");
}

#[test]
fn assert_models_prints_models_in_symbol_order() {
    let models = all_models("a.");
    let message = panics_with(|| assert_models!(models, ["p(2) b p(1)"]));
    assert!(message.contains("missing: b p(1) p(2)"), "{message}");
    assert!(message.contains("unexpected: a"), "{message}");
}

#[test]
fn assert_models_counts_models() {
    let models = all_models("a.");
    let message = panics_with(|| assert_models!(models, ["a", "a"]));
    assert!(message.contains("missing: a"), "{message}");
    assert!(!message.contains("unexpected"), "{message}");
}

#[test]
fn assert_models_compares_whole_models() {
    let models = all_models("a. b.");
    let message = panics_with(|| assert_models!(models, ["a"]));
    assert!(message.contains("missing: a"), "{message}");
    assert!(message.contains("unexpected: a b"), "{message}");
}

#[test]
#[should_panic(expected = "p(")]
fn assert_models_panics_on_an_expected_line_that_does_not_parse() {
    let models = all_models("a.");
    assert_models!(models, ["p("]);
}

thread_local! {
    static LAST_PANIC: RefCell<Option<(String, u32)>> = const { RefCell::new(None) };
}

/// Records the location of every panic on the panicking thread, then defers
/// to the hook that was installed before.
fn record_panic_locations() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if let Some(location) = info.location() {
                let place = (location.file().to_owned(), location.line());
                LAST_PANIC.with(|last| *last.borrow_mut() = Some(place));
            }
            previous(info);
        }));
    });
}

#[test]
fn assert_models_reports_the_callers_line() {
    record_panic_locations();
    let models = all_models("a.");
    let line = line!() + 1;
    let result = panic::catch_unwind(AssertUnwindSafe(|| assert_models!(models, ["b"])));
    assert!(result.is_err(), "assert_models! must fail");
    let (file, reported) = LAST_PANIC
        .with(|last| last.borrow_mut().take())
        .expect("assert_models! panicked");
    assert!(file.ends_with("api_testing.rs"), "{file}");
    assert_eq!(reported, line);
}

// ---------------------------------------------------------------------------
// a NUL byte keeps its kind

#[test]
fn parse_answer_reports_a_nul_byte_as_nul() {
    let err = parse_answer("a b\0c").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    let err = parse_answer("p(\"x\0\")").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
}

#[test]
fn assert_models_is_reached_through_the_testing_module_only() {
    // The crate-root path `clingox::assert_models!` is gone ;
    // `tests/ui/assert_models_at_the_crate_root.rs` checks that it does not
    // compile. The module path works with and without a `use`.
    let models = all_models("a.");
    clingox::testing::assert_models!(models, ["a"]);
}

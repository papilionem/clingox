//! Reading models: numbers, costs, symbols by show type, lookups, snapshots
//! and their text forms.
//!
//! Expected values were checked against the Python module `clingo` 5.8.2.

#![forbid(unsafe_code)]

use std::fmt::{Debug, Display};
use std::hash::Hash;

use clingox::{Control, Model, OwnedModel, Part, ShowType, Symbol};

fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn symbols(texts: &[&str]) -> Vec<Symbol> {
    texts
        .iter()
        .map(|t| t.parse().expect("the test term parses"))
        .collect()
}

fn sorted(mut symbols: Vec<Symbol>) -> Vec<Symbol> {
    symbols.sort();
    symbols
}

fn constant(name: &str) -> Symbol {
    Symbol::function(name, &[]).expect("the name has no NUL byte")
}

/// Runs `f` on the only model of `program`, solved under `assumptions`.
fn with_only_model<T>(
    program: &str,
    assumptions: &[clingox::Assumption],
    f: impl FnOnce(&Model) -> T,
) -> T {
    let mut ctl = grounded(&["--models=0"], program);
    let mut handle = ctl.solve_yield(assumptions).expect("the search starts");
    let value = f(handle
        .next_model()
        .expect("the search succeeds")
        .expect("the program has a model"));
    assert!(
        handle.next_model().expect("the search succeeds").is_none(),
        "the program has exactly one model"
    );
    value
}

/// A program whose shown symbols differ from its atoms: `hidden` and `q` are
/// hidden atoms, `t(1)`, `t(2)` and `42` are shown terms, and `r` is false
/// under the assumption the tests make.
const SHOW_PROGRAM: &str = "p(2). p(1). q :- p(1). {r}. hidden. \
                            #show p/1. #show r/0. #show 42. #show t(X) : p(X).";

fn r_false() -> [clingox::Assumption; 1] {
    [(constant("r"), false).into()]
}

#[test]
fn model_numbers_count_from_one_in_each_solve() {
    let mut ctl = grounded(&["--models=0"], "{a}.");
    for _ in 0..2 {
        let mut handle = ctl.solve_yield(&[]).unwrap();
        let mut numbers = Vec::new();
        while let Some(model) = handle.next_model().unwrap() {
            numbers.push(model.number());
            assert!(numbers.len() <= 2, "{{a}} has two models");
        }
        assert_eq!(numbers, [1, 2]);
    }
}

#[test]
fn shown_symbols_follow_the_show_directives() {
    let shown = with_only_model(SHOW_PROGRAM, &r_false(), |m| {
        m.symbols(ShowType::SHOWN).unwrap()
    });
    assert_eq!(
        sorted(shown),
        symbols(&["42", "p(1)", "p(2)", "t(1)", "t(2)"])
    );
}

#[test]
fn atoms_include_hidden_atoms_and_exclude_terms() {
    let atoms = with_only_model(SHOW_PROGRAM, &r_false(), |m| {
        m.symbols(ShowType::ATOMS).unwrap()
    });
    assert_eq!(sorted(atoms), symbols(&["hidden", "q", "p(1)", "p(2)"]));
}

#[test]
fn terms_are_the_shown_terms() {
    let terms = with_only_model(SHOW_PROGRAM, &r_false(), |m| {
        m.symbols(ShowType::TERMS).unwrap()
    });
    assert_eq!(sorted(terms), symbols(&["42", "t(1)", "t(2)"]));
}

#[test]
fn show_types_combine() {
    let both = with_only_model(SHOW_PROGRAM, &r_false(), |m| {
        m.symbols(ShowType::ATOMS | ShowType::TERMS).unwrap()
    });
    assert_eq!(
        sorted(both),
        symbols(&["42", "hidden", "q", "p(1)", "p(2)", "t(1)", "t(2)"])
    );
}

#[test]
fn complement_selects_the_false_atoms() {
    let false_atoms = with_only_model(SHOW_PROGRAM, &r_false(), |m| {
        m.symbols(ShowType::ATOMS | ShowType::COMPLEMENT).unwrap()
    });
    assert_eq!(false_atoms, symbols(&["r"]));
}

#[test]
fn symbols_come_in_clingo_order_not_sorted() {
    // clingo 5.8.2 reports the shown symbols of this model as
    // `p(2) p(1) t(2) t(1) 42`, which is not `Symbol`'s order.
    let shown = with_only_model(SHOW_PROGRAM, &r_false(), |m| {
        m.symbols(ShowType::SHOWN).unwrap()
    });
    assert_eq!(shown, symbols(&["p(2)", "p(1)", "t(2)", "t(1)", "42"]));
}

#[test]
fn contains_looks_up_atoms_whether_shown_or_not() {
    let answers = with_only_model(SHOW_PROGRAM, &r_false(), |m| {
        [
            "hidden", "q", "p(2)", // atoms in the model, hidden or shown
            "r",    // an atom of the program that is false
            "t(1)", // a shown term, which is not an atom
            "zz",   // not an atom of the program
        ]
        .map(|t| m.contains(t.parse().unwrap()).unwrap())
    });
    assert_eq!(answers, [true, true, true, false, false, false]);
}

#[test]
fn a_model_without_optimisation_has_no_cost() {
    let (cost, proven) = with_only_model("a.", &[], |m| {
        (m.cost().unwrap(), m.optimality_proven().unwrap())
    });
    assert_eq!(cost, []);
    assert!(!proven);
}

#[test]
fn costs_are_listed_highest_priority_first() {
    let cost = with_only_model("a. b. :~ a. [2@2] :~ b. [5@1]", &[], |m| m.cost().unwrap());
    assert_eq!(cost, [2, 5]);
}

#[test]
fn a_yielded_model_does_not_report_proven_optimality() {
    // Even the only, hence optimal, model reports false while it is yielded;
    // clingo sets the flag only on the model it returns after the search.
    let proven = with_only_model("a. b. :~ a. [2@2] :~ b. [5@1]", &[], |m| {
        m.optimality_proven().unwrap()
    });
    assert!(!proven);
}

#[test]
fn a_snapshot_keeps_everything_after_the_search_moves_on() {
    let mut ctl = grounded(&["--models=0"], "p(1). {q}. hidden. #show p/1. #show q/0.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle.next_model().unwrap().expect("a first model");
    let snapshot = model.snapshot().unwrap();
    let shown = sorted(model.symbols(ShowType::SHOWN).unwrap());
    let atoms = sorted(model.symbols(ShowType::ATOMS).unwrap());
    let text = model.to_string();
    assert!(handle.next_model().unwrap().is_some(), "a second model");
    assert!(handle.next_model().unwrap().is_none());
    drop(handle);

    assert_eq!(snapshot.number(), 1);
    assert_eq!(snapshot.symbols(), shown.as_slice());
    assert_eq!(snapshot.all_atoms(), atoms.as_slice());
    assert_eq!(snapshot.cost(), []);
    assert!(!snapshot.optimality_proven());
    assert_eq!(snapshot.to_string(), text);
    // The two models of the program, whichever came first.
    assert!(
        snapshot.symbols() == symbols(&["p(1)"]) || snapshot.symbols() == symbols(&["q", "p(1)"]),
        "{snapshot:?}"
    );
    assert!(snapshot.contains(constant("hidden")));
}

#[test]
fn a_snapshot_sorts_its_symbols() {
    let snapshot = with_only_model(SHOW_PROGRAM, &r_false(), |m| m.snapshot().unwrap());
    assert_eq!(
        snapshot.symbols(),
        symbols(&["42", "p(1)", "p(2)", "t(1)", "t(2)"])
    );
    assert_eq!(
        snapshot.all_atoms(),
        symbols(&["hidden", "q", "p(1)", "p(2)"])
    );
}

#[test]
fn a_snapshot_answers_contains_like_its_model() {
    let queries = symbols(&["hidden", "q", "p(2)", "r", "t(1)", "zz"]);
    let (from_model, snapshot) = with_only_model(SHOW_PROGRAM, &r_false(), |m| {
        let answers: Vec<bool> = queries.iter().map(|&s| m.contains(s).unwrap()).collect();
        (answers, m.snapshot().unwrap())
    });
    let from_snapshot: Vec<bool> = queries.iter().map(|&s| snapshot.contains(s)).collect();
    assert_eq!(from_snapshot, from_model);
}

#[test]
fn a_snapshot_keeps_the_cost() {
    let snapshot = with_only_model("a. b. :~ a. [2@2] :~ b. [5@1]", &[], |m| {
        m.snapshot().unwrap()
    });
    assert_eq!(snapshot.cost(), [2, 5]);
    assert!(!snapshot.optimality_proven());
}

#[test]
fn models_print_as_answer_lines_with_sorted_symbols() {
    let (text, snapshot) = with_only_model(SHOW_PROGRAM, &r_false(), |m| {
        (m.to_string(), m.snapshot().unwrap())
    });
    assert_eq!(text, "Answer 1: 42 p(1) p(2) t(1) t(2)");
    assert_eq!(snapshot.to_string(), text);
}

#[test]
fn an_empty_model_prints_without_symbols() {
    let text = with_only_model("a. #show.", &[], ToString::to_string);
    assert_eq!(text, "Answer 1:");
}

#[test]
fn debug_shows_the_symbols_of_a_model() {
    let (text, snapshot) = with_only_model("p(7). #show p/1.", &[], |m| {
        (format!("{m:?}"), m.snapshot().unwrap())
    });
    assert!(text.contains("p(7)"), "{text}");
    let text = format!("{snapshot:?}");
    assert!(text.contains("p(7)"), "{text}");
}

#[test]
fn snapshots_of_the_same_model_are_equal() {
    let (first, second) = with_only_model(SHOW_PROGRAM, &r_false(), |m| {
        (m.snapshot().unwrap(), m.snapshot().unwrap())
    });
    assert_eq!(first, second);
    assert_eq!(first.clone(), second);
}

#[test]
fn owned_models_can_leave_the_thread() {
    fn owned_traits<T: Send + Sync + Clone + Eq + Hash + Debug + Display + 'static>() {}
    owned_traits::<OwnedModel>();
}

#[test]
fn show_types_are_flags() {
    fn flag_traits<T: Copy + Eq + Hash + Debug>() {}
    flag_traits::<ShowType>();
    let both = ShowType::ATOMS | ShowType::TERMS;
    assert!(both.contains(ShowType::ATOMS));
    assert!(both.contains(ShowType::TERMS));
    assert!(!both.contains(ShowType::SHOWN));
    assert!(ShowType::ALL.contains(ShowType::SHOWN | ShowType::ATOMS | ShowType::TERMS));
    assert!(ShowType::ALL.contains(ShowType::THEORY));
    assert!(!ShowType::ALL.contains(ShowType::COMPLEMENT));
}

// ---------------------------------------------------------------------------
// `ShowType` is clingox's own type , and models compare by
// their symbols

#[test]
fn show_types_combine_with_or_and_or_assign() {
    let mut show = ShowType::ATOMS;
    show |= ShowType::TERMS;
    assert_eq!(show, ShowType::ATOMS | ShowType::TERMS);
    assert!(show.contains(ShowType::ATOMS | ShowType::TERMS));
    assert!(!show.contains(ShowType::ATOMS | ShowType::SHOWN));
    assert_eq!(
        ShowType::ALL,
        ShowType::SHOWN | ShowType::ATOMS | ShowType::TERMS | ShowType::THEORY
    );
    assert_ne!(ShowType::ATOMS, ShowType::TERMS);
}

#[test]
fn show_type_debug_names_its_flags() {
    assert_eq!(format!("{:?}", ShowType::ATOMS), "ShowType(ATOMS)");
    assert_eq!(
        format!("{:?}", ShowType::ATOMS | ShowType::TERMS),
        "ShowType(ATOMS | TERMS)"
    );
    assert_eq!(
        format!("{:?}", ShowType::ATOMS | ShowType::COMPLEMENT),
        "ShowType(ATOMS | COMPLEMENT)"
    );
}

#[test]
fn models_with_different_atoms_are_not_equal() {
    let mut ctl = grounded(&[], "{a;b}. :- a, b.");
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 3);
    // `{a}` and `{b}` have the same number of symbols, so only comparing
    // the symbols themselves tells them apart.
    let a = models.iter().find(|m| m.contains(constant("a"))).unwrap();
    let b = models.iter().find(|m| m.contains(constant("b"))).unwrap();
    assert_eq!(a.symbols().len(), b.symbols().len());
    assert_ne!(a.symbols(), b.symbols());
    assert_ne!(a, b);
    assert_eq!(a.symbols(), symbols(&["a"]));
    assert_ne!(a.symbols(), symbols(&["b"]));
}

// ---------------------------------------------------------------------------
// What the rustdoc of `optimality_proven`, `number` and `solve_optimal`
// must say under `--opt-mode=optN`

#[test]
fn under_opt_n_the_enumerated_optima_are_proven_and_numbered_again() {
    // Python clingo 5.8.2 with `--opt-mode=optN --models=0`: the search phase
    // reports model 1 (not proven, cost [0]), and the enumeration phase starts
    // numbering again, reporting model 1 (proven, cost [0]).
    let mut ctl = grounded(
        &["--opt-mode=optN", "--models=0"],
        "{a;b}. :~ a. [1] :~ b. [1]",
    );
    let mut seen = Vec::new();
    let result = ctl
        .for_each_model(&[], |model| {
            seen.push((model.number(), model.optimality_proven()?, model.cost()?));
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(result.is_exhausted());
    assert_eq!(seen, [(1, false, vec![0]), (1, true, vec![0])]);
}

//! Reading models as Rust values: `atoms::<T>()` and `shown::<T>()` on `Model`
//! and `OwnedModel`.
//!
//! The shown symbols and atoms of every program below were checked against the
//! Python module `clingo` 5.8.2 (`Model.symbols(shown=True)` and
//! `Model.symbols(atoms=True)`).

#![forbid(unsafe_code)]

use std::ops::ControlFlow;

use clingox::{Control, ErrorKind, FromSymbol, Model, Outcome, OwnedModel, Part, Symbol, ToSymbol};

#[derive(FromSymbol, Debug, PartialEq)]
struct P(i32);

#[derive(FromSymbol, Debug, PartialEq)]
#[clingo(name = "p")]
struct P2(i32, i32);

#[derive(FromSymbol, Debug, PartialEq)]
struct T(i32);

#[derive(FromSymbol, Debug, PartialEq)]
struct Q(#[clingo(constant)] String);

#[derive(FromSymbol, Debug, PartialEq)]
struct Done;

#[derive(FromSymbol, Debug, PartialEq)]
struct Pair<A>(A, A);

/// A fact type for the tests below.
#[derive(ToSymbol)]
struct Asserted {
    #[clingo(constant)]
    object: String,
    #[clingo(constant)]
    property: String,
    value: i32,
}

/// A result type for the tests below.
#[derive(FromSymbol, Debug, PartialEq)]
struct Violation {
    #[clingo(string)]
    rule: String,
    #[clingo(constant)]
    culprit: String,
}

/// One model. Shown: `q(a) t(1) t(2) t(10)`. Atoms: `-p(3) p(1) p(1,2) p(2)
/// p(10) pp(4) q(a)`. The facts for `p/1` are given out of order.
const SHOW_PROGRAM: &str = "p(10). p(2). p(1). -p(3). p(1,2). pp(4). q(a). \
                            #show q/1. #show t(X) : p(X).";

fn grounded(program: &str) -> Control {
    let mut ctl = Control::with_args(["--models=0"]).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// The only model of `program`, as an `OwnedModel`.
fn only_owned_model(program: &str) -> OwnedModel {
    let (result, mut models) = grounded(program).solve_all().expect("the search succeeds");
    assert!(result.is_sat() && result.is_exhausted(), "{result}");
    assert_eq!(models.len(), 1, "the program has exactly one model");
    models.pop().expect("one model")
}

/// Runs `f` on the only model of `program`, lent by the solve handle.
fn with_only_model<R>(program: &str, f: impl FnOnce(&Model) -> R) -> R {
    let mut ctl = grounded(program);
    let mut handle = ctl.solve_yield(&[]).expect("the search starts");
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

#[test]
fn atoms_reads_every_atom_of_the_predicate_including_hidden_ones() {
    let expected = vec![P(1), P(2), P(10)];
    assert_eq!(
        only_owned_model(SHOW_PROGRAM).atoms::<P>().unwrap(),
        expected
    );
    with_only_model(SHOW_PROGRAM, |model| {
        assert_eq!(model.atoms::<P>().unwrap(), expected);
    });
}

#[test]
fn shown_reads_only_shown_symbols_including_terms() {
    let owned = only_owned_model(SHOW_PROGRAM);
    assert_eq!(owned.shown::<P>().unwrap(), Vec::<P>::new());
    assert_eq!(owned.shown::<T>().unwrap(), vec![T(1), T(2), T(10)]);
    assert_eq!(owned.shown::<Q>().unwrap(), vec![Q("a".into())]);
    // `t/1` is shown as a term, and is not an atom.
    assert_eq!(owned.atoms::<T>().unwrap(), Vec::<T>::new());

    with_only_model(SHOW_PROGRAM, |model| {
        assert_eq!(model.shown::<P>().unwrap(), Vec::<P>::new());
        assert_eq!(model.shown::<T>().unwrap(), vec![T(1), T(2), T(10)]);
        assert_eq!(model.atoms::<T>().unwrap(), Vec::<T>::new());
    });
}

#[test]
fn atoms_skips_other_names_and_arities() {
    let owned = only_owned_model(SHOW_PROGRAM);
    // `p(1,2)` and `pp(4)` are not `p/1`; `p(1,2)` is `p/2`.
    assert_eq!(owned.atoms::<P>().unwrap().len(), 3);
    assert_eq!(owned.atoms::<P2>().unwrap(), vec![P2(1, 2)]);
    assert_eq!(
        only_owned_model("pair(1,2). other(1).")
            .atoms::<P>()
            .unwrap(),
        Vec::<P>::new()
    );
}

#[test]
fn atoms_skips_classical_negation() {
    let owned = only_owned_model(SHOW_PROGRAM);
    let negated: Symbol = "-p(3)".parse().unwrap();
    assert!(owned.all_atoms().contains(&negated));
    assert!(!owned.atoms::<P>().unwrap().contains(&P(3)));
    assert_eq!(
        only_owned_model("-p(1). -p(2).").atoms::<P>().unwrap(),
        Vec::<P>::new()
    );
}

#[test]
fn a_matching_atom_that_does_not_convert_is_a_conversion_error() {
    let owned = only_owned_model("p(a). p(1).");
    let err = owned.atoms::<P>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert!(err.to_string().contains("p(a)"), "{err}");
    assert_eq!(
        owned.shown::<P>().unwrap_err().kind(),
        ErrorKind::Conversion
    );

    with_only_model("p(a). p(1).", |model| {
        let err = model.atoms::<P>().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Conversion);
        assert!(err.to_string().contains("p(a)"), "{err}");
        assert_eq!(
            model.shown::<P>().unwrap_err().kind(),
            ErrorKind::Conversion
        );
    });
}

#[test]
fn a_string_does_not_match_a_constant_field() {
    // clingo 5.8.2: `q("a")` does not satisfy `q(a)`, so `r` is not derived.
    let owned = only_owned_model(r#"q("a"). r :- q(a)."#);
    assert!(!owned.contains("r".parse().unwrap()));
    // Read with a constant field, the string is an error, not a silent skip.
    let err = owned.atoms::<Q>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert!(err.to_string().contains(r#"q("a")"#), "{err}");
}

#[test]
fn shown_fails_on_a_shown_term_that_does_not_convert() {
    // Shown: `t("x") t(1)`, where `t("x")` is a term. Atoms: `t(1)`.
    let program = r#"t(1). #show t/1. #show t("x")."#;
    let owned = only_owned_model(program);
    assert_eq!(owned.atoms::<T>().unwrap(), vec![T(1)]);
    let err = owned.shown::<T>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert!(err.to_string().contains(r#"t("x")"#), "{err}");

    with_only_model(program, |model| {
        assert_eq!(model.atoms::<T>().unwrap(), vec![T(1)]);
        assert_eq!(
            model.shown::<T>().unwrap_err().kind(),
            ErrorKind::Conversion
        );
    });
}

#[test]
fn owned_models_read_the_same_as_models() {
    let mut ctl = grounded("{a; b}. p(1) :- a. p(2) :- b. p(10) :- a, b. t(X) :- p(X).");
    let mut seen = 0;
    let result = ctl
        .for_each_model(&[], |model| {
            let snapshot = model.snapshot()?;
            assert_eq!(model.atoms::<P>()?, snapshot.atoms::<P>()?);
            assert_eq!(model.shown::<P>()?, snapshot.shown::<P>()?);
            assert_eq!(model.atoms::<T>()?, snapshot.atoms::<T>()?);
            seen += 1;
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(result.is_exhausted(), "{result}");
    assert_eq!(seen, 4);
}

#[test]
fn results_are_in_symbol_order() {
    // The facts are given as p(10), p(2), p(1); Symbol's order is 1, 2, 10.
    let program = "p(10). p(2). p(1). #show p/1.";
    assert_eq!(
        only_owned_model(program).atoms::<P>().unwrap(),
        vec![P(1), P(2), P(10)]
    );
    with_only_model(program, |model| {
        assert_eq!(model.shown::<P>().unwrap(), vec![P(1), P(2), P(10)]);
    });
}

#[test]
fn a_unit_struct_reads_a_constant_atom() {
    // `#show.` hides every atom.
    let owned = only_owned_model("done. #show.");
    assert_eq!(owned.atoms::<Done>().unwrap(), vec![Done]);
    assert_eq!(owned.shown::<Done>().unwrap(), Vec::<Done>::new());
}

#[test]
fn generic_predicates_read_by_their_name() {
    let owned = only_owned_model("pair(1,2). pair(3,4). other(1).");
    assert_eq!(
        owned.atoms::<Pair<i32>>().unwrap(),
        vec![Pair(1, 2), Pair(3, 4)]
    );

    let mixed = only_owned_model("pair(1,2). pair(a,b).");
    assert_eq!(
        mixed.atoms::<Pair<i32>>().unwrap_err().kind(),
        ErrorKind::Conversion
    );
    assert_eq!(mixed.atoms::<Pair<Symbol>>().unwrap().len(), 2);
}

#[test]
fn the_checker_example_finds_the_violation() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base(
        r#"violation("floor3_enc", O) :- asserted(O, encrypted, 0), asserted(O, floor, 3)."#,
    )
    .unwrap();
    ctl.add_facts([
        Asserted {
            object: "comp13".into(),
            property: "encrypted".into(),
            value: 0,
        },
        Asserted {
            object: "comp13".into(),
            property: "floor".into(),
            value: 3,
        },
    ])
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let violations = match ctl.solve_first().unwrap() {
        Outcome::Sat(model, _) => model.atoms::<Violation>().unwrap(),
        Outcome::Unsat => panic!("the facts contradict the rules"),
        Outcome::Unknown(result) => panic!("undecided: {result}"),
    };
    assert_eq!(
        violations,
        vec![Violation {
            rule: "floor3_enc".into(),
            culprit: "comp13".into(),
        }]
    );
}

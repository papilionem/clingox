//! Trees whose nodes are of the wrong kind, through the public API.
//!
//! Oracle: pyclingo 5.8.2 (`clingo.ast` constructors, which do not check kinds,
//! `ProgramBuilder.add` and `clingo_ast_unpool` through
//! `clingo._internal._lib`). The fuzz that measured them (53929 mutated trees)
//! found the ten slot and replacement pairs used below. The tests need a real
//! clingo, so none of them runs under Miri.
//!
//! Three groups:
//!
//! - the refusals `add` makes (kind `Parse`, no messages, the control
//!   poisoned), and `unpool`, which never refuses;
//! - ten slot and replacement pairs with their outcomes: `add` refuses, or
//!   accepts and `ground` fails with `Runtime`, or accepts and grounds;
//! - the `External` trees that crashed `ground` in clingo 5.8.2 (a partial
//!   function term over a variable, `|X|`, `X\2` or `X**2`, as the type or as
//!   the atom's symbol). They are fixed at their root in the vendored clingo
//!   (upstream entry U35), so these tests expect a clean result. Each runs in a
//!   child process, so a crash is a failed test and not a dead test binary.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::too_many_lines,
    reason = "a scenario reads better in one piece"
)]

use clingox::ast::{
    self, Ast, AstType, Attribute, BinaryOperator, LiteralSign, Span, UnaryOperator, Unpool,
};
use clingox::testing::assert_models;
use clingox::{Control, ErrorKind, Part, Symbol};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn span() -> Span {
    Span::new("<t>", 1, 1, "<t>", 1, 2).unwrap()
}

/// The statements `text` parses to, starting with the implicit
/// `#program base.`.
fn statements(text: &str) -> Vec<Ast> {
    let mut all = Vec::new();
    ast::parse_string(text, |statement| {
        all.push(statement);
        Ok(())
    })
    .unwrap();
    all
}

fn is_poisoned(ctl: &Control) -> bool {
    format!("{ctl:?}").contains("poisoned")
}

/// A well-formed rule, to add next to a bad one.
fn good_rule() -> Ast {
    statements("a.").remove(1)
}

// ---------------------------------------------------------------------------
// The refusals
// ---------------------------------------------------------------------------

/// The trees `add` refuses.
///
/// Oracle: code 1 (`clingo_error_runtime`), no logged message, for a
/// `Function` term ("invalid ast: statement expected"), a `Rule` used as a
/// literal's atom, terms as body literals ("invalid ast: body literal
/// expected"), a `Program` as a head ("invalid ast: head literal expected"),
/// a `Variable` as an atom ("atom expected") and a `Program` as an atom
/// ("invalid ast: atom expected").
fn refused_trees() -> Vec<(&'static str, Ast)> {
    let location = span();
    let rule = statements("p(X) :- q(X).").remove(1);
    let function = rule
        .ast(Attribute::Head)
        .unwrap()
        .ast(Attribute::Atom)
        .unwrap()
        .ast(Attribute::Symbol)
        .unwrap();
    assert_eq!(function.ast_type(), AstType::Function);
    let variable = function.ast_at(Attribute::Arguments, 0).unwrap();
    assert_eq!(variable.ast_type(), AstType::Variable);
    let program = statements("#program foo.").remove(1);
    assert_eq!(program.ast_type(), AstType::Program);
    let fact = good_rule();

    let rule_as_atom = ast::rule(
        &location,
        &ast::literal(&location, LiteralSign::NoSign, &fact).unwrap(),
        &[],
    )
    .unwrap();
    let terms_as_body = ast::rule(
        &location,
        &rule.ast(Attribute::Head).unwrap(),
        std::slice::from_ref(&function),
    )
    .unwrap();
    let program_as_head = ast::rule(&location, &program, &[]).unwrap();
    let variable_as_atom = ast::rule(
        &location,
        &ast::literal(
            &location,
            LiteralSign::NoSign,
            &ast::symbolic_atom(&variable).unwrap(),
        )
        .unwrap(),
        &[],
    )
    .unwrap();
    let program_as_atom = ast::rule(
        &location,
        &ast::literal(&location, LiteralSign::NoSign, &program).unwrap(),
        &[],
    )
    .unwrap();
    vec![
        ("a function term", function),
        ("a rule as a literal's atom", rule_as_atom),
        ("terms as body literals", terms_as_body),
        ("a program as a head", program_as_head),
        ("a variable as an atom", variable_as_atom),
        ("a program as an atom", program_as_atom),
    ]
}

/// `add` refuses each with `Parse` and no messages, and the control is
/// poisoned afterwards.
#[test]
fn add_refuses_a_mis_kinded_tree_as_parse_and_poisons() {
    for (name, bad) in refused_trees() {
        let mut ctl = Control::new().unwrap();
        let err = ctl
            .with_program_builder(|builder| builder.add(&bad))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Parse, "{name}: {err:?}");
        assert!(err.messages().is_empty(), "{name}: {err:?}");
        assert!(is_poisoned(&ctl), "{name}");
        assert_eq!(
            ctl.ground(&[Part::base()]).unwrap_err().kind(),
            ErrorKind::Poisoned,
            "{name}"
        );
    }
}

/// The tree stays usable after `add` refused it: it prints, and edits still
/// work.
#[test]
fn a_refused_tree_stays_valid() {
    for (name, bad) in refused_trees() {
        let mut ctl = Control::new().unwrap();
        let _ = ctl.with_program_builder(|builder| builder.add(&bad));
        let text = bad.to_string();
        assert!(!text.is_empty(), "{name}");
        assert_eq!(bad.to_string(), text, "{name}");
    }
}

/// `unpool` accepts all of them: no error, and (having no pool) one call with
/// the input itself.
///
/// Oracle: no error and no crash for any of the 53929 trees; `unpool`
/// walks attributes by name and never inspects kinds it does not know.
#[test]
fn unpool_accepts_every_mis_kinded_tree() {
    for (name, bad) in refused_trees() {
        let mut calls = 0;
        bad.unpool(Unpool::ALL, |alternative| {
            calls += 1;
            assert!(alternative.ptr_eq(&bad), "{name}: nothing to unpool");
            Ok(())
        })
        .unwrap_or_else(|err| panic!("{name}: {err:?}"));
        assert_eq!(calls, 1, "{name}");
    }
}

// ---------------------------------------------------------------------------
// Ten slot and replacement pairs
// ---------------------------------------------------------------------------

/// A step from a node to the child holding the slot.
#[derive(Clone, Copy)]
enum Step {
    Attr(Attribute),
    At(Attribute, usize),
}

/// What replaces the slot's node.
#[derive(Clone, Copy, Debug)]
enum Replacement {
    Variable,
    Function,
    Rule,
    Program,
}

impl Replacement {
    /// A fresh node of that kind, from the sources the oracle used
    /// (`mk` in `cases.py`).
    fn node(self) -> Ast {
        match self {
            Replacement::Rule => statements("z.").remove(1),
            Replacement::Function => statements("f.")
                .remove(1)
                .ast(Attribute::Head)
                .unwrap()
                .ast(Attribute::Atom)
                .unwrap()
                .ast(Attribute::Symbol)
                .unwrap(),
            Replacement::Program => statements("#program foo.").remove(1),
            Replacement::Variable => statements("p(X).")
                .remove(1)
                .ast(Attribute::Head)
                .unwrap()
                .ast(Attribute::Atom)
                .unwrap()
                .ast(Attribute::Symbol)
                .unwrap()
                .ast_at(Attribute::Arguments, 0)
                .unwrap(),
        }
    }
}

/// What `cases.py` printed for a pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// `add` raised.
    Refused,
    /// `add` accepted and `ground` raised.
    GroundError,
    /// `add` accepted and `ground` succeeded.
    Grounds,
}

struct Case {
    name: &'static str,
    source: &'static str,
    /// Steps from the statement to the node holding the slot.
    path: &'static [Step],
    /// The slot in that node: `Attr` for a node child, `At` for an array
    /// element.
    slot: Step,
    replacement: Replacement,
    outcome: Outcome,
}

/// Oracle: `cases.py` (phase 2 transcripts in the notes), pyclingo 5.8.2:
///
/// ```text
/// argument_is_variable         ground-error
/// argument_is_function         grounds
/// argument_is_rule             refused
/// external_type_is_function    grounds
/// external_type_is_program     refused
/// symbol_is_variable           refused
/// show_term_is_function        grounds
/// show_term_is_variable        ground-error
/// body_atom_symbol_is_variable refused
/// weight_is_function           grounds
/// ```
fn cases() -> Vec<Case> {
    use Attribute as A;
    use Outcome::{GroundError, Grounds, Refused};
    use Replacement::{Function, Program, Rule, Variable};
    use Step::{At, Attr};
    const HEAD_SYMBOL: &[Step] = &[Attr(A::Head), Attr(A::Atom), Attr(A::Symbol)];
    vec![
        Case {
            name: "argument_is_variable",
            source: "p(1) :- q.",
            path: HEAD_SYMBOL,
            slot: At(A::Arguments, 0),
            replacement: Variable,
            outcome: GroundError,
        },
        Case {
            name: "argument_is_function",
            source: "p(1) :- q.",
            path: HEAD_SYMBOL,
            slot: At(A::Arguments, 0),
            replacement: Function,
            outcome: Grounds,
        },
        Case {
            name: "argument_is_rule",
            source: "p(1) :- q.",
            path: HEAD_SYMBOL,
            slot: At(A::Arguments, 0),
            replacement: Rule,
            outcome: Refused,
        },
        Case {
            name: "external_type_is_function",
            source: "#external e. [true]",
            path: &[],
            slot: Attr(A::ExternalType),
            replacement: Function,
            outcome: Grounds,
        },
        Case {
            name: "external_type_is_program",
            source: "#external e. [true]",
            path: &[],
            slot: Attr(A::ExternalType),
            replacement: Program,
            outcome: Refused,
        },
        Case {
            name: "symbol_is_variable",
            source: "p(1) :- q.",
            path: &[Attr(A::Head), Attr(A::Atom)],
            slot: Attr(A::Symbol),
            replacement: Variable,
            outcome: Refused,
        },
        Case {
            name: "show_term_is_function",
            source: "#show t : q.",
            path: &[],
            slot: Attr(A::Term),
            replacement: Function,
            outcome: Grounds,
        },
        Case {
            name: "show_term_is_variable",
            source: "#show t : q.",
            path: &[],
            slot: Attr(A::Term),
            replacement: Variable,
            outcome: GroundError,
        },
        Case {
            name: "body_atom_symbol_is_variable",
            source: "p :- q.",
            path: &[At(A::Body, 0), Attr(A::Atom)],
            slot: Attr(A::Symbol),
            replacement: Variable,
            outcome: Refused,
        },
        Case {
            name: "weight_is_function",
            source: "#minimize {1,a : b}.",
            path: &[],
            slot: Attr(A::Weight),
            replacement: Function,
            outcome: Grounds,
        },
    ]
}

fn mutated(case: &Case) -> Ast {
    let statement = statements(case.source).remove(1);
    let mut node = statement.clone();
    for step in case.path {
        node = match *step {
            Step::Attr(attribute) => node.ast(attribute).unwrap(),
            Step::At(attribute, index) => node.ast_at(attribute, index).unwrap(),
        };
    }
    let replacement = case.replacement.node();
    match case.slot {
        Step::Attr(attribute) => node.set_ast(attribute, &replacement).unwrap(),
        Step::At(attribute, index) => node.set_ast_at(attribute, index, &replacement).unwrap(),
    }
    statement
}

/// Each pair ends as the oracle's did.
#[test]
fn the_slot_and_replacement_pairs_end_as_the_oracle_did() {
    for case in cases() {
        let statement = mutated(&case);
        let mut ctl = Control::new().unwrap();
        let added = ctl.with_program_builder(|builder| builder.add(&statement));
        match case.outcome {
            Outcome::Refused => {
                let err = added.unwrap_err();
                assert_eq!(err.kind(), ErrorKind::Parse, "{}: {err:?}", case.name);
                assert!(err.messages().is_empty(), "{}: {err:?}", case.name);
                assert!(is_poisoned(&ctl), "{}", case.name);
            }
            Outcome::GroundError => {
                added.unwrap_or_else(|err| panic!("{}: {err:?}", case.name));
                let err = ctl.ground(&[Part::base()]).unwrap_err();
                assert_eq!(err.kind(), ErrorKind::Runtime, "{}: {err:?}", case.name);
                assert!(!is_poisoned(&ctl), "{}: a failed ground", case.name);
            }
            Outcome::Grounds => {
                added.unwrap_or_else(|err| panic!("{}: {err:?}", case.name));
                ctl.ground(&[Part::base()])
                    .unwrap_or_else(|err| panic!("{}: {err:?}", case.name));
                assert!(!is_poisoned(&ctl), "{}", case.name);
            }
        }
    }
}

/// None of the ten trees stops `unpool`.
///
/// Oracle: .
#[test]
fn unpool_accepts_the_ten_pairs() {
    for case in cases() {
        let statement = mutated(&case);
        let mut calls = 0;
        statement
            .unpool(Unpool::ALL, |_| {
                calls += 1;
                Ok(())
            })
            .unwrap_or_else(|err| panic!("{}: {err:?}", case.name));
        assert_eq!(calls, 1, "{}", case.name);
    }
}

// ---------------------------------------------------------------------------
// External types that are variable-dependent terms and do not crash
// ---------------------------------------------------------------------------

fn x() -> Ast {
    ast::variable(&span(), "X").unwrap()
}

fn number(n: i32) -> Ast {
    ast::symbolic_term(&span(), Symbol::number(n)).unwrap()
}

/// `f(X)` as a body literal.
fn f_of_x() -> Ast {
    let location = span();
    let f = ast::function(&location, "f", &[x()], false).unwrap();
    ast::literal(
        &location,
        LiteralSign::NoSign,
        &ast::symbolic_atom(&f).unwrap(),
    )
    .unwrap()
}

/// `#external e(X) : f(X). [<external_type>]`.
fn external_with_type(external_type: &Ast) -> Ast {
    let location = span();
    let e = ast::function(&location, "e", &[x()], false).unwrap();
    let atom = ast::symbolic_atom(&e).unwrap();
    ast::external(&location, &atom, &[f_of_x()], external_type).unwrap()
}

/// Adds the fact `f(1).` and `external`, grounds, and returns the models.
fn ground_with(external: &Ast) -> Vec<clingox::OwnedModel> {
    let mut ctl = Control::new().unwrap();
    let fact = statements("f(1).").remove(1);
    ctl.with_program_builder(|builder| {
        builder.add(&fact)?;
        builder.add(external)
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.solve_all().unwrap().1
}

/// A variable-dependent external type that is not a partial function term
/// grounds, and the external is left unassigned (false by default), as from
/// text.
///
/// Oracle: "text `external_type` ...": `[X]`, `[X+1]`, `[-X]`, `[1..X]`,
/// `[X*2]` (and the constant types) do not crash; `f(1). #external e(X) :
/// f(X). [X+1]` grounds and has the single model `f(1)`.
#[test]
fn a_variable_dependent_external_type_grounds_and_leaves_the_external_false() {
    let location = span();
    let types: Vec<(&str, Ast)> = vec![
        ("X", x()),
        (
            "X+1",
            ast::binary_operation(&location, BinaryOperator::Plus, &x(), &number(1)).unwrap(),
        ),
        (
            "-X",
            ast::unary_operation(&location, UnaryOperator::Minus, &x()).unwrap(),
        ),
        ("1..X", ast::interval(&location, &number(1), &x()).unwrap()),
        (
            "X*2",
            ast::binary_operation(&location, BinaryOperator::Multiplication, &x(), &number(2))
                .unwrap(),
        ),
    ];
    for (name, external_type) in types {
        let models = ground_with(&external_with_type(&external_type));
        assert_eq!(models.len(), 1, "[{name}]");
        assert_models!(models, ["f(1)"]);
    }
}

// ---------------------------------------------------------------------------
// The External trees that crashed `ground`, in a child process
// ---------------------------------------------------------------------------

/// The trees that segfaulted `ground` in clingo 5.8.2, and the
/// text form of the type crash. The crash is fixed at
/// its root (U35), so each must end cleanly.
///
/// The tests run their case in a child process (this binary run again with
/// [`CHILD`] set), so a crash is a failed test, not a dead test binary.
/// Spawning needs a host with processes, so the group is host-only (iOS has
/// no `fork`/`exec` for an app or test executable).
#[cfg(not(any(target_os = "android", target_os = "ios", target_family = "wasm")))]
mod crash_cases {
    use super::*;
    use std::io::{Read, Write};
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    /// The environment variable that puts this binary in child mode, with the
    /// case as its value.
    const CHILD: &str = "CLINGOX_TEST_MISKINDED_CHILD";
    const RESULT: &str = "MISKINDED-RESULT";
    /// A child gets this long; a hang is a failure as much as a crash.
    const LIMIT: Duration = Duration::from_secs(60);

    fn op(kind: &str, operand: &Ast) -> Ast {
        let location = span();
        match kind {
            "abs" => ast::unary_operation(&location, UnaryOperator::Absolute, operand).unwrap(),
            "mod" => ast::binary_operation(&location, BinaryOperator::Modulo, operand, &number(2))
                .unwrap(),
            "pow" => ast::binary_operation(&location, BinaryOperator::Power, operand, &number(2))
                .unwrap(),
            other => panic!("unknown operation {other}"),
        }
    }

    /// The tree of a case named `<slot>-<operation>`: `type-*` puts the term
    /// in `external_type`, `atom-*` in the atom's `symbol`, `bare-*` is the
    /// body-less `#external x. [<term>]` of the first fuzz pass.
    fn tree(case: &str) -> Ast {
        let location = span();
        let (slot, kind) = case.split_once('-').unwrap();
        let term = op(kind, &x());
        match slot {
            "type" => external_with_type(&term),
            "atom" => {
                let atom = ast::symbolic_atom(&term).unwrap();
                let truth =
                    ast::symbolic_term(&location, Symbol::function("true", &[]).unwrap()).unwrap();
                ast::external(&location, &atom, &[f_of_x()], &truth).unwrap()
            }
            "bare" => {
                let x_atom =
                    ast::symbolic_atom(&ast::function(&location, "x", &[], false).unwrap())
                        .unwrap();
                ast::external(&location, &x_atom, &[], &term).unwrap()
            }
            other => panic!("unknown slot {other}"),
        }
    }

    /// What the child reports: the outcome of `ground` and, if it returned,
    /// the models.
    fn run_case(case: &str) -> String {
        let mut ctl = Control::new().unwrap();
        if let Some(text) = case.strip_prefix("text:") {
            ctl.add_base(&format!("f(1). {text}")).unwrap();
        } else {
            let fact = statements("f(1).").remove(1);
            let external = tree(case);
            ctl.with_program_builder(|builder| {
                builder.add(&fact)?;
                builder.add(&external)
            })
            .unwrap();
        }
        match ctl.ground(&[Part::base()]) {
            Ok(()) => {
                let (_, models) = ctl.solve_all().unwrap();
                let lines: Vec<String> = models
                    .iter()
                    .map(|model| {
                        let mut atoms: Vec<String> =
                            model.symbols().iter().map(ToString::to_string).collect();
                        atoms.sort();
                        atoms.join(" ")
                    })
                    .collect();
                format!("ok [{}]", lines.join(" | "))
            }
            Err(err) => format!("err {:?} poisoned={} {err}", err.kind(), is_poisoned(&ctl)),
        }
    }

    /// The child. In a normal run of this binary the variable is unset and
    /// the test does nothing.
    #[test]
    fn child_grounds_the_case() {
        let Ok(case) = std::env::var(CHILD) else {
            return;
        };
        let result = run_case(&case);
        // The harness prints "test ... " first, so start a fresh line.
        let mut out = std::io::stdout().lock();
        writeln!(out, "\n{RESULT} {result}").expect("stdout is a pipe to the parent test");
    }

    /// Runs `case` in a child and returns what it reported.
    fn in_child(case: &str) -> String {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "crash_cases::child_grounds_the_case",
                "--exact",
                "--nocapture",
            ])
            .args(["--test-threads=1"])
            .env(CHILD, case)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() > LIMIT {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{case}: the child did not finish within {LIMIT:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut stdout = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut stdout)
            .unwrap();
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        // A crash is a signal on Unix; elsewhere it is a failing exit status,
        // which the `success` check below reports.
        #[cfg(unix)]
        assert!(
            status.signal().is_none(),
            "{case}: the child died of signal {:?}\nstdout: {stdout}\nstderr: {stderr}",
            status.signal()
        );
        assert!(status.success(), "{case}: {status}\n{stdout}\n{stderr}");
        stdout
            .lines()
            .find_map(|line| line.split_once(RESULT).map(|(_, rest)| rest))
            .unwrap_or_else(|| panic!("{case}: no result line\n{stdout}\n{stderr}"))
            .trim()
            .to_owned()
    }

    /// The type term is bound by the condition: `ground` succeeds, the
    /// external is never assigned, and the only model is `f(1)` (U35 notes,
    /// Q3; the neighbours `[X+1]` and `[X]` give the same).
    fn assert_grounds_without_the_external(case: &str) {
        assert_eq!(in_child(case), "ok [f(1)]", "{case}");
    }

    /// The atom's symbol is an operator over a variable: libgringo refuses it
    /// at `ground` with `Logic`, "`Term::getSig` must not be called on
    /// `VarTerm`". Oracle: pyclingo 5.8.2 gives error code 2 (logic) with the
    /// same text for a variable and, for `X+2`, "... on `LinearTerm`". A logic
    /// error poisons the control, as everywhere else.
    fn assert_refused_as_logic(case: &str) {
        let result = in_child(case);
        assert!(
            result.starts_with("err Logic poisoned=true ")
                && result.contains("Term::getSig must not be called"),
            "{case}: {result}"
        );
    }

    #[test]
    fn external_type_abs_of_a_variable_does_not_crash_ground() {
        assert_grounds_without_the_external("type-abs");
    }

    #[test]
    fn external_type_modulo_of_a_variable_does_not_crash_ground() {
        assert_grounds_without_the_external("type-mod");
    }

    #[test]
    fn external_type_power_of_a_variable_does_not_crash_ground() {
        assert_grounds_without_the_external("type-pow");
    }

    #[test]
    fn external_atom_symbol_abs_of_a_variable_does_not_crash_ground() {
        assert_refused_as_logic("atom-abs");
    }

    #[test]
    fn external_atom_symbol_modulo_of_a_variable_does_not_crash_ground() {
        assert_refused_as_logic("atom-mod");
    }

    #[test]
    fn external_atom_symbol_power_of_a_variable_does_not_crash_ground() {
        assert_refused_as_logic("atom-pow");
    }

    #[test]
    fn a_bodyless_external_with_a_modulo_type_does_not_crash_ground() {
        // The variable is unbound: the safety check reports it, as it does for
        // `[X+2]`, and a `Runtime` error from `ground` does not poison.
        let result = in_child("bare-mod");
        assert!(
            result.starts_with("err Runtime poisoned=false ")
                && result.contains("unsafe variables"),
            "{result}"
        );
    }

    /// The same crash from plain text through `Control::add`, which the fix
    /// covers as well.
    #[test]
    fn the_text_form_of_the_external_type_crash_is_clean_too() {
        assert_grounds_without_the_external("text:#external e(X) : f(X). [X\\2]");
        assert_grounds_without_the_external("text:#external e(X) : f(X). [|X|]");
    }

    /// The child protocol itself: a case that must succeed does, and reports
    /// its models. This is what makes the tests above mean something (a
    /// harness that reports nothing would fail them, not pass them).
    #[test]
    fn the_child_reports_a_case_that_grounds() {
        assert_eq!(in_child("text:#external e(X) : f(X). [X+1]"), "ok [f(1)]");
        assert_eq!(
            in_child("text:#external e(X) : f(X). [true]"),
            "ok [e(1) f(1)]"
        );
    }
}

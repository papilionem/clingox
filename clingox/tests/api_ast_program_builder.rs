//! The program builder: `Control::with_program_builder` and
//! `ProgramBuilder::add`.
//!
//! Oracle: pyclingo 5.8.2 (`clingo.ast.ProgramBuilder`, and the
//! `clingo_program_builder_*` functions called directly through
//! `clingo._internal._lib` where pyclingo hides an error code). Every
//! expected model and error below was produced by that session, including the
//! rewrite oracle.
//! The tests need a real clingo, so none of them runs under Miri.
//!
//! What the tests pin, in order: the round trip, where statements land, what
//! `add` refuses, the state a logged error leaves, how the session ends when
//! the closure fails or panics, leftovers from other
//! sessions, and the end-to-end rewrite.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::too_many_lines,
    reason = "a scenario reads better in one piece"
)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use clingox::ast::{self, Ast, AstType, Attribute, LiteralSign, Span, Visitor, walk};
use clingox::backend::Head;
use clingox::testing::assert_models;
use clingox::{Control, Error, ErrorKind, MessageCode, Part, Result, Symbol};

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

/// Adds every statement of `text` through one builder session.
fn add_text(ctl: &mut Control, text: &str) -> Result<()> {
    let all = statements(text);
    ctl.with_program_builder(|builder| {
        for statement in &all {
            builder.add(statement)?;
        }
        Ok(())
    })
}

fn ground_base(ctl: &mut Control) {
    ctl.ground(&[Part::base()]).unwrap();
}

type Log = Arc<Mutex<Vec<(MessageCode, String)>>>;

/// A control whose logger records the text of every message.
fn recording() -> (Control, Log) {
    let log: Log = Arc::default();
    let sink = Arc::clone(&log);
    let ctl = Control::builder()
        .logger(move |code, text: &str| sink.lock().unwrap().push((code, text.to_owned())))
        .build()
        .unwrap();
    (ctl, log)
}

fn is_poisoned(ctl: &Control) -> bool {
    format!("{ctl:?}").contains("poisoned")
}

fn sym(text: &str) -> Symbol {
    text.parse().expect("the term parses")
}

/// A well-formed rule, to add next to a bad one.
fn good_rule() -> Ast {
    statements("a.").remove(1)
}

// ---------------------------------------------------------------------------
// The round trip
// ---------------------------------------------------------------------------

/// begin, add, end, then ground and solve give the models the same program
/// gives as text.
///
/// Oracle: "basic begin/add/end + ground/solve": `a. b :- a.` added node
/// by node, models `['a b']`.
#[test]
fn statements_added_through_the_builder_are_grounded_and_solved() {
    let mut ctl = Control::new().unwrap();
    add_text(&mut ctl, "a. b :- a.").unwrap();
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b"]);
}

/// A typical use: parse, collect (here unchanged but for a
/// guard), add later inside the session.
///
/// Oracle: the same program in pyclingo with a `Transformer` appending
/// `not c` to every non-empty body: models `a b`.
#[test]
fn statements_collected_first_are_added_in_a_later_session() {
    let mut rewritten: Vec<Ast> = Vec::new();
    ast::parse_string("a :- b. b.", |statement| {
        rewritten.push(statement);
        Ok(())
    })
    .unwrap();
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|builder| {
        for statement in &rewritten {
            builder.add(statement)?;
        }
        Ok(())
    })
    .unwrap();
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b"]);
}

/// The closure's value is the call's value.
#[test]
fn the_closures_value_is_returned() {
    let mut ctl = Control::new().unwrap();
    let all = statements("a.");
    let count = ctl
        .with_program_builder(|builder| {
            for statement in &all {
                builder.add(statement)?;
            }
            Ok(all.len())
        })
        .unwrap();
    assert_eq!(count, 2, "the implicit `#program base.` and the fact");
}

/// A session that adds nothing changes nothing, and the control still works.
///
/// Oracle: "end without begin" and "begin twice": an empty session leaves a
/// control that grounds normally.
#[test]
fn a_session_with_no_statements_is_a_no_op() {
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|_| Ok(())).unwrap();
    assert!(!is_poisoned(&ctl));
    ctl.add_base("a.").unwrap();
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a"]);
}

/// Sessions can follow one another, and text may be mixed in between them.
#[test]
fn two_sessions_and_a_text_add_all_count() {
    let mut ctl = Control::new().unwrap();
    add_text(&mut ctl, "a.").unwrap();
    ctl.add_base("b :- a.").unwrap();
    add_text(&mut ctl, "c :- b.").unwrap();
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b c"]);
}

/// Every statement kind a parse produces is accepted (no error, nothing
/// logged).
///
/// Oracle: "add with parse-produced statement kinds all accepted": the 16
/// nodes below all returned success and the logger stayed empty.
#[test]
fn every_statement_kind_the_parser_produces_is_accepted() {
    let (mut ctl, log) = recording();
    let source = "#const n=3. p(X,Y) :- q(X), not r(Y). #show p/2. #show t : q. #project p/1. \
        #external e. [true] :~ a. [1@2] #edge (a,b) : c. #heuristic h : p. [1, sign] \
        #defined d/1. #program p(k). \
        #theory t { term { + : 1, binary, left }; &a/0 : term, any }. &a { 1 } . \
        { a } = 1. a ; b :- c.";
    let all = statements(source);
    assert_eq!(all.len(), 16);
    let mut kinds = Vec::new();
    ctl.with_program_builder(|builder| {
        for statement in &all {
            kinds.push(statement.ast_type());
            builder.add(statement)?;
        }
        Ok(())
    })
    .unwrap();
    assert!(kinds.contains(&AstType::Program));
    assert!(kinds.contains(&AstType::Rule));
    assert!(kinds.contains(&AstType::Definition));
    assert!(kinds.contains(&AstType::External));
    assert!(kinds.contains(&AstType::TheoryDefinition));
    assert!(log.lock().unwrap().is_empty(), "{:?}", log.lock().unwrap());
    assert!(!is_poisoned(&ctl));
}

// ---------------------------------------------------------------------------
// Where statements land
// ---------------------------------------------------------------------------

/// Statements added before any `#program` go to `base`, without the parse's
/// own `#program base.` node.
///
/// Oracle: "no #program before statements": the fact alone, then
/// `ground([("base", [])])`, models `['a']`.
#[test]
fn statements_without_a_program_directive_go_to_base() {
    let mut ctl = Control::new().unwrap();
    let fact = good_rule();
    ctl.with_program_builder(|builder| builder.add(&fact))
        .unwrap();
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a"]);
}

/// A `#program p(k).` node selects the part, and `k` takes the value the
/// part is grounded with.
///
/// Oracle: "statement for a program part": `#program p(k). q(k).` then
/// `ground([("p", [Number(7)])])`, models `['q(7)']`.
#[test]
fn a_program_node_selects_the_part() {
    let mut ctl = Control::new().unwrap();
    add_text(&mut ctl, "#program p(k). q(k).").unwrap();
    ctl.ground(&[Part::new("p", &[Symbol::number(7)]).unwrap()])
        .unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["q(7)"]);
}

/// The same part, with the `#program` node built by the AST constructors
/// rather than parsed.
///
/// Oracle: as above; the built node prints as `#program p(k).`.
#[test]
fn a_built_program_node_selects_the_part_too() {
    let location = span();
    let parameter = ast::id(&location, "k").unwrap();
    let program = ast::program(&location, "p", &[parameter]).unwrap();
    assert_eq!(program.to_string(), "#program p(k).");
    let fact = statements("q(k).").remove(1);
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|builder| {
        builder.add(&program)?;
        builder.add(&fact)
    })
    .unwrap();
    ctl.ground(&[Part::new("p", &[Symbol::number(7)]).unwrap()])
        .unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["q(7)"]);
}

/// The current part persists across sessions: the second session's fact goes
/// to the part the first one selected.
///
/// Oracle: "statements added in two sessions": `#program q. z.` then, in a
/// second session, `a.`; `ground([("q", [])])` gives `['z a']`.
#[test]
fn the_current_part_persists_across_sessions() {
    let mut ctl = Control::new().unwrap();
    add_text(&mut ctl, "#program q. z.").unwrap();
    let second = good_rule();
    ctl.with_program_builder(|builder| builder.add(&second))
        .unwrap();
    ctl.ground(&[Part::new("q", &[]).unwrap()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a z"]);
}

/// Statements can be added after a ground and grounded again.
///
/// Oracle: "ground with no program, builder used after ground": grounding
/// `base` twice with a session in between gives `['a b']`.
#[test]
fn statements_can_be_added_after_a_ground() {
    let mut ctl = Control::new().unwrap();
    add_text(&mut ctl, "#program p. a.").unwrap();
    ctl.ground(&[Part::new("p", &[]).unwrap()]).unwrap();
    add_text(&mut ctl, "#program q. b :- a.").unwrap();
    ctl.ground(&[Part::new("q", &[]).unwrap()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b"]);
}

// ---------------------------------------------------------------------------
// `add` copies
// ---------------------------------------------------------------------------

/// Editing a node after it was added, inside the session or after it, does
/// not change the program.
///
/// Oracle: "mutate node after add; add twice": the head of an added rule
/// replaced afterwards, models unchanged `['a b']`.
#[test]
fn editing_a_node_after_add_does_not_change_the_program() {
    let all = statements("a. b :- a.");
    let replacement = statements("zzz.").remove(1).ast(Attribute::Head).unwrap();
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|builder| {
        builder.add(&all[1])?;
        builder.add(&all[2])?;
        all[2].set_ast(Attribute::Head, &replacement)?;
        Ok(())
    })
    .unwrap();
    all[1].set_ast(Attribute::Head, &replacement).unwrap();
    assert_eq!(all[2].to_string(), "zzz :- a.", "the edit itself worked");
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b"]);
}

/// `add` takes `&Ast` and the node stays valid and usable afterwards.
#[test]
fn the_added_node_stays_valid() {
    let fact = good_rule();
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|builder| builder.add(&fact))
        .unwrap();
    assert_eq!(fact.to_string(), "a.");
    ctl.with_program_builder(|builder| builder.add(&fact))
        .unwrap();
    assert_eq!(fact.ast_type(), AstType::Rule);
}

/// An `Ast` added twice is added twice. A `#const` defined twice is a logged
/// error, so it shows the second add where a repeated fact would not.
///
/// Oracle: "mutate node after add; add twice" for the repeated rule
/// (`['a b']`), and for the message of a doubled `#const`.
#[test]
fn a_node_added_twice_is_added_twice() {
    let (mut once, once_log) = recording();
    let constant = statements("#const c=1.").remove(1);
    once.with_program_builder(|builder| builder.add(&constant))
        .unwrap();
    assert!(once_log.lock().unwrap().is_empty());
    ground_base(&mut once);

    let (mut twice, twice_log) = recording();
    twice
        .with_program_builder(|builder| {
            builder.add(&constant)?;
            builder.add(&constant)
        })
        .unwrap();
    let log = twice_log.lock().unwrap();
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0].0, MessageCode::RuntimeError);
    assert!(log[0].1.contains("redefinition of constant"), "{log:?}");
    drop(log);
    assert_eq!(
        twice.ground(&[Part::base()]).unwrap_err().kind(),
        ErrorKind::Runtime
    );

    // The repeated rule adds nothing new to the models.
    let rule = statements("a. b :- a.");
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|builder| {
        builder.add(&rule[1])?;
        builder.add(&rule[2])?;
        builder.add(&rule[2])
    })
    .unwrap();
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b"]);
}

// ---------------------------------------------------------------------------
// What `add` refuses
// ---------------------------------------------------------------------------

/// The trees `add` refuses, each with the oracle's message.
///
/// Oracle: code 1 (`clingo_error_runtime`), no logged message, for
///
/// - a `Function` term ("invalid ast: statement expected");
/// - a `Rule` used as a literal's atom ("atom expected" is the observed
///   text; see the assertions on the kind only);
/// - terms as body literals ("invalid ast: body literal expected");
/// - a `Program` as a head ("invalid ast: head literal expected");
/// - a `Variable` as an atom ("atom expected");
/// - a `Program` as an atom ("invalid ast: atom expected").
fn refused_trees() -> Vec<(&'static str, Ast)> {
    let location = span();
    // `p(X) :- q(X).`: its head atom's symbol is a Function, its argument a
    // Variable.
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

/// Each refusal is `ErrorKind::Parse` with no messages, ends the closure
/// through `?`, and poisons the control.
#[test]
fn a_tree_that_is_not_a_statement_is_refused_as_parse_and_poisons() {
    for (name, bad) in refused_trees() {
        let mut ctl = Control::new().unwrap();
        let err = ctl
            .with_program_builder(|builder| builder.add(&bad))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Parse, "{name}: {err:?}");
        assert!(err.messages().is_empty(), "{name}: {err:?}");
        assert!(is_poisoned(&ctl), "{name}");
        let after = ctl.add_base("a.").unwrap_err();
        assert_eq!(after.kind(), ErrorKind::Poisoned, "{name}");
    }
}

/// After a refusal has poisoned the control, another `add` in the same
/// closure returns `Poisoned`, and so does the call once the closure ends.
///
/// A failed `add` poisons the control; the refusal itself is checked
/// here too.
#[test]
fn a_second_add_after_a_refusal_is_poisoned() {
    for (name, bad) in refused_trees() {
        let mut ctl = Control::new().unwrap();
        let fact = good_rule();
        let mut first = None;
        let mut second = None;
        let result = ctl.with_program_builder(|builder| {
            first = Some(builder.add(&bad).unwrap_err());
            second = Some(builder.add(&fact).unwrap_err());
            Ok(())
        });
        assert_eq!(first.unwrap().kind(), ErrorKind::Parse, "{name}");
        assert_eq!(second.unwrap().kind(), ErrorKind::Poisoned, "{name}");
        // The closure swallowed both errors and returned Ok, but the control
        // is poisoned, and closing the session did not clear that.
        result.unwrap();
        assert!(is_poisoned(&ctl), "{name}");
        assert_eq!(
            ctl.ground(&[Part::base()]).unwrap_err().kind(),
            ErrorKind::Poisoned,
            "{name}"
        );
    }
}

/// A refused statement does not stop the earlier ones from being added: the
/// session still ends (the poisoned control refuses everything afterwards, so
/// this is checked through the message a later text add would have made:
/// none is logged).
#[test]
fn a_refusal_logs_nothing() {
    let (mut ctl, log) = recording();
    let (_, bad) = refused_trees().remove(0);
    let fact = good_rule();
    let _ = ctl.with_program_builder(|builder| {
        builder.add(&fact)?;
        builder.add(&bad)
    });
    assert!(log.lock().unwrap().is_empty(), "{:?}", log.lock().unwrap());
}

/// A poisoned control refuses the session without calling the closure.
#[test]
fn a_poisoned_control_refuses_the_session_without_running_it() {
    let mut ctl = Control::new().unwrap();
    assert!(ctl.add_base("this is not a program (").is_err());
    assert!(is_poisoned(&ctl));
    let mut called = false;
    let err = ctl
        .with_program_builder(|_| {
            called = true;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
    assert!(!called);
}

// ---------------------------------------------------------------------------
// Errors that are logged, not returned
// ---------------------------------------------------------------------------

/// A well-formed statement with a semantic error is accepted; the message
/// goes to the logger and `ground` reports the failure.
///
/// Oracle: and `log.py`: `#const c=1. #const c=2. a.` added through the
/// builder gives one logged `RuntimeError` message, "<string>:1:13-24: error:
/// redefinition of constant:\n  #const c=2.\n<string>:1:1-12: note: constant
/// also defined here\n", and `ground` raises "grounding stopped because of
/// errors" (code 1).
#[test]
fn a_semantic_error_is_logged_and_reported_at_ground() {
    let (mut ctl, log) = recording();
    add_text(&mut ctl, "#const c=1. #const c=2. a.").unwrap();
    {
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 1, "{log:?}");
        assert_eq!(log[0].0, MessageCode::RuntimeError);
        assert!(log[0].1.contains("redefinition of constant"), "{log:?}");
        assert!(log[0].1.contains("#const c=2."), "{log:?}");
    }
    assert!(!is_poisoned(&ctl), "the session itself succeeded");

    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
    assert!(
        err.to_string()
            .contains("grounding stopped because of errors"),
        "{err}"
    );
    assert!(!is_poisoned(&ctl), "a failed ground does not poison");
}

/// The logger's error is sticky: the next session is refused, `Parse`,
/// "parsing failed", the closure is not called, and the control is poisoned.
///
/// Oracle: "begin after logged error": `begin` fails with
/// `clingo_error_runtime`, "parsing failed".
#[test]
fn a_control_that_logged_an_error_refuses_the_next_session() {
    let (mut ctl, _) = recording();
    add_text(&mut ctl, "#const c=1. #const c=2.").unwrap();
    let mut called = false;
    let err = ctl
        .with_program_builder(|_| {
            called = true;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    assert!(err.to_string().contains("parsing failed"), "{err}");
    assert!(!called, "a refused begin does not run the closure");
    assert!(is_poisoned(&ctl));
}

/// The same sticky state refuses a text add, `Parse` and poisoned.
///
/// Oracle: "text add error then builder": after a logged error,
/// `ctl.add(...)` raises "parsing failed" too.
#[test]
fn a_control_that_logged_an_error_refuses_the_next_text_add() {
    let (mut ctl, _) = recording();
    add_text(&mut ctl, "#const c=1. #const c=2.").unwrap();
    let err = ctl.add_base("z.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    assert!(err.to_string().contains("parsing failed"), "{err}");
    assert!(is_poisoned(&ctl));
}

// ---------------------------------------------------------------------------
// The closure fails or panics
// ---------------------------------------------------------------------------

/// A closure error ends the session and passes through unchanged; the
/// statements added before it are kept and grounded by the next `ground`.
///
/// Oracle: the statements of an ended session are part of the program;
/// the closure's error handling follows `with_backend`.
#[test]
fn a_closure_error_ends_the_session_and_keeps_the_earlier_statements() {
    let mut ctl = Control::new().unwrap();
    let all = statements("a. b :- a.");
    let err = ctl
        .with_program_builder(|builder| -> Result<()> {
            builder.add(&all[1])?;
            builder.add(&all[2])?;
            Err(Error::new(ErrorKind::InvalidInput, "changed my mind"))
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(err.to_string().contains("changed my mind"), "{err}");
    assert!(!is_poisoned(&ctl), "InvalidInput does not poison");
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b"]);
}

/// A closure error of a poisoning kind poisons, and the session was still
/// ended first (the poisoned control cannot be grounded, so this is checked
/// on the kind and the poison).
#[test]
fn a_closure_error_of_a_poisoning_kind_poisons() {
    let mut ctl = Control::new().unwrap();
    let err = ctl
        .with_program_builder(|_| -> Result<()> { Err(Error::new(ErrorKind::Parse, "no")) })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    assert!(is_poisoned(&ctl));
    assert_eq!(
        ctl.ground(&[Part::base()]).unwrap_err().kind(),
        ErrorKind::Poisoned
    );
}

/// A panic in the closure unwinds through the call with its payload; the
/// control is not poisoned; and `ground`, the very next call, sees the
/// statements: the leftover session was ended by it, not lost.
///
/// Oracle: "ground while open": with the session left open, `ground`
/// grounds nothing (models `['']`) and after `end` a second `ground` gives
/// the program; the clingox contract is that the caller never sees the empty
/// program. Expected here: `["a b"]`.
#[test]
fn with_program_builder_finishes_when_the_closure_panics() {
    let mut ctl = Control::new().unwrap();
    let all = statements("a. b :- a.");
    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_program_builder(|builder| -> Result<()> {
            builder.add(&all[1])?;
            builder.add(&all[2])?;
            panic!("stop here")
        })
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"stop here"));
    assert!(!is_poisoned(&ctl));
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");

    // `ground` first and only: no other call has ended the session.
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b"]);
}

/// The same panic, and the next call is a text `add_base`: the session must
/// end *before* the text add, or the text add loses part of the session's
/// statements.
///
/// Oracle: "Control.add while open": `a. b :- a.` through the builder and
/// `z.` added as text while the session is open give `['a z']`; ended first,
/// `['a b z']`.
#[test]
fn a_text_add_after_a_panic_in_the_closure_keeps_every_statement() {
    let mut ctl = Control::new().unwrap();
    let all = statements("a. b :- a.");
    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_program_builder(|builder| -> Result<()> {
            builder.add(&all[1])?;
            builder.add(&all[2])?;
            panic!("stop here")
        })
    }));
    assert!(caught.is_err());
    ctl.add_base("z.").unwrap();
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b z"]);
}

/// The same panic, and the next call is `with_backend`: it ends the leftover
/// session, its own atom is added, and the models have both.
#[test]
fn a_backend_after_a_panic_in_the_closure_keeps_every_statement() {
    let mut ctl = Control::new().unwrap();
    let all = statements("a. b :- a.");
    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_program_builder(|builder| -> Result<()> {
            builder.add(&all[1])?;
            builder.add(&all[2])?;
            panic!("stop here")
        })
    }));
    assert!(caught.is_err());
    ctl.with_backend(|backend| {
        let f = backend.add_atom(Some(sym("f")))?;
        backend.add_rule(Head::Normal(&[f]), &[])
    })
    .unwrap();
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b f"]);
}

/// The same panic, and the next call is a `solve` before any ground: it does
/// not fail, and a `ground` after it still has every statement.
#[test]
fn a_solve_after_a_panic_in_the_closure_does_not_lose_the_statements() {
    let mut ctl = Control::new().unwrap();
    let all = statements("a. b :- a.");
    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_program_builder(|builder| -> Result<()> {
            builder.add(&all[1])?;
            builder.add(&all[2])?;
            panic!("stop here")
        })
    }));
    assert!(caught.is_err());
    let (_, before) = ctl.solve_all().unwrap();
    assert_models!(before, [""]);
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b"]);
}

/// The same panic, and the next call is a read through `&self`
/// (`symbolic_atoms`): it ends the session too, and the atoms grounded after
/// it are complete.
#[test]
fn a_read_after_a_panic_in_the_closure_does_not_lose_the_statements() {
    let mut ctl = Control::new().unwrap();
    let all = statements("a. b :- a.");
    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_program_builder(|builder| -> Result<()> {
            builder.add(&all[1])?;
            builder.add(&all[2])?;
            panic!("stop here")
        })
    }));
    assert!(caught.is_err());
    assert_eq!(ctl.symbolic_atoms().unwrap().iter().count(), 0);
    ground_base(&mut ctl);
    let b = ctl
        .symbolic_atoms()
        .unwrap()
        .find(sym("b"))
        .unwrap()
        .expect("b is a ground atom");
    assert!(b.is_fact());
}

/// Dropping a control that still has an open session ends the session
/// quietly; under `ASan` and `LSan` this also shows nothing leaks.
#[test]
fn dropping_a_control_with_a_leftover_session_is_clean() {
    let mut ctl = Control::new().unwrap();
    let all = statements("a. b :- a.");
    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_program_builder(|builder| -> Result<()> {
            builder.add(&all[1])?;
            panic!("stop here")
        })
    }));
    assert!(caught.is_err());
    drop(ctl);
}

/// A panic in the closure and a caught unwind leave the control usable for
/// another session.
#[test]
fn a_session_after_a_panicked_session_works() {
    let mut ctl = Control::new().unwrap();
    let all = statements("a. b :- a.");
    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_program_builder(|builder| -> Result<()> {
            builder.add(&all[1])?;
            panic!("stop here")
        })
    }));
    assert!(caught.is_err());
    ctl.with_program_builder(|builder| builder.add(&all[2]))
        .unwrap();
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a b"]);
}

// ---------------------------------------------------------------------------
// Leftovers from other sessions
// ---------------------------------------------------------------------------

/// A backend session left open by a panic is finished before the program
/// builder opens.
///
/// Oracle: "builder begin while backend open" shows the C level is benign;
/// the clingox rule is S4 (every entry point finishes leftovers first).
#[test]
fn a_leftover_backend_is_finished_before_the_session() {
    let mut ctl = Control::new().unwrap();
    let caught = catch_unwind(AssertUnwindSafe(|| {
        ctl.with_backend(|backend| -> Result<()> {
            let f = backend.add_atom(Some(sym("f")))?;
            backend.add_rule(Head::Normal(&[f]), &[])?;
            panic!("stop here")
        })
    }));
    assert!(caught.is_err());
    add_text(&mut ctl, "a.").unwrap();
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    assert_models!(models, ["a f"]);
    let f = ctl
        .symbolic_atoms()
        .unwrap()
        .find(sym("f"))
        .unwrap()
        .expect("f is a ground atom");
    assert!(f.is_fact(), "the backend session was finished");
}

/// A search left open by a forgotten handle is closed before the session.
///
/// Oracle: "solve open then builder": the builder used during a live
/// `solve(yield_=True)` works at the C level; clingox closes the search first.
#[test]
fn a_leftover_search_is_closed_before_the_session() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("{x; y}.").unwrap();
    ground_base(&mut ctl);
    let mut handle = ctl.solve_yield(&[]).unwrap();
    assert!(handle.next_model().unwrap().is_some());
    std::mem::forget(handle);
    add_text(&mut ctl, "#program p. a.").unwrap();
    assert!(format!("{ctl:?}").contains("idle"), "{ctl:?}");
    ctl.ground(&[Part::new("p", &[]).unwrap()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert!(!models.is_empty());
    assert!(
        models.iter().all(|m| m.symbols().contains(&sym("a"))),
        "every model has the added fact"
    );
}

/// `Debug` does not change: `idle` between sessions.
#[test]
fn debug_is_idle_after_a_session() {
    let mut ctl = Control::new().unwrap();
    add_text(&mut ctl, "a.").unwrap();
    let text = format!("{ctl:?}");
    assert!(text.contains("idle"), "{text}");
    assert!(!text.contains("poisoned"), "{text}");
}

// ---------------------------------------------------------------------------
// The end-to-end rewrite
// ---------------------------------------------------------------------------

/// Renames every function named `p` to `pp`, as the oracle's `Transformer`
/// does.
struct Rename;

impl Visitor for Rename {
    fn visit_function(&mut self, node: &Ast) -> Result<Ast> {
        let node = walk(self, node)?;
        if node.string(Attribute::Name)? == "p" {
            let renamed = node.copy()?;
            renamed.set_string(Attribute::Name, "pp")?;
            return Ok(renamed);
        }
        Ok(node)
    }
}

/// Changes nothing.
struct Identity;

impl Visitor for Identity {}

/// Appends `not skip` to the body of every rule that has one.
struct Guard;

impl Visitor for Guard {
    fn visit_rule(&mut self, node: &Ast) -> Result<Ast> {
        if node.ast_array_len(Attribute::Body)? == 0 {
            return Ok(node.clone());
        }
        let location = span();
        let skip = ast::function(&location, "skip", &[], false)?;
        let atom = ast::symbolic_atom(&skip)?;
        let literal = ast::literal(&location, LiteralSign::Negation, &atom)?;
        let rule = node.copy()?;
        rule.push_ast(Attribute::Body, &literal)?;
        Ok(rule)
    }
}

fn rewrite_and_solve(
    visitor: &mut impl Visitor,
    text: &str,
    extra: &str,
) -> (Vec<String>, Vec<String>) {
    let mut rewritten = Vec::new();
    ast::parse_string(text, |statement| {
        rewritten.push(visitor.visit(&statement)?);
        Ok(())
    })
    .unwrap();
    let printed = rewritten.iter().map(ToString::to_string).collect();
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|builder| {
        for statement in &rewritten {
            builder.add(statement)?;
        }
        Ok(())
    })
    .unwrap();
    if !extra.is_empty() {
        ctl.add_base(extra).unwrap();
    }
    ground_base(&mut ctl);
    let (_, models) = ctl.solve_all().unwrap();
    let lines = models
        .iter()
        .map(|m| {
            let mut atoms: Vec<String> = m.symbols().iter().map(ToString::to_string).collect();
            atoms.sort();
            atoms.join(" ")
        })
        .collect();
    (printed, lines)
}

/// Parse, rewrite through a `Visitor` and the setters, add, ground, solve.
///
/// Oracle (`p2/rw.py`, in the notes): for
/// `p(1..3). q(X) :- p(X), X > 1. r(X,Y) :- q(X), p(Y), X != Y.` the original
/// gives `p(1) p(2) p(3) q(2) q(3) r(2,1) r(2,3) r(3,1) r(3,2)`; with `p`
/// renamed by a `Transformer` the statements print as
/// `pp((1..3)).`, `q(X) :- pp(X); X > 1.`, `r(X,Y) :- q(X); pp(Y); X != Y.`
/// and the model is `pp(1) pp(2) pp(3) q(2) q(3) r(2,1) r(2,3) r(3,1) r(3,2)`.
#[test]
fn a_renaming_rewrite_gives_the_oracles_models() {
    let source = "p(1..3). q(X) :- p(X), X > 1. r(X,Y) :- q(X), p(Y), X != Y.";

    // A visitor that changes nothing reproduces the original program.
    let (_, unchanged) = rewrite_and_solve(&mut Identity, source, "");
    assert_eq!(
        unchanged,
        ["p(1) p(2) p(3) q(2) q(3) r(2,1) r(2,3) r(3,1) r(3,2)"]
    );

    let (printed, renamed) = rewrite_and_solve(&mut Rename, source, "");
    assert_eq!(
        printed,
        [
            "#program base.",
            "pp((1..3)).",
            "q(X) :- pp(X); X > 1.",
            "r(X,Y) :- q(X); pp(Y); X != Y.",
        ]
    );
    assert_eq!(
        renamed,
        ["pp(1) pp(2) pp(3) q(2) q(3) r(2,1) r(2,3) r(3,1) r(3,2)"]
    );
}

/// A second rewrite that builds new nodes: a guard literal appended to every
/// body. With `skip` false the models are the original's; with `skip` true
/// the guarded rules stop firing.
///
/// Oracle (`p2/rw.py`): `a. b :- a. c :- b. d.` guarded prints
/// `#program base.`, `a.`, `b :- a; not skip.`, `c :- b; not skip.`, `d.`
/// and has the model `a b c d`; with `skip.` added as text the model is
/// `a d skip`.
#[test]
fn a_guarding_rewrite_gives_the_oracles_models() {
    let source = "a. b :- a. c :- b. d.";
    let (printed, models) = rewrite_and_solve(&mut Guard, source, "");
    assert_eq!(
        printed,
        [
            "#program base.",
            "a.",
            "b :- a; not skip.",
            "c :- b; not skip.",
            "d.",
        ]
    );
    assert_eq!(models, ["a b c d"]);

    let (_, skipped) = rewrite_and_solve(&mut Guard, source, "skip.");
    assert_eq!(skipped, ["a d skip"]);
}

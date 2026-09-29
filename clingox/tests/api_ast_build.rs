//! The generated typed constructors over `clingo_ast_build`: one test per node
//! kind, and the argument, ownership, enum and error rules they share.
//!
//! Oracle: pyclingo 5.8.2 (`clingo.ast` constructors, which call
//! `clingo_ast_build`, and `clingo._internal._lib.g_clingo_ast_constructors`
//! for the attribute table). Every expected `Display` string, and the attribute
//! table in `EXPECTED`, was produced by that session. The tests need a real
//! clingo, so none of them runs under Miri.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::cloned_ref_to_slice_refs,
    reason = "the arguments read as the constructor's own list; a clone is a reference count"
)]
#![allow(
    clippy::too_many_lines,
    reason = "the fixture builds all the node parts once, in one place"
)]

use clingox::ast::{
    self, AggregateFunction, Ast, AstType, Attribute, AttributeType, BinaryOperator, CommentType,
    ComparisonOperator, LiteralSign, Span, TheoryAtomType, TheoryOperatorType, TheorySequenceType,
    UnaryOperator,
};
use clingox::{Error, ErrorKind, Symbol};

// ---------------------------------------------------------------------------
// The oracle's constructor table
// ---------------------------------------------------------------------------

/// `g_clingo_ast_constructors`, transcribed from the oracle: for every
/// node kind, the attributes a built node has and their kinds. Every other
/// attribute must be absent.
const EXPECTED: &[(AstType, &[(Attribute, AttributeType)])] = &[
    (
        AstType::Id,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
        ],
    ),
    (
        AstType::Variable,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
        ],
    ),
    (
        AstType::SymbolicTerm,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Symbol, AttributeType::Symbol),
        ],
    ),
    (
        AstType::UnaryOperation,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::OperatorType, AttributeType::Number),
            (Attribute::Argument, AttributeType::Ast),
        ],
    ),
    (
        AstType::BinaryOperation,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::OperatorType, AttributeType::Number),
            (Attribute::Left, AttributeType::Ast),
            (Attribute::Right, AttributeType::Ast),
        ],
    ),
    (
        AstType::Interval,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Left, AttributeType::Ast),
            (Attribute::Right, AttributeType::Ast),
        ],
    ),
    (
        AstType::Function,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Arguments, AttributeType::AstArray),
            (Attribute::External, AttributeType::Number),
        ],
    ),
    (
        AstType::Pool,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Arguments, AttributeType::AstArray),
        ],
    ),
    (
        AstType::BooleanConstant,
        &[(Attribute::Value, AttributeType::Number)],
    ),
    (
        AstType::SymbolicAtom,
        &[(Attribute::Symbol, AttributeType::Ast)],
    ),
    (
        AstType::Comparison,
        &[
            (Attribute::Term, AttributeType::Ast),
            (Attribute::Guards, AttributeType::AstArray),
        ],
    ),
    (
        AstType::Guard,
        &[
            (Attribute::Comparison, AttributeType::Number),
            (Attribute::Term, AttributeType::Ast),
        ],
    ),
    (
        AstType::ConditionalLiteral,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Literal, AttributeType::Ast),
            (Attribute::Condition, AttributeType::AstArray),
        ],
    ),
    (
        AstType::Aggregate,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::LeftGuard, AttributeType::OptionalAst),
            (Attribute::Elements, AttributeType::AstArray),
            (Attribute::RightGuard, AttributeType::OptionalAst),
        ],
    ),
    (
        AstType::BodyAggregateElement,
        &[
            (Attribute::Terms, AttributeType::AstArray),
            (Attribute::Condition, AttributeType::AstArray),
        ],
    ),
    (
        AstType::BodyAggregate,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::LeftGuard, AttributeType::OptionalAst),
            (Attribute::Function, AttributeType::Number),
            (Attribute::Elements, AttributeType::AstArray),
            (Attribute::RightGuard, AttributeType::OptionalAst),
        ],
    ),
    (
        AstType::HeadAggregateElement,
        &[
            (Attribute::Terms, AttributeType::AstArray),
            (Attribute::Condition, AttributeType::Ast),
        ],
    ),
    (
        AstType::HeadAggregate,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::LeftGuard, AttributeType::OptionalAst),
            (Attribute::Function, AttributeType::Number),
            (Attribute::Elements, AttributeType::AstArray),
            (Attribute::RightGuard, AttributeType::OptionalAst),
        ],
    ),
    (
        AstType::Disjunction,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Elements, AttributeType::AstArray),
        ],
    ),
    (
        AstType::TheorySequence,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::SequenceType, AttributeType::Number),
            (Attribute::Terms, AttributeType::AstArray),
        ],
    ),
    (
        AstType::TheoryFunction,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Arguments, AttributeType::AstArray),
        ],
    ),
    (
        AstType::TheoryUnparsedTermElement,
        &[
            (Attribute::Operators, AttributeType::StringArray),
            (Attribute::Term, AttributeType::Ast),
        ],
    ),
    (
        AstType::TheoryUnparsedTerm,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Elements, AttributeType::AstArray),
        ],
    ),
    (
        AstType::TheoryGuard,
        &[
            (Attribute::OperatorName, AttributeType::String),
            (Attribute::Term, AttributeType::Ast),
        ],
    ),
    (
        AstType::TheoryAtomElement,
        &[
            (Attribute::Terms, AttributeType::AstArray),
            (Attribute::Condition, AttributeType::AstArray),
        ],
    ),
    (
        AstType::TheoryAtom,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Term, AttributeType::Ast),
            (Attribute::Elements, AttributeType::AstArray),
            (Attribute::Guard, AttributeType::OptionalAst),
        ],
    ),
    (
        AstType::Literal,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Sign, AttributeType::Number),
            (Attribute::Atom, AttributeType::Ast),
        ],
    ),
    (
        AstType::TheoryOperatorDefinition,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Priority, AttributeType::Number),
            (Attribute::OperatorType, AttributeType::Number),
        ],
    ),
    (
        AstType::TheoryTermDefinition,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Operators, AttributeType::AstArray),
        ],
    ),
    (
        AstType::TheoryGuardDefinition,
        &[
            (Attribute::Operators, AttributeType::StringArray),
            (Attribute::Term, AttributeType::String),
        ],
    ),
    (
        AstType::TheoryAtomDefinition,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::AtomType, AttributeType::Number),
            (Attribute::Name, AttributeType::String),
            (Attribute::Arity, AttributeType::Number),
            (Attribute::Term, AttributeType::String),
            (Attribute::Guard, AttributeType::OptionalAst),
        ],
    ),
    (
        AstType::Rule,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Head, AttributeType::Ast),
            (Attribute::Body, AttributeType::AstArray),
        ],
    ),
    (
        AstType::Definition,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Value, AttributeType::Ast),
            (Attribute::IsDefault, AttributeType::Number),
        ],
    ),
    (
        AstType::ShowSignature,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Arity, AttributeType::Number),
            (Attribute::Positive, AttributeType::Number),
        ],
    ),
    (
        AstType::ShowTerm,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Term, AttributeType::Ast),
            (Attribute::Body, AttributeType::AstArray),
        ],
    ),
    (
        AstType::Minimize,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Weight, AttributeType::Ast),
            (Attribute::Priority, AttributeType::Ast),
            (Attribute::Terms, AttributeType::AstArray),
            (Attribute::Body, AttributeType::AstArray),
        ],
    ),
    (
        AstType::Script,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Code, AttributeType::String),
        ],
    ),
    (
        AstType::Program,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Parameters, AttributeType::AstArray),
        ],
    ),
    (
        AstType::External,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Atom, AttributeType::Ast),
            (Attribute::Body, AttributeType::AstArray),
            (Attribute::ExternalType, AttributeType::Ast),
        ],
    ),
    (
        AstType::Edge,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::NodeU, AttributeType::Ast),
            (Attribute::NodeV, AttributeType::Ast),
            (Attribute::Body, AttributeType::AstArray),
        ],
    ),
    (
        AstType::Heuristic,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Atom, AttributeType::Ast),
            (Attribute::Body, AttributeType::AstArray),
            (Attribute::Bias, AttributeType::Ast),
            (Attribute::Priority, AttributeType::Ast),
            (Attribute::Modifier, AttributeType::Ast),
        ],
    ),
    (
        AstType::ProjectAtom,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Atom, AttributeType::Ast),
            (Attribute::Body, AttributeType::AstArray),
        ],
    ),
    (
        AstType::ProjectSignature,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Arity, AttributeType::Number),
            (Attribute::Positive, AttributeType::Number),
        ],
    ),
    (
        AstType::Defined,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Arity, AttributeType::Number),
            (Attribute::Positive, AttributeType::Number),
        ],
    ),
    (
        AstType::TheoryDefinition,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Name, AttributeType::String),
            (Attribute::Terms, AttributeType::AstArray),
            (Attribute::Atoms, AttributeType::AstArray),
        ],
    ),
    (
        AstType::Comment,
        &[
            (Attribute::Location, AttributeType::Location),
            (Attribute::Value, AttributeType::String),
            (Attribute::CommentType, AttributeType::Number),
        ],
    ),
];

/// All 45 attributes, so a built node can be checked for the ones it must NOT
/// have as well as the ones it must.
const ALL_ATTRIBUTES: [Attribute; 45] = [
    Attribute::Argument,
    Attribute::Arguments,
    Attribute::Arity,
    Attribute::Atom,
    Attribute::Atoms,
    Attribute::AtomType,
    Attribute::Bias,
    Attribute::Body,
    Attribute::Code,
    Attribute::Coefficient,
    Attribute::Comparison,
    Attribute::Condition,
    Attribute::Elements,
    Attribute::External,
    Attribute::ExternalType,
    Attribute::Function,
    Attribute::Guard,
    Attribute::Guards,
    Attribute::Head,
    Attribute::IsDefault,
    Attribute::Left,
    Attribute::LeftGuard,
    Attribute::Literal,
    Attribute::Location,
    Attribute::Modifier,
    Attribute::Name,
    Attribute::NodeU,
    Attribute::NodeV,
    Attribute::OperatorName,
    Attribute::OperatorType,
    Attribute::Operators,
    Attribute::Parameters,
    Attribute::Positive,
    Attribute::Priority,
    Attribute::Right,
    Attribute::RightGuard,
    Attribute::SequenceType,
    Attribute::Sign,
    Attribute::Symbol,
    Attribute::Term,
    Attribute::Terms,
    Attribute::Value,
    Attribute::Variable,
    Attribute::Weight,
    Attribute::CommentType,
];

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn span() -> Span {
    Span::new("<t>", 1, 1, "<t>", 1, 2).unwrap()
}

fn sym(name: &str) -> Symbol {
    Symbol::function(name, &[]).unwrap()
}

/// The building blocks the oracle script used, built through the
/// constructors themselves.
struct P {
    s: Span,
    x: Ast,
    a: Ast,
    one: Ast,
    fa: Ast,
    satom: Ast,
    satom_a: Ast,
    lit: Ast,
    nlit: Ast,
    glt: Ast,
    guard: Ast,
    cl: Ast,
    bae: Ast,
    hae: Ast,
    tf: Ast,
    tue: Ast,
    tut: Ast,
    tg: Ast,
    tae: Ast,
    od: Ast,
    td: Ast,
    gd: Ast,
    ad: Ast,
    truth: Ast,
}

impl P {
    fn new() -> P {
        let s = span();
        let x = ast::variable(&s, "X").unwrap();
        let a = ast::symbolic_term(&s, sym("a")).unwrap();
        let one = ast::symbolic_term(&s, Symbol::number(1)).unwrap();
        let fa = ast::function(&s, "f", &[a.clone(), x.clone()], false).unwrap();
        let satom = ast::symbolic_atom(&fa).unwrap();
        let satom_a = ast::symbolic_atom(&a).unwrap();
        let lit = ast::literal(&s, LiteralSign::NoSign, &satom).unwrap();
        let nlit = ast::literal(&s, LiteralSign::Negation, &satom_a).unwrap();
        let glt = ast::guard(ComparisonOperator::LessThan, &one).unwrap();
        let guard = ast::guard(ComparisonOperator::LessEqual, &one).unwrap();
        let cl = ast::conditional_literal(&s, &lit, &[nlit.clone()]).unwrap();
        let bae = ast::body_aggregate_element(&[x.clone()], &[lit.clone()]).unwrap();
        let hae = ast::head_aggregate_element(&[x.clone()], &cl).unwrap();
        let tf = ast::theory_function(&s, "g", &[a.clone()]).unwrap();
        let tue = ast::theory_unparsed_term_element(&["+"], &tf).unwrap();
        let tut = ast::theory_unparsed_term(&s, &[tue.clone()]).unwrap();
        let tg = ast::theory_guard("<=", &tut).unwrap();
        let tae = ast::theory_atom_element(&[tut.clone()], &[lit.clone()]).unwrap();
        let od =
            ast::theory_operator_definition(&s, "+", 5, TheoryOperatorType::BinaryLeft).unwrap();
        let td = ast::theory_term_definition(&s, "t", &[od.clone()]).unwrap();
        let gd = ast::theory_guard_definition(&["<="], "t").unwrap();
        let ad =
            ast::theory_atom_definition(&s, TheoryAtomType::Any, "p", 1, "t", Some(&gd)).unwrap();
        let truth = ast::symbolic_term(&s, sym("true")).unwrap();
        P {
            s,
            x,
            a,
            one,
            fa,
            satom,
            satom_a,
            lit,
            nlit,
            glt,
            guard,
            cl,
            bae,
            hae,
            tf,
            tue,
            tut,
            tg,
            tae,
            od,
            td,
            gd,
            ad,
            truth,
        }
    }
}

/// Checks the node kind, the oracle's `Display`, a `Location` equal to the
/// one passed, and the exact attribute set against `EXPECTED`.
fn check(built: Result<Ast, Error>, kind: AstType, text: &str) {
    let node = built.unwrap();
    assert_eq!(node.ast_type(), kind);
    assert_eq!(node.to_string(), text);
    let (_, expected) = EXPECTED
        .iter()
        .find(|(t, _)| *t == kind)
        .expect("every kind is in the table");
    for attribute in ALL_ATTRIBUTES {
        let want = expected
            .iter()
            .find(|(a, _)| *a == attribute)
            .map(|(_, t)| *t);
        assert_eq!(
            node.has_attribute(attribute),
            want.is_some(),
            "{kind:?}.{attribute:?} presence"
        );
        assert_eq!(
            node.attribute_type(attribute),
            want,
            "{kind:?}.{attribute:?} kind"
        );
    }
    if node.has_attribute(Attribute::Location) {
        assert_eq!(node.span(Attribute::Location).unwrap(), span());
    }
}

// ---------------------------------------------------------------------------
// One test per node kind (46)
// ---------------------------------------------------------------------------

#[test]
fn builds_id() {
    let p = P::new();
    check(ast::id(&p.s, "x"), AstType::Id, "x");
}

#[test]
fn builds_variable() {
    let p = P::new();
    check(ast::variable(&p.s, "X"), AstType::Variable, "X");
}

#[test]
fn builds_symbolic_term() {
    let p = P::new();
    check(
        ast::symbolic_term(&p.s, sym("a")),
        AstType::SymbolicTerm,
        "a",
    );
}

#[test]
fn builds_unary_operation() {
    let p = P::new();
    check(
        ast::unary_operation(&p.s, UnaryOperator::Minus, &p.x),
        AstType::UnaryOperation,
        "-X",
    );
}

#[test]
fn builds_binary_operation() {
    let p = P::new();
    check(
        ast::binary_operation(&p.s, BinaryOperator::Plus, &p.x, &p.one),
        AstType::BinaryOperation,
        "(X+1)",
    );
}

#[test]
fn builds_interval() {
    let p = P::new();
    check(
        ast::interval(&p.s, &p.one, &p.x),
        AstType::Interval,
        "(1..X)",
    );
}

#[test]
fn builds_function() {
    let p = P::new();
    check(
        ast::function(&p.s, "f", &[p.a.clone(), p.x.clone()], false),
        AstType::Function,
        "f(a,X)",
    );
}

#[test]
fn builds_pool() {
    let p = P::new();
    check(
        ast::pool(&p.s, &[p.a.clone(), p.one.clone()]),
        AstType::Pool,
        "(a;1)",
    );
}

#[test]
fn builds_boolean_constant() {
    check(
        ast::boolean_constant(true),
        AstType::BooleanConstant,
        "#true",
    );
}

#[test]
fn builds_symbolic_atom() {
    let p = P::new();
    check(ast::symbolic_atom(&p.fa), AstType::SymbolicAtom, "f(a,X)");
}

#[test]
fn builds_comparison() {
    let p = P::new();
    check(
        ast::comparison(&p.x, &[p.glt.clone()]),
        AstType::Comparison,
        "X < 1",
    );
}

#[test]
fn builds_guard() {
    let p = P::new();
    check(
        ast::guard(ComparisonOperator::LessEqual, &p.one),
        AstType::Guard,
        " <= 1",
    );
}

#[test]
fn builds_conditional_literal() {
    let p = P::new();
    check(
        ast::conditional_literal(&p.s, &p.lit, &[p.nlit.clone()]),
        AstType::ConditionalLiteral,
        "f(a,X): not a",
    );
}

#[test]
fn builds_aggregate() {
    let p = P::new();
    check(
        ast::aggregate(&p.s, Some(&p.guard), &[p.cl.clone()], None),
        AstType::Aggregate,
        "1 <= { f(a,X): not a }",
    );
}

#[test]
fn builds_body_aggregate_element() {
    let p = P::new();
    check(
        ast::body_aggregate_element(&[p.x.clone()], &[p.lit.clone()]),
        AstType::BodyAggregateElement,
        "X: f(a,X)",
    );
}

#[test]
fn builds_body_aggregate() {
    let p = P::new();
    check(
        ast::body_aggregate(
            &p.s,
            None,
            AggregateFunction::Sum,
            &[p.bae.clone()],
            Some(&p.guard),
        ),
        AstType::BodyAggregate,
        "#sum { X: f(a,X) } <= 1",
    );
}

#[test]
fn builds_head_aggregate_element() {
    let p = P::new();
    check(
        ast::head_aggregate_element(&[p.x.clone()], &p.cl),
        AstType::HeadAggregateElement,
        "X: f(a,X): not a",
    );
}

#[test]
fn builds_head_aggregate() {
    let p = P::new();
    check(
        ast::head_aggregate(
            &p.s,
            Some(&p.guard),
            AggregateFunction::Count,
            &[p.hae.clone()],
            None,
        ),
        AstType::HeadAggregate,
        "1 <= #count { X: f(a,X): not a }",
    );
}

#[test]
fn builds_disjunction() {
    let p = P::new();
    check(
        ast::disjunction(&p.s, &[p.cl.clone()]),
        AstType::Disjunction,
        "f(a,X): not a",
    );
}

#[test]
fn builds_theory_sequence() {
    let p = P::new();
    check(
        ast::theory_sequence(&p.s, TheorySequenceType::List, &[p.tf.clone()]),
        AstType::TheorySequence,
        "[g(a)]",
    );
}

#[test]
fn builds_theory_function() {
    let p = P::new();
    check(
        ast::theory_function(&p.s, "g", &[p.a.clone()]),
        AstType::TheoryFunction,
        "g(a)",
    );
}

#[test]
fn builds_theory_unparsed_term_element() {
    let p = P::new();
    check(
        ast::theory_unparsed_term_element(&["+"], &p.tf),
        AstType::TheoryUnparsedTermElement,
        "+ g(a)",
    );
}

#[test]
fn builds_theory_unparsed_term() {
    let p = P::new();
    check(
        ast::theory_unparsed_term(&p.s, &[p.tue.clone()]),
        AstType::TheoryUnparsedTerm,
        "(+ g(a))",
    );
}

#[test]
fn builds_theory_guard() {
    let p = P::new();
    check(
        ast::theory_guard("<=", &p.tut),
        AstType::TheoryGuard,
        "<= (+ g(a))",
    );
}

#[test]
fn builds_theory_atom_element() {
    let p = P::new();
    check(
        ast::theory_atom_element(&[p.tut.clone()], &[p.lit.clone()]),
        AstType::TheoryAtomElement,
        "(+ g(a)): f(a,X)",
    );
}

#[test]
fn builds_theory_atom() {
    let p = P::new();
    check(
        ast::theory_atom(&p.s, &p.fa, &[p.tae.clone()], Some(&p.tg)),
        AstType::TheoryAtom,
        "&f(a,X) { (+ g(a)): f(a,X) } <= (+ g(a))",
    );
}

#[test]
fn builds_literal() {
    let p = P::new();
    check(
        ast::literal(&p.s, LiteralSign::Negation, &p.satom_a),
        AstType::Literal,
        "not a",
    );
}

#[test]
fn builds_theory_operator_definition() {
    let p = P::new();
    check(
        ast::theory_operator_definition(&p.s, "+", 5, TheoryOperatorType::BinaryLeft),
        AstType::TheoryOperatorDefinition,
        "+ : 5, binary, left",
    );
}

#[test]
fn builds_theory_term_definition() {
    let p = P::new();
    check(
        ast::theory_term_definition(&p.s, "t", &[p.od.clone()]),
        AstType::TheoryTermDefinition,
        "t {\n  + : 5, binary, left\n}",
    );
}

#[test]
fn builds_theory_guard_definition() {
    check(
        ast::theory_guard_definition(&["<="], "t"),
        AstType::TheoryGuardDefinition,
        "{ <= }, t",
    );
}

#[test]
fn builds_theory_atom_definition() {
    let p = P::new();
    check(
        ast::theory_atom_definition(&p.s, TheoryAtomType::Any, "p", 1, "t", Some(&p.gd)),
        AstType::TheoryAtomDefinition,
        "&p/1: t, { <= }, t, any",
    );
}

#[test]
fn builds_rule() {
    let p = P::new();
    check(
        ast::rule(&p.s, &p.lit, &[p.nlit.clone()]),
        AstType::Rule,
        "f(a,X) :- not a.",
    );
}

#[test]
fn builds_definition() {
    let p = P::new();
    check(
        ast::definition(&p.s, "k", &p.one, false),
        AstType::Definition,
        "#const k = 1. [override]",
    );
}

#[test]
fn builds_show_signature() {
    let p = P::new();
    check(
        ast::show_signature(&p.s, "p", 2, true),
        AstType::ShowSignature,
        "#show p/2.",
    );
}

#[test]
fn builds_show_term() {
    let p = P::new();
    check(
        ast::show_term(&p.s, &p.x, &[p.lit.clone()]),
        AstType::ShowTerm,
        "#show X : f(a,X).",
    );
}

#[test]
fn builds_minimize() {
    let p = P::new();
    // Weight and priority differ, so swapping them in the constructor shows.
    let weight = ast::symbolic_term(&p.s, Symbol::number(3)).unwrap();
    let priority = ast::symbolic_term(&p.s, Symbol::number(2)).unwrap();
    check(
        ast::minimize(&p.s, &weight, &priority, &[p.x.clone()], &[p.lit.clone()]),
        AstType::Minimize,
        ":~ f(a,X). [3@2,X]",
    );
}

#[test]
fn builds_script() {
    let p = P::new();
    check(
        ast::script(&p.s, "python", "#x"),
        AstType::Script,
        "#script (python)#x#end.",
    );
}

#[test]
fn builds_program() {
    let p = P::new();
    check(
        ast::program(&p.s, "base", &[]),
        AstType::Program,
        "#program base.",
    );
}

#[test]
fn builds_external() {
    let p = P::new();
    check(
        ast::external(&p.s, &p.satom, &[p.nlit.clone()], &p.truth),
        AstType::External,
        "#external f(a,X) : not a. [true]",
    );
}

#[test]
fn builds_edge() {
    let p = P::new();
    check(
        ast::edge(&p.s, &p.x, &p.one, &[p.lit.clone()]),
        AstType::Edge,
        "#edge (X,1) : f(a,X).",
    );
}

#[test]
fn builds_heuristic() {
    let p = P::new();
    // Bias and priority differ, so swapping them in the constructor shows.
    let bias = ast::symbolic_term(&p.s, Symbol::number(3)).unwrap();
    let priority = ast::symbolic_term(&p.s, Symbol::number(2)).unwrap();
    check(
        ast::heuristic(&p.s, &p.satom, &[p.nlit.clone()], &bias, &priority, &p.a),
        AstType::Heuristic,
        "#heuristic f(a,X) : not a. [3@2,a]",
    );
}

#[test]
fn builds_project_atom() {
    let p = P::new();
    check(
        ast::project_atom(&p.s, &p.satom, &[p.lit.clone()]),
        AstType::ProjectAtom,
        "#project f(a,X) : f(a,X).",
    );
}

#[test]
fn builds_project_signature() {
    let p = P::new();
    check(
        ast::project_signature(&p.s, "p", 1, false),
        AstType::ProjectSignature,
        "#project -p/1.",
    );
}

#[test]
fn builds_defined() {
    let p = P::new();
    check(
        ast::defined(&p.s, "p", 1, true),
        AstType::Defined,
        "#defined p/1.",
    );
}

#[test]
fn builds_theory_definition() {
    let p = P::new();
    check(
        ast::theory_definition(&p.s, "th", &[p.td.clone()], &[p.ad.clone()]),
        AstType::TheoryDefinition,
        "#theory th {\n  t {\n    + : 5, binary, left\n  };\n  &p/1: t, { <= }, t, any\n}.",
    );
}

#[test]
fn builds_comment() {
    let p = P::new();
    check(
        ast::comment(&p.s, "% hi", CommentType::Line),
        AstType::Comment,
        "% hi",
    );
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

#[test]
fn optional_asts_take_none_and_some() {
    let p = P::new();
    let both_none = ast::aggregate(&p.s, None, &[p.cl.clone()], None).unwrap();
    assert_eq!(both_none.to_string(), "{ f(a,X): not a }");
    assert!(
        both_none
            .optional_ast(Attribute::LeftGuard)
            .unwrap()
            .is_none()
    );
    assert!(
        both_none
            .optional_ast(Attribute::RightGuard)
            .unwrap()
            .is_none()
    );

    let some = ast::aggregate(&p.s, Some(&p.guard), &[p.cl.clone()], None).unwrap();
    assert_eq!(some.to_string(), "1 <= { f(a,X): not a }");
    let stored = some.optional_ast(Attribute::LeftGuard).unwrap().unwrap();
    assert!(stored.ptr_eq(&p.guard));
    assert!(some.optional_ast(Attribute::RightGuard).unwrap().is_none());
}

#[test]
fn empty_arrays_are_valid() {
    let p = P::new();
    let rule = ast::rule(&p.s, &p.lit, &[]).unwrap();
    assert_eq!(rule.to_string(), "f(a,X).");
    assert_eq!(rule.ast_array_len(Attribute::Body).unwrap(), 0);

    let no_strings: [&str; 0] = [];
    let element = ast::theory_unparsed_term_element(&no_strings, &p.tf).unwrap();
    assert_eq!(element.to_string(), "g(a)");
    assert_eq!(element.string_array_len(Attribute::Operators).unwrap(), 0);
}

#[test]
fn string_arrays_accept_str_and_string_slices() {
    let p = P::new();
    let owned = vec![String::from("+"), String::from("-")];
    let from_strings = ast::theory_unparsed_term_element(&owned, &p.tf).unwrap();
    let from_strs = ast::theory_unparsed_term_element(&["+", "-"], &p.tf).unwrap();
    assert_eq!(from_strings.to_string(), "+ - g(a)");
    assert_eq!(from_strings, from_strs);
    assert_eq!(
        from_strings.string_array_len(Attribute::Operators).unwrap(),
        2
    );
    assert_eq!(
        from_strings.string_at(Attribute::Operators, 0).unwrap(),
        "+"
    );
    assert_eq!(
        from_strings.string_at(Attribute::Operators, 1).unwrap(),
        "-"
    );
}

#[test]
fn ast_array_elements_keep_their_order() {
    let p = P::new();
    let rule = ast::rule(
        &p.s,
        &p.lit,
        &[p.nlit.clone(), p.lit.clone(), p.nlit.clone()],
    )
    .unwrap();
    assert_eq!(rule.ast_array_len(Attribute::Body).unwrap(), 3);
    assert_eq!(rule.to_string(), "f(a,X) :- not a; f(a,X); not a.");
    assert!(rule.ast_at(Attribute::Body, 0).unwrap().ptr_eq(&p.nlit));
    assert!(rule.ast_at(Attribute::Body, 1).unwrap().ptr_eq(&p.lit));
    assert!(rule.ast_at(Attribute::Body, 2).unwrap().ptr_eq(&p.nlit));
}

#[test]
fn arguments_are_borrowed_and_shared_not_copied() {
    let p = P::new();
    let body = [p.nlit.clone()];
    let rule = ast::rule(&p.s, &p.lit, &body).unwrap();
    // The caller's handles are untouched and are the very nodes stored.
    assert!(rule.ast(Attribute::Head).unwrap().ptr_eq(&p.lit));
    assert!(rule.ast_at(Attribute::Body, 0).unwrap().ptr_eq(&body[0]));
    assert_eq!(p.lit.to_string(), "f(a,X)");
    assert_eq!(body[0].to_string(), "not a");
    // The built node holds its own references: drop every argument handle and
    // the node still reads and prints.
    let (head, tail) = (p.lit.clone(), body);
    drop(p);
    drop(head);
    drop(tail);
    assert_eq!(rule.to_string(), "f(a,X) :- not a.");
    assert_eq!(rule.ast(Attribute::Head).unwrap().to_string(), "f(a,X)");
}

#[test]
fn dropping_the_built_node_leaves_its_arguments_alone() {
    let p = P::new();
    let rule = ast::rule(&p.s, &p.lit, &[p.nlit.clone()]).unwrap();
    drop(rule);
    assert_eq!(p.lit.to_string(), "f(a,X)");
    assert_eq!(p.nlit.to_string(), "not a");
}

#[test]
fn a_constructor_always_returns_a_new_top_node() {
    let p = P::new();
    let first = ast::rule(&p.s, &p.lit, &[]).unwrap();
    let second = ast::rule(&p.s, &p.lit, &[]).unwrap();
    assert!(!first.ptr_eq(&second));
    assert_eq!(first, second);
    assert!(!first.ptr_eq(&p.lit));
}

#[test]
fn a_built_rule_equals_the_parsed_one() {
    // Oracle: `parse_string("a :- b, not c.")[1] == Rule(...)` and equal
    // hashes (the atoms are `Function` terms, not `SymbolicTerm`s).
    let s = span();
    let function = |name: &str| ast::function(&s, name, &[], false).unwrap();
    let literal = |name: &str, sign| {
        let atom = ast::symbolic_atom(&function(name)).unwrap();
        ast::literal(&s, sign, &atom).unwrap()
    };
    let built = ast::rule(
        &s,
        &literal("a", LiteralSign::NoSign),
        &[
            literal("b", LiteralSign::NoSign),
            literal("c", LiteralSign::Negation),
        ],
    )
    .unwrap();
    let mut parsed = Vec::new();
    ast::parse_string("a :- b, not c.", |node| {
        parsed.push(node);
        Ok(())
    })
    .unwrap();
    let parsed = &parsed[1];
    assert_eq!(&built, parsed);
    assert!(!built.ptr_eq(parsed));
    assert_eq!(built.to_string(), parsed.to_string());
}

#[test]
fn a_child_of_the_wrong_kind_is_not_checked() {
    // Oracle: clingo builds, prints and compares a `Rule` used as a
    // `Literal`'s atom without complaint. clingox adds no kind check here; the
    // the program builder with such a tree.
    let p = P::new();
    let rule = ast::rule(&p.s, &p.lit, &[p.nlit.clone()]).unwrap();
    let odd = ast::literal(&p.s, LiteralSign::NoSign, &rule).unwrap();
    assert_eq!(odd.to_string(), "f(a,X) :- not a.");
}

// ---------------------------------------------------------------------------
// Scalar arguments
// ---------------------------------------------------------------------------

#[test]
fn symbols_of_every_kind_round_trip() {
    let s = span();
    for (symbol, text) in [
        (Symbol::number(7), "7"),
        (Symbol::string("s").unwrap(), "\"s\""),
        (Symbol::infimum(), "#inf"),
        (sym("a"), "a"),
    ] {
        let node = ast::symbolic_term(&s, symbol).unwrap();
        assert_eq!(node.to_string(), text);
        assert_eq!(node.symbol(Attribute::Symbol).unwrap(), symbol);
    }
}

#[test]
fn a_span_round_trips_including_the_largest_columns() {
    let across = Span::new("first.lp", 3, 4, "second.lp", 5, 6).unwrap();
    let node = ast::id(&across, "x").unwrap();
    assert_eq!(node.span(Attribute::Location).unwrap(), across);

    let big = usize::try_from(u32::MAX).unwrap();
    let largest = Span::new("big.lp", big, big, "big.lp", big, big).unwrap();
    let node = ast::id(&largest, "x").unwrap();
    let back = node.span(Attribute::Location).unwrap();
    assert_eq!(back, largest);
    assert_eq!(back.begin_line(), big);
}

#[test]
fn booleans_are_stored_as_zero_or_one() {
    let s = span();
    for (value, stored, text) in [(true, 1, "@f"), (false, 0, "f")] {
        let node = ast::function(&s, "f", &[], value).unwrap();
        assert_eq!(node.number(Attribute::External).unwrap(), stored);
        assert_eq!(node.to_string(), text);
    }
    assert_eq!(ast::boolean_constant(true).unwrap().to_string(), "#true");
    let off = ast::boolean_constant(false).unwrap();
    assert_eq!(off.to_string(), "#false");
    assert_eq!(off.number(Attribute::Value).unwrap(), 0);
    let default = ast::definition(
        &s,
        "k",
        &ast::symbolic_term(&s, Symbol::number(1)).unwrap(),
        true,
    )
    .unwrap();
    assert_eq!(default.number(Attribute::IsDefault).unwrap(), 1);
    assert_eq!(default.to_string(), "#const k = 1.");
    for kind in ["show", "project", "defined"] {
        let node = match kind {
            "show" => ast::show_signature(&s, "s", 2, false),
            "project" => ast::project_signature(&s, "s", 2, false),
            _ => ast::defined(&s, "s", 2, false),
        }
        .unwrap();
        assert_eq!(node.number(Attribute::Positive).unwrap(), 0, "{kind}");
    }
}

#[test]
fn plain_numbers_keep_their_sign() {
    // Oracle: clingo prints a negative sign or priority as given, and clingox
    // does not narrow those ranges. (Arity is narrowed, see
    // `a_negative_arity_is_invalid_input`.)
    let s = span();
    let negative_sign = ast::show_signature(&s, "s", 2, false).unwrap();
    assert_eq!(negative_sign.to_string(), "#show -s/2.");
    let priority = ast::theory_operator_definition(&s, "+", -3, TheoryOperatorType::Unary).unwrap();
    assert_eq!(priority.number(Attribute::Priority).unwrap(), -3);
}

/// the four arity attributes take a non-negative
/// domain. clingo reads `-1` as 4294967295, and `#project` builds one
/// variable per argument (OOM-killed at 4 GB), so a negative arity is refused
/// in every constructor. `0` stays accepted.
#[test]
fn a_negative_arity_is_invalid_input() {
    let s = span();
    for arity in [-1, -2, i32::MIN] {
        let results = [
            ("show_signature", ast::show_signature(&s, "p", arity, true)),
            (
                "project_signature",
                ast::project_signature(&s, "p", arity, true),
            ),
            ("defined", ast::defined(&s, "p", arity, true)),
            (
                "theory_atom_definition",
                ast::theory_atom_definition(&s, TheoryAtomType::Any, "p", arity, "t", None),
            ),
        ];
        for (label, result) in results {
            let err = result.unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{label} {arity}");
        }
    }
}

#[test]
fn an_arity_of_zero_is_accepted() {
    let s = span();
    let show = ast::show_signature(&s, "p", 0, true).unwrap();
    assert_eq!(show.number(Attribute::Arity).unwrap(), 0);
    assert_eq!(show.to_string(), "#show p/0.");
    let project = ast::project_signature(&s, "p", 0, true).unwrap();
    assert_eq!(project.number(Attribute::Arity).unwrap(), 0);
    assert_eq!(project.to_string(), "#project p/0.");
    let defined = ast::defined(&s, "p", 0, true).unwrap();
    assert_eq!(defined.number(Attribute::Arity).unwrap(), 0);
    let atom = ast::theory_atom_definition(&s, TheoryAtomType::Any, "p", 0, "t", None).unwrap();
    assert_eq!(atom.number(Attribute::Arity).unwrap(), 0);
}

// ---------------------------------------------------------------------------
// The nine enum types
// ---------------------------------------------------------------------------

/// For one enum: every variant has the header's value, converts back with
/// `TryFrom`, and the values just outside the range are `InvalidInput`.
macro_rules! enum_values {
    ($name:ident, $ty:ident, [$(($variant:ident, $value:expr)),+ $(,)?]) => {
        #[test]
        fn $name() {
            let all = [$(($ty::$variant, $value)),+];
            for (variant, value) in all {
                assert_eq!(i32::from(variant), value, "{variant:?}");
                assert_eq!($ty::try_from(value).unwrap(), variant);
            }
            let max = i32::try_from(all.len()).unwrap();
            for outside in [-1, max, i32::MAX, i32::MIN] {
                let err = $ty::try_from(outside).unwrap_err();
                assert_eq!(err.kind(), ErrorKind::InvalidInput, "{outside}");
            }
        }
    };
}

enum_values!(
    literal_sign_values,
    LiteralSign,
    [(NoSign, 0), (Negation, 1), (DoubleNegation, 2)]
);
enum_values!(
    comparison_operator_values,
    ComparisonOperator,
    [
        (GreaterThan, 0),
        (LessThan, 1),
        (LessEqual, 2),
        (GreaterEqual, 3),
        (NotEqual, 4),
        (Equal, 5)
    ]
);
enum_values!(
    unary_operator_values,
    UnaryOperator,
    [(Minus, 0), (Negation, 1), (Absolute, 2)]
);
enum_values!(
    binary_operator_values,
    BinaryOperator,
    [
        (Xor, 0),
        (Or, 1),
        (And, 2),
        (Plus, 3),
        (Minus, 4),
        (Multiplication, 5),
        (Division, 6),
        (Modulo, 7),
        (Power, 8)
    ]
);
enum_values!(
    aggregate_function_values,
    AggregateFunction,
    [(Count, 0), (Sum, 1), (SumPlus, 2), (Min, 3), (Max, 4)]
);
enum_values!(
    theory_sequence_type_values,
    TheorySequenceType,
    [(Tuple, 0), (Set, 1), (List, 2)]
);
enum_values!(
    theory_operator_type_values,
    TheoryOperatorType,
    [(Unary, 0), (BinaryLeft, 1), (BinaryRight, 2)]
);
enum_values!(
    theory_atom_type_values,
    TheoryAtomType,
    [(Head, 0), (Body, 1), (Any, 2), (Directive, 3)]
);
enum_values!(comment_type_values, CommentType, [(Line, 0), (Block, 1)]);

/// Every variant of every enum, through its constructor: the stored number is
/// the header's value, and `Display` is the oracle's (N-oracle script `o1`).
#[test]
fn every_enum_variant_is_stored_and_printed_as_the_oracle_does() {
    let p = P::new();
    let s = &p.s;
    let a = &p.a;
    let b = ast::symbolic_term(s, sym("b")).unwrap();

    for (sign, text) in [
        (LiteralSign::NoSign, "a"),
        (LiteralSign::Negation, "not a"),
        (LiteralSign::DoubleNegation, "not not a"),
    ] {
        let node = ast::literal(s, sign, &p.satom_a).unwrap();
        assert_eq!(node.to_string(), text);
        assert_eq!(node.number(Attribute::Sign).unwrap(), i32::from(sign));
    }
    for (op, text) in [
        (ComparisonOperator::Equal, " = a"),
        (ComparisonOperator::GreaterEqual, " >= a"),
        (ComparisonOperator::GreaterThan, " > a"),
        (ComparisonOperator::LessEqual, " <= a"),
        (ComparisonOperator::LessThan, " < a"),
        (ComparisonOperator::NotEqual, " != a"),
    ] {
        let node = ast::guard(op, a).unwrap();
        assert_eq!(node.to_string(), text);
        assert_eq!(node.number(Attribute::Comparison).unwrap(), i32::from(op));
    }
    for (op, text) in [
        (UnaryOperator::Absolute, "|a|"),
        (UnaryOperator::Minus, "-a"),
        (UnaryOperator::Negation, "~a"),
    ] {
        let node = ast::unary_operation(s, op, a).unwrap();
        assert_eq!(node.to_string(), text);
        assert_eq!(node.number(Attribute::OperatorType).unwrap(), i32::from(op));
    }
    for (op, text) in [
        (BinaryOperator::And, "(a&b)"),
        (BinaryOperator::Division, "(a/b)"),
        (BinaryOperator::Minus, "(a-b)"),
        (BinaryOperator::Modulo, "(a\\b)"),
        (BinaryOperator::Multiplication, "(a*b)"),
        (BinaryOperator::Or, "(a?b)"),
        (BinaryOperator::Plus, "(a+b)"),
        (BinaryOperator::Power, "(a**b)"),
        (BinaryOperator::Xor, "(a^b)"),
    ] {
        let node = ast::binary_operation(s, op, a, &b).unwrap();
        assert_eq!(node.to_string(), text);
        assert_eq!(node.number(Attribute::OperatorType).unwrap(), i32::from(op));
    }
    for (function, text) in [
        (AggregateFunction::Count, "#count { }"),
        (AggregateFunction::Max, "#max { }"),
        (AggregateFunction::Min, "#min { }"),
        (AggregateFunction::Sum, "#sum { }"),
        (AggregateFunction::SumPlus, "#sum+ { }"),
    ] {
        let body = ast::body_aggregate(s, None, function, &[], None).unwrap();
        assert_eq!(body.to_string(), text);
        assert_eq!(
            body.number(Attribute::Function).unwrap(),
            i32::from(function)
        );
        let head = ast::head_aggregate(s, None, function, &[], None).unwrap();
        assert_eq!(head.to_string(), text);
        assert_eq!(
            head.number(Attribute::Function).unwrap(),
            i32::from(function)
        );
    }
    for (kind, text) in [
        (TheorySequenceType::List, "[]"),
        (TheorySequenceType::Set, "{}"),
        (TheorySequenceType::Tuple, "()"),
    ] {
        let node = ast::theory_sequence(s, kind, &[]).unwrap();
        assert_eq!(node.to_string(), text);
        assert_eq!(
            node.number(Attribute::SequenceType).unwrap(),
            i32::from(kind)
        );
    }
    for (kind, text) in [
        (TheoryOperatorType::BinaryLeft, "+ : 1, binary, left"),
        (TheoryOperatorType::BinaryRight, "+ : 1, binary, right"),
        (TheoryOperatorType::Unary, "+ : 1, unary"),
    ] {
        let node = ast::theory_operator_definition(s, "+", 1, kind).unwrap();
        assert_eq!(node.to_string(), text);
        assert_eq!(
            node.number(Attribute::OperatorType).unwrap(),
            i32::from(kind)
        );
    }
    for (kind, text) in [
        (TheoryAtomType::Any, "&p/0: t, any"),
        (TheoryAtomType::Body, "&p/0: t, body"),
        (TheoryAtomType::Directive, "&p/0: t, directive"),
        (TheoryAtomType::Head, "&p/0: t, head"),
    ] {
        let node = ast::theory_atom_definition(s, kind, "p", 0, "t", None).unwrap();
        assert_eq!(node.to_string(), text);
        assert_eq!(node.number(Attribute::AtomType).unwrap(), i32::from(kind));
    }
    for kind in [CommentType::Line, CommentType::Block] {
        let node = ast::comment(s, "x", kind).unwrap();
        assert_eq!(node.to_string(), "x");
        assert_eq!(
            node.number(Attribute::CommentType).unwrap(),
            i32::from(kind)
        );
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[test]
fn a_nul_byte_in_a_string_is_a_nul_error() {
    let p = P::new();
    let s = &p.s;
    assert_eq!(ast::id(s, "a\0b").unwrap_err().kind(), ErrorKind::Nul);
    assert_eq!(ast::variable(s, "\0").unwrap_err().kind(), ErrorKind::Nul);
    assert_eq!(
        ast::script(s, "python", "x\0").unwrap_err().kind(),
        ErrorKind::Nul
    );
    assert_eq!(
        ast::comment(s, "%\0", CommentType::Line)
            .unwrap_err()
            .kind(),
        ErrorKind::Nul
    );
    assert_eq!(
        ast::theory_guard_definition(&["<="], "t\0")
            .unwrap_err()
            .kind(),
        ErrorKind::Nul
    );
}

#[test]
fn a_nul_byte_in_a_string_array_element_is_a_nul_error() {
    let p = P::new();
    let err = ast::theory_unparsed_term_element(&["+", "-\0"], &p.tf).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    let err = ast::theory_guard_definition(&["a\0b"], "t").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
}

#[test]
fn text_that_is_not_ascii_round_trips() {
    let s = span();
    let node = ast::id(&s, "größe").unwrap();
    assert_eq!(node.string(Attribute::Name).unwrap(), "größe");
    assert_eq!(node.to_string(), "größe");
}

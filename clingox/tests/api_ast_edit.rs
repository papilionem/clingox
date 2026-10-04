//! The attribute setters and the array editors of `Ast`: success
//! paths read back through the matching getter, wrong-kind and absent
//! attributes, range checks on indices and on enum and boolean numbers, NUL
//! bytes, shared clones, borrowed values, and the cycle guard.
//!
//! Oracle: pyclingo 5.8.2 (`clingo.ast`: attribute assignment, `ASTSequence`
//! editing, `copy.copy`/`copy.deepcopy`, and the raw
//! `clingo_ast_attribute_*_string_at` calls, because pyclingo's own
//! `StrSequence.__setitem__` calls a function that does not exist). The
//! expected strings come from that session. The six array editors are not
//! bounds-checked by clingo (the oracle killed its process for an index
//! past the end), so the out-of-range tests here are the ones that keep
//! clingox from that undefined behaviour. None runs under Miri.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::cloned_ref_to_slice_refs,
    reason = "the arguments read as the constructor's own list; a clone is a reference count"
)]
#![allow(
    clippy::too_many_lines,
    reason = "each test walks one editor through its whole boundary"
)]

use std::hash::{DefaultHasher, Hash, Hasher};

use clingox::ErrorKind;
use clingox::ast::{
    self, AggregateFunction, Ast, Attribute, BinaryOperator, CommentType, ComparisonOperator,
    LiteralSign, Span, TheoryAtomType, TheoryOperatorType, TheorySequenceType, UnaryOperator,
};
use clingox::{Error, Symbol};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn span() -> Span {
    Span::new("<t>", 1, 1, "<t>", 1, 2).unwrap()
}

fn func(name: &str, arguments: &[Ast]) -> Ast {
    ast::function(&span(), name, arguments, false).unwrap()
}

/// A literal over a `Function` term (what the parser produces), printing as
/// `name` or `not name`.
fn lit(name: &str, sign: LiteralSign) -> Ast {
    let atom = ast::symbolic_atom(&func(name, &[])).unwrap();
    ast::literal(&span(), sign, &atom).unwrap()
}

fn pos(name: &str) -> Ast {
    lit(name, LiteralSign::NoSign)
}

/// `head :- body...` with plain positive literals.
fn rule(head: &str, body: &[&str]) -> Ast {
    let body: Vec<Ast> = body.iter().map(|name| pos(name)).collect();
    ast::rule(&span(), &pos(head), &body).unwrap()
}

fn one() -> Ast {
    ast::symbolic_term(&span(), Symbol::number(1)).unwrap()
}

/// A theory operator list `["+", "-"]` on an element printing as `+ - g`.
fn operators() -> Ast {
    let g = ast::theory_function(&span(), "g", &[]).unwrap();
    ast::theory_unparsed_term_element(&["+", "-"], &g).unwrap()
}

fn hash_of(node: &Ast) -> u64 {
    let mut hasher = DefaultHasher::new();
    node.hash(&mut hasher);
    hasher.finish()
}

#[track_caller]
fn assert_kind<T: std::fmt::Debug>(result: Result<T, Error>, kind: ErrorKind) {
    assert_eq!(result.unwrap_err().kind(), kind);
}

// ---------------------------------------------------------------------------
// Success paths, one per setter
// ---------------------------------------------------------------------------

#[test]
fn set_number_changes_a_sign() {
    let node = pos("a");
    assert_eq!(node.number(Attribute::Sign).unwrap(), 0);
    node.set_number(Attribute::Sign, 1).unwrap();
    assert_eq!(node.number(Attribute::Sign).unwrap(), 1);
    assert_eq!(node.to_string(), "not a");
    node.set_number(Attribute::Sign, 2).unwrap();
    assert_eq!(node.to_string(), "not not a");
}

#[test]
fn set_symbol_replaces_a_symbol() {
    let node = ast::symbolic_term(&span(), Symbol::function("a", &[]).unwrap()).unwrap();
    node.set_symbol(Attribute::Symbol, Symbol::number(7))
        .unwrap();
    assert_eq!(node.symbol(Attribute::Symbol).unwrap(), Symbol::number(7));
    assert_eq!(node.to_string(), "7");
    let text = Symbol::string("s").unwrap();
    node.set_symbol(Attribute::Symbol, text).unwrap();
    assert_eq!(node.symbol(Attribute::Symbol).unwrap(), text);
    assert_eq!(node.to_string(), "\"s\"");
}

#[test]
fn set_span_replaces_a_location_and_nothing_else() {
    let node = func("f", &[]);
    let before = node.to_string();
    let moved = Span::new("f.lp", 3, 4, "g.lp", 5, 6).unwrap();
    node.set_span(Attribute::Location, &moved).unwrap();
    assert_eq!(node.span(Attribute::Location).unwrap(), moved);
    assert_eq!(node.to_string(), before);
}

#[test]
fn set_string_replaces_a_name() {
    let node = ast::variable(&span(), "X").unwrap();
    node.set_string(Attribute::Name, "Y").unwrap();
    assert_eq!(node.string(Attribute::Name).unwrap(), "Y");
    assert_eq!(node.to_string(), "Y");
    node.set_string(Attribute::Name, "größe").unwrap();
    assert_eq!(node.string(Attribute::Name).unwrap(), "größe");
    node.set_string(Attribute::Name, "").unwrap();
    assert_eq!(node.string(Attribute::Name).unwrap(), "");
}

#[test]
fn set_ast_replaces_a_child_and_borrows_the_value() {
    let node = rule("a", &["b"]);
    let value = pos("c");
    node.set_ast(Attribute::Head, &value).unwrap();
    assert_eq!(node.to_string(), "c :- b.");
    assert!(node.ast(Attribute::Head).unwrap().ptr_eq(&value));
    // The caller's handle is still valid and still the same node.
    assert_eq!(value.to_string(), "c");
    value.set_number(Attribute::Sign, 1).unwrap();
    assert_eq!(node.to_string(), "not c :- b.");
}

#[test]
fn set_optional_ast_sets_and_unsets() {
    let cond = ast::conditional_literal(&span(), &pos("a"), &[pos("b")]).unwrap();
    let node = ast::aggregate(&span(), None, &[cond], None).unwrap();
    assert_eq!(node.to_string(), "{ a: b }");
    let guard = ast::guard(ComparisonOperator::LessEqual, &one()).unwrap();
    node.set_optional_ast(Attribute::LeftGuard, Some(&guard))
        .unwrap();
    assert_eq!(node.to_string(), "1 <= { a: b }");
    let stored = node.optional_ast(Attribute::LeftGuard).unwrap().unwrap();
    assert!(stored.ptr_eq(&guard));
    node.set_optional_ast(Attribute::LeftGuard, None).unwrap();
    assert!(node.optional_ast(Attribute::LeftGuard).unwrap().is_none());
    assert_eq!(node.to_string(), "{ a: b }");
}

#[test]
fn the_string_array_editors_follow_the_oracle() {
    // Oracle/o1: start `+ -`, insert at the end, set the last, delete the
    // first, insert at the front.
    let node = operators();
    assert_eq!(node.to_string(), "+ - g");
    node.insert_string_at(Attribute::Operators, 2, "*").unwrap();
    assert_eq!(node.to_string(), "+ - * g");
    node.set_string_at(Attribute::Operators, 2, "/").unwrap();
    assert_eq!(node.to_string(), "+ - / g");
    node.delete_string_at(Attribute::Operators, 0).unwrap();
    assert_eq!(node.to_string(), "- / g");
    node.insert_string_at(Attribute::Operators, 0, "^").unwrap();
    assert_eq!(node.to_string(), "^ - / g");
    assert_eq!(node.string_array_len(Attribute::Operators).unwrap(), 3);
    assert_eq!(node.string_at(Attribute::Operators, 0).unwrap(), "^");
    node.push_string(Attribute::Operators, "%").unwrap();
    assert_eq!(node.string_at(Attribute::Operators, 3).unwrap(), "%");
    node.set_string_at(Attribute::Operators, 1, "").unwrap();
    assert_eq!(node.string_at(Attribute::Operators, 1).unwrap(), "");
}

#[test]
fn the_ast_array_editors_follow_the_oracle() {
    // Oracle o1: `a :- b; c.` then set 1, insert at the end, insert at 0,
    // delete 1.
    let node = rule("a", &["b", "c"]);
    node.set_ast_at(Attribute::Body, 1, &pos("a")).unwrap();
    assert_eq!(node.to_string(), "a :- b; a.");
    node.insert_ast_at(Attribute::Body, 2, &pos("c")).unwrap();
    assert_eq!(node.to_string(), "a :- b; a; c.");
    node.insert_ast_at(Attribute::Body, 0, &pos("c")).unwrap();
    assert_eq!(node.to_string(), "a :- c; b; a; c.");
    node.delete_ast_at(Attribute::Body, 1).unwrap();
    assert_eq!(node.to_string(), "a :- c; a; c.");
    assert_eq!(node.ast_array_len(Attribute::Body).unwrap(), 3);
}

#[test]
fn push_appends_to_either_kind_of_array() {
    let node = func("f", &[func("a", &[])]);
    node.push_ast(Attribute::Arguments, &func("b", &[]))
        .unwrap();
    assert_eq!(node.to_string(), "f(a,b)");
    let empty = ast::program(&span(), "p", &[]).unwrap();
    empty
        .push_ast(Attribute::Parameters, &ast::id(&span(), "k").unwrap())
        .unwrap();
    empty
        .push_ast(Attribute::Parameters, &ast::id(&span(), "j").unwrap())
        .unwrap();
    assert_eq!(empty.to_string(), "#program p(k, j).");
}

#[test]
fn whole_array_replacement_follows_the_oracle() {
    let node = rule("a", &["b"]);
    node.set_ast_array(Attribute::Body, &[pos("c"), pos("a")])
        .unwrap();
    assert_eq!(node.to_string(), "a :- c; a.");
    node.set_ast_array(Attribute::Body, &[]).unwrap();
    assert_eq!(node.to_string(), "a.");
    node.set_ast_array(Attribute::Body, &[pos("b"), pos("c"), pos("a")])
        .unwrap();
    assert_eq!(node.ast_array_len(Attribute::Body).unwrap(), 3);

    let ops = operators();
    ops.set_string_array(Attribute::Operators, &["a", "b", "c"])
        .unwrap();
    assert_eq!(ops.to_string(), "a b c g");
    let no_strings: [&str; 0] = [];
    ops.set_string_array(Attribute::Operators, &no_strings)
        .unwrap();
    assert_eq!(ops.to_string(), "g");
    ops.set_string_array(Attribute::Operators, &[String::from("+")])
        .unwrap();
    assert_eq!(ops.to_string(), "+ g");
}

#[test]
fn setters_work_on_parsed_nodes() {
    // Oracle: replacing `q(X)` in `p(X) :- q(X).` by the head of `r.`.
    let mut rules = Vec::new();
    ast::parse_string("p(X) :- q(X). r.", |node| {
        rules.push(node);
        Ok(())
    })
    .unwrap();
    let replacement = rules[2].ast(Attribute::Head).unwrap();
    rules[1]
        .set_ast_at(Attribute::Body, 0, &replacement)
        .unwrap();
    assert_eq!(rules[1].to_string(), "p(X) :- r.");
    assert_eq!(rules[2].to_string(), "r.");
}

#[test]
fn setting_an_attribute_to_its_current_value_is_allowed() {
    let node = rule("a", &["b"]);
    let head = node.ast(Attribute::Head).unwrap();
    node.set_ast(Attribute::Head, &head).unwrap();
    node.set_number(Attribute::Sign, 0).unwrap_err();
    head.set_number(Attribute::Sign, 0).unwrap();
    node.set_span(
        Attribute::Location,
        &node.span(Attribute::Location).unwrap(),
    )
    .unwrap();
    assert_eq!(node.to_string(), "a :- b.");
}

// ---------------------------------------------------------------------------
// Wrong kind, absent attribute
// ---------------------------------------------------------------------------

/// Every setter, on an attribute of another kind or one the node lacks, is
/// `InvalidInput` (clingo itself reports `Runtime`) and changes nothing.
#[test]
fn a_setter_on_an_attribute_of_another_kind_is_invalid_input() {
    let f = func("f", &[func("a", &[])]);
    let r = rule("a", &["b"]);
    let ag = ast::aggregate(&span(), None, &[], None).unwrap();
    let ops = operators();
    let sym = ast::symbolic_term(&span(), Symbol::number(1)).unwrap();
    let other = pos("z");
    let s = span();
    let (fb, rb, ab, ob) = (
        f.to_string(),
        r.to_string(),
        ag.to_string(),
        ops.to_string(),
    );
    let bad = ErrorKind::InvalidInput;

    // f: Name string, External number, Arguments ast_array, Location location.
    assert_kind(f.set_number(Attribute::Name, 1), bad);
    assert_kind(f.set_number(Attribute::Arguments, 1), bad);
    assert_kind(f.set_number(Attribute::Location, 1), bad);
    assert_kind(f.set_symbol(Attribute::External, Symbol::number(1)), bad);
    assert_kind(f.set_span(Attribute::Name, &s), bad);
    assert_kind(f.set_string(Attribute::External, "x"), bad);
    assert_kind(f.set_string(Attribute::Arguments, "x"), bad);
    assert_kind(f.set_ast(Attribute::Arguments, &other), bad);
    assert_kind(f.set_ast(Attribute::Name, &other), bad);
    assert_kind(f.set_optional_ast(Attribute::Name, Some(&other)), bad);
    assert_kind(f.set_string_at(Attribute::Arguments, 0, "x"), bad);
    assert_kind(f.insert_string_at(Attribute::Arguments, 0, "x"), bad);
    assert_kind(f.delete_string_at(Attribute::Arguments, 0), bad);
    assert_kind(f.push_string(Attribute::Arguments, "x"), bad);
    assert_kind(f.set_string_array(Attribute::Arguments, &["x"]), bad);
    assert_kind(f.set_ast_at(Attribute::Name, 0, &other), bad);
    assert_kind(f.insert_ast_at(Attribute::Name, 0, &other), bad);
    assert_kind(f.delete_ast_at(Attribute::Name, 0), bad);
    assert_kind(f.push_ast(Attribute::Name, &other), bad);
    assert_kind(f.set_ast_array(Attribute::Name, &[other.clone()]), bad);
    // r: Head ast, Body ast_array. `set_ast` on an optional and the reverse.
    assert_kind(r.set_ast(Attribute::Body, &other), bad);
    assert_kind(r.set_optional_ast(Attribute::Head, Some(&other)), bad);
    assert_kind(r.set_optional_ast(Attribute::Head, None), bad);
    assert_kind(ag.set_ast(Attribute::LeftGuard, &other), bad);
    assert_kind(ag.set_ast_at(Attribute::LeftGuard, 0, &other), bad);
    // string array attribute used as an ast array and the reverse.
    assert_kind(ops.set_ast_at(Attribute::Operators, 0, &other), bad);
    assert_kind(ops.push_ast(Attribute::Operators, &other), bad);
    assert_kind(ops.set_ast_array(Attribute::Operators, &[]), bad);
    assert_kind(ops.set_string_at(Attribute::Term, 0, "x"), bad);
    assert_kind(sym.set_number(Attribute::Symbol, 1), bad);
    assert_kind(sym.set_string(Attribute::Symbol, "x"), bad);

    assert_eq!(f.to_string(), fb);
    assert_eq!(r.to_string(), rb);
    assert_eq!(ag.to_string(), ab);
    assert_eq!(ops.to_string(), ob);
}

#[test]
fn a_setter_on_an_absent_attribute_is_invalid_input() {
    let r = rule("a", &["b"]);
    let other = pos("z");
    let bad = ErrorKind::InvalidInput;
    assert_kind(r.set_number(Attribute::Sign, 1), bad);
    assert_kind(r.set_symbol(Attribute::Symbol, Symbol::number(1)), bad);
    assert_kind(r.set_string(Attribute::Name, "x"), bad);
    assert_kind(r.set_ast(Attribute::Atom, &other), bad);
    assert_kind(r.set_optional_ast(Attribute::LeftGuard, None), bad);
    assert_kind(r.set_ast_at(Attribute::Arguments, 0, &other), bad);
    assert_kind(r.push_ast(Attribute::Arguments, &other), bad);
    assert_kind(r.set_string_at(Attribute::Operators, 0, "x"), bad);
    // `Coefficient` is in the attribute table but on no node at all.
    assert_kind(r.set_number(Attribute::Coefficient, 1), bad);
    assert_kind(one().set_number(Attribute::Coefficient, 1), bad);
    assert_eq!(r.to_string(), "a :- b.");
}

// ---------------------------------------------------------------------------
// Number domains
// ---------------------------------------------------------------------------

/// A node, the number attribute to set, and its largest valid value.
fn domain_cases() -> Vec<(&'static str, Ast, Attribute, i32)> {
    let s = span();
    let a = ast::symbolic_term(&s, Symbol::function("a", &[]).unwrap()).unwrap();
    let satom = ast::symbolic_atom(&func("a", &[])).unwrap();
    vec![
        (
            "literal.sign",
            ast::literal(&s, LiteralSign::NoSign, &satom).unwrap(),
            Attribute::Sign,
            2,
        ),
        (
            "guard.comparison",
            ast::guard(ComparisonOperator::Equal, &a).unwrap(),
            Attribute::Comparison,
            5,
        ),
        (
            "unary.operator_type",
            ast::unary_operation(&s, UnaryOperator::Minus, &a).unwrap(),
            Attribute::OperatorType,
            2,
        ),
        (
            "binary.operator_type",
            ast::binary_operation(&s, BinaryOperator::Plus, &a, &a).unwrap(),
            Attribute::OperatorType,
            8,
        ),
        (
            "body_aggregate.function",
            ast::body_aggregate(&s, None, AggregateFunction::Count, &[], None).unwrap(),
            Attribute::Function,
            4,
        ),
        (
            "head_aggregate.function",
            ast::head_aggregate(&s, None, AggregateFunction::Count, &[], None).unwrap(),
            Attribute::Function,
            4,
        ),
        (
            "theory_sequence.sequence_type",
            ast::theory_sequence(&s, TheorySequenceType::Tuple, &[]).unwrap(),
            Attribute::SequenceType,
            2,
        ),
        (
            "theory_operator_definition.operator_type",
            ast::theory_operator_definition(&s, "+", 1, TheoryOperatorType::Unary).unwrap(),
            Attribute::OperatorType,
            2,
        ),
        (
            "theory_atom_definition.atom_type",
            ast::theory_atom_definition(&s, TheoryAtomType::Head, "p", 0, "t", None).unwrap(),
            Attribute::AtomType,
            3,
        ),
        (
            "comment.comment_type",
            ast::comment(&s, "x", CommentType::Line).unwrap(),
            Attribute::CommentType,
            1,
        ),
        (
            "boolean_constant.value",
            ast::boolean_constant(true).unwrap(),
            Attribute::Value,
            1,
        ),
        (
            "function.external",
            ast::function(&s, "f", &[], false).unwrap(),
            Attribute::External,
            1,
        ),
        (
            "definition.is_default",
            ast::definition(&s, "k", &a, false).unwrap(),
            Attribute::IsDefault,
            1,
        ),
        (
            "show_signature.positive",
            ast::show_signature(&s, "p", 1, true).unwrap(),
            Attribute::Positive,
            1,
        ),
        (
            "project_signature.positive",
            ast::project_signature(&s, "p", 1, true).unwrap(),
            Attribute::Positive,
            1,
        ),
        (
            "defined.positive",
            ast::defined(&s, "p", 1, true).unwrap(),
            Attribute::Positive,
            1,
        ),
    ]
}

/// clingo takes any integer and misprints; clingox accepts exactly the
/// header's values for the enum-valued and `0`/`1` for boolean-valued ones.
#[test]
fn enum_and_boolean_numbers_are_range_checked() {
    for (label, node, attribute, max) in domain_cases() {
        for value in 0..=max {
            node.set_number(attribute, value).unwrap();
            assert_eq!(node.number(attribute).unwrap(), value, "{label}");
        }
        let kept = node.to_string();
        for outside in [-1, max + 1, max + 2, i32::MAX, i32::MIN] {
            let err = node.set_number(attribute, outside).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{label} {outside}");
            assert_eq!(node.number(attribute).unwrap(), max, "{label} unchanged");
            assert_eq!(node.to_string(), kept, "{label} unchanged");
        }
    }
}

#[test]
fn other_numbers_are_not_narrowed() {
    let s = span();
    let signature = ast::show_signature(&s, "s", 1, true).unwrap();
    for value in [0, 7, i32::MAX] {
        signature.set_number(Attribute::Arity, value).unwrap();
        assert_eq!(signature.number(Attribute::Arity).unwrap(), value);
    }
    let operator = ast::theory_operator_definition(&s, "+", 1, TheoryOperatorType::Unary).unwrap();
    for value in [-3, 0, i32::MAX, i32::MIN] {
        operator.set_number(Attribute::Priority, value).unwrap();
        assert_eq!(operator.number(Attribute::Priority).unwrap(), value);
    }
}

/// `set_number` refuses a negative arity on all
/// four arity-carrying nodes and leaves the node unchanged; `0` is accepted.
#[test]
fn set_number_refuses_a_negative_arity() {
    let s = span();
    let nodes = [
        (
            "show_signature",
            ast::show_signature(&s, "p", 2, true).unwrap(),
        ),
        (
            "project_signature",
            ast::project_signature(&s, "p", 2, true).unwrap(),
        ),
        ("defined", ast::defined(&s, "p", 2, true).unwrap()),
        (
            "theory_atom_definition",
            ast::theory_atom_definition(&s, TheoryAtomType::Any, "p", 2, "t", None).unwrap(),
        ),
    ];
    for (label, node) in nodes {
        for value in [-1, -2, i32::MIN] {
            let err = node.set_number(Attribute::Arity, value).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{label} {value}");
            assert_eq!(node.number(Attribute::Arity).unwrap(), 2, "{label}");
        }
        node.set_number(Attribute::Arity, 0).unwrap();
        assert_eq!(node.number(Attribute::Arity).unwrap(), 0, "{label}");
    }
}

// ---------------------------------------------------------------------------
// The six array editors: out of range, and the boundary
// ---------------------------------------------------------------------------

/// A message that names the index and the length (uniform with the getters).
#[track_caller]
fn assert_names_index_and_len(err: &Error, index: usize, len: usize) {
    let text = err.to_string();
    assert!(text.contains(&index.to_string()), "{text}");
    assert!(text.contains(&len.to_string()), "{text}");
}

#[test]
fn set_string_at_rejects_an_index_past_the_end_and_accepts_the_last() {
    let node = operators(); // len 2
    let before = node.to_string();
    for index in [2, 3, 1000, usize::MAX] {
        let err = node
            .set_string_at(Attribute::Operators, index, "x")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{index}");
        assert_eq!(node.to_string(), before);
        assert_eq!(node.string_array_len(Attribute::Operators).unwrap(), 2);
    }
    assert_names_index_and_len(
        &node
            .set_string_at(Attribute::Operators, 7, "x")
            .unwrap_err(),
        7,
        2,
    );
    node.set_string_at(Attribute::Operators, 1, "*").unwrap();
    assert_eq!(node.to_string(), "+ * g");
    node.set_string_at(Attribute::Operators, 0, "/").unwrap();
    assert_eq!(node.to_string(), "/ * g");
}

#[test]
fn delete_string_at_rejects_an_index_past_the_end_and_accepts_the_last() {
    let node = operators();
    let before = node.to_string();
    for index in [2, 3, 1000, usize::MAX] {
        let err = node
            .delete_string_at(Attribute::Operators, index)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{index}");
        assert_eq!(node.to_string(), before);
        assert_eq!(node.string_array_len(Attribute::Operators).unwrap(), 2);
    }
    assert_names_index_and_len(
        &node.delete_string_at(Attribute::Operators, 5).unwrap_err(),
        5,
        2,
    );
    node.delete_string_at(Attribute::Operators, 1).unwrap();
    assert_eq!(node.to_string(), "+ g");
    node.delete_string_at(Attribute::Operators, 0).unwrap();
    assert_eq!(node.to_string(), "g");
    // An empty array has no valid index at all.
    assert_kind(
        node.delete_string_at(Attribute::Operators, 0),
        ErrorKind::InvalidInput,
    );
}

#[test]
fn insert_string_at_accepts_the_length_but_nothing_past_it() {
    let node = operators(); // len 2
    let before = node.to_string();
    for index in [3, 4, 1000, usize::MAX] {
        let err = node
            .insert_string_at(Attribute::Operators, index, "x")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{index}");
        assert_eq!(node.to_string(), before);
        assert_eq!(node.string_array_len(Attribute::Operators).unwrap(), 2);
    }
    assert_names_index_and_len(
        &node
            .insert_string_at(Attribute::Operators, 9, "x")
            .unwrap_err(),
        9,
        2,
    );
    node.insert_string_at(Attribute::Operators, 2, "*").unwrap();
    assert_eq!(node.to_string(), "+ - * g");
    node.insert_string_at(Attribute::Operators, 0, "^").unwrap();
    assert_eq!(node.to_string(), "^ + - * g");
    // Into an empty array: 0 is the length, 1 is past it.
    let no_strings: [&str; 0] = [];
    node.set_string_array(Attribute::Operators, &no_strings)
        .unwrap();
    assert_kind(
        node.insert_string_at(Attribute::Operators, 1, "x"),
        ErrorKind::InvalidInput,
    );
    node.insert_string_at(Attribute::Operators, 0, "x").unwrap();
    assert_eq!(node.to_string(), "x g");
}

#[test]
fn set_ast_at_rejects_an_index_past_the_end_and_accepts_the_last() {
    let node = rule("a", &["b", "c"]);
    let before = node.to_string();
    let value = pos("z");
    for index in [2, 3, 1000, usize::MAX] {
        let err = node.set_ast_at(Attribute::Body, index, &value).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{index}");
        assert_eq!(node.to_string(), before);
        assert_eq!(node.ast_array_len(Attribute::Body).unwrap(), 2);
    }
    assert_names_index_and_len(
        &node.set_ast_at(Attribute::Body, 7, &value).unwrap_err(),
        7,
        2,
    );
    node.set_ast_at(Attribute::Body, 1, &value).unwrap();
    assert_eq!(node.to_string(), "a :- b; z.");
    node.set_ast_at(Attribute::Body, 0, &value).unwrap();
    assert_eq!(node.to_string(), "a :- z; z.");
    // `value` was borrowed, not consumed, and is stored twice.
    assert_eq!(value.to_string(), "z");
    assert!(node.ast_at(Attribute::Body, 0).unwrap().ptr_eq(&value));
    assert!(node.ast_at(Attribute::Body, 1).unwrap().ptr_eq(&value));
}

#[test]
fn delete_ast_at_rejects_an_index_past_the_end_and_accepts_the_last() {
    let node = rule("a", &["b", "c"]);
    let before = node.to_string();
    for index in [2, 3, 1000, usize::MAX] {
        let err = node.delete_ast_at(Attribute::Body, index).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{index}");
        assert_eq!(node.to_string(), before);
        assert_eq!(node.ast_array_len(Attribute::Body).unwrap(), 2);
    }
    assert_names_index_and_len(&node.delete_ast_at(Attribute::Body, 5).unwrap_err(), 5, 2);
    node.delete_ast_at(Attribute::Body, 1).unwrap();
    assert_eq!(node.to_string(), "a :- b.");
    node.delete_ast_at(Attribute::Body, 0).unwrap();
    assert_eq!(node.to_string(), "a.");
    assert_kind(
        node.delete_ast_at(Attribute::Body, 0),
        ErrorKind::InvalidInput,
    );
    assert_kind(
        node.set_ast_at(Attribute::Body, 0, &pos("z")),
        ErrorKind::InvalidInput,
    );
    assert_eq!(node.to_string(), "a.");
}

#[test]
fn insert_ast_at_accepts_the_length_but_nothing_past_it() {
    let node = rule("a", &["b", "c"]);
    let before = node.to_string();
    let value = pos("z");
    for index in [3, 4, 1000, usize::MAX] {
        let err = node
            .insert_ast_at(Attribute::Body, index, &value)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{index}");
        assert_eq!(node.to_string(), before);
        assert_eq!(node.ast_array_len(Attribute::Body).unwrap(), 2);
    }
    assert_names_index_and_len(
        &node.insert_ast_at(Attribute::Body, 9, &value).unwrap_err(),
        9,
        2,
    );
    node.insert_ast_at(Attribute::Body, 2, &value).unwrap();
    assert_eq!(node.to_string(), "a :- b; c; z.");
    node.insert_ast_at(Attribute::Body, 1, &value).unwrap();
    assert_eq!(node.to_string(), "a :- b; z; c; z.");
    // Into an empty array: 0 is the length, 1 is past it.
    let empty = rule("a", &[]);
    assert_kind(
        empty.insert_ast_at(Attribute::Body, 1, &value),
        ErrorKind::InvalidInput,
    );
    empty.insert_ast_at(Attribute::Body, 0, &value).unwrap();
    assert_eq!(empty.to_string(), "a :- z.");
}

// ---------------------------------------------------------------------------
// NUL bytes
// ---------------------------------------------------------------------------

#[test]
fn a_nul_byte_in_a_string_is_a_nul_error_and_changes_nothing() {
    let name = ast::variable(&span(), "X").unwrap();
    assert_kind(name.set_string(Attribute::Name, "a\0b"), ErrorKind::Nul);
    assert_eq!(name.to_string(), "X");

    let ops = operators();
    assert_kind(
        ops.set_string_at(Attribute::Operators, 0, "a\0"),
        ErrorKind::Nul,
    );
    assert_kind(
        ops.insert_string_at(Attribute::Operators, 0, "\0"),
        ErrorKind::Nul,
    );
    assert_kind(ops.push_string(Attribute::Operators, "\0"), ErrorKind::Nul);
    assert_kind(
        ops.set_string_array(Attribute::Operators, &["ok", "no\0"]),
        ErrorKind::Nul,
    );
    assert_eq!(ops.to_string(), "+ - g");
}

#[test]
fn the_kind_is_checked_before_the_value() {
    // A NUL byte for the wrong kind of attribute is the kind error.
    let f = func("f", &[]);
    assert_kind(
        f.set_string(Attribute::External, "a\0"),
        ErrorKind::InvalidInput,
    );
    assert_kind(
        f.set_string_at(Attribute::Arguments, 0, "a\0"),
        ErrorKind::InvalidInput,
    );
    // An index error for an in-range-kind, bad-index call wins over nothing:
    // the NUL byte is the value check, the index check follows it.
    let ops = operators();
    assert_kind(
        ops.set_string_at(Attribute::Operators, 99, "ok"),
        ErrorKind::InvalidInput,
    );
}

// ---------------------------------------------------------------------------
// Sharing, copies, borrowed values
// ---------------------------------------------------------------------------

#[test]
fn clones_share_one_node() {
    let node = rule("a", &["b"]);
    let alias = node.clone();
    alias.set_ast_at(Attribute::Body, 0, &pos("z")).unwrap();
    assert_eq!(node.to_string(), "a :- z.");
    assert!(alias.ptr_eq(&node));
}

#[test]
fn a_child_changed_through_its_own_handle_is_seen_by_the_parent() {
    let node = rule("a", &["b"]);
    let head = node.ast(Attribute::Head).unwrap();
    head.set_number(Attribute::Sign, 1).unwrap();
    assert_eq!(node.to_string(), "not a :- b.");
    let body = node.ast_at(Attribute::Body, 0).unwrap();
    body.set_number(Attribute::Sign, 2).unwrap();
    assert_eq!(node.to_string(), "not a :- not not b.");
}

#[test]
fn a_shallow_copy_has_its_own_arrays_but_shares_its_children() {
    // Oracle: editing the copy's array leaves the original's array
    // alone; editing a child reached through the copy is seen by the
    // original.
    let original = func("o", &[func("a", &[])]);
    let copy = original.copy().unwrap();
    copy.push_ast(Attribute::Arguments, &func("b", &[]))
        .unwrap();
    assert_eq!(original.to_string(), "o(a)");
    assert_eq!(copy.to_string(), "o(a,b)");

    let outer = func("w", &[func("in", &[])]);
    let shallow = outer.copy().unwrap();
    shallow
        .ast_at(Attribute::Arguments, 0)
        .unwrap()
        .set_string(Attribute::Name, "zz")
        .unwrap();
    assert_eq!(outer.to_string(), "w(zz)");
}

#[test]
fn a_deep_copy_shares_nothing() {
    let outer = func("w", &[func("in", &[])]);
    let deep = outer.deep_copy().unwrap();
    deep.ast_at(Attribute::Arguments, 0)
        .unwrap()
        .set_string(Attribute::Name, "yy")
        .unwrap();
    assert_eq!(outer.to_string(), "w(in)");
    assert_eq!(deep.to_string(), "w(yy)");
    deep.push_ast(Attribute::Arguments, &func("b", &[]))
        .unwrap();
    assert_eq!(outer.to_string(), "w(in)");
}

#[test]
fn values_are_borrowed_by_every_setter() {
    let node = func("f", &[]);
    let value = func("v", &[]);
    node.push_ast(Attribute::Arguments, &value).unwrap();
    node.insert_ast_at(Attribute::Arguments, 0, &value).unwrap();
    node.set_ast_at(Attribute::Arguments, 1, &value).unwrap();
    node.set_ast_array(Attribute::Arguments, &[value.clone(), value.clone()])
        .unwrap();
    for index in 0..2 {
        assert!(
            node.ast_at(Attribute::Arguments, index)
                .unwrap()
                .ptr_eq(&value)
        );
    }
    assert_eq!(node.to_string(), "f(v,v)");
    drop(node);
    assert_eq!(value.to_string(), "v");
}

#[test]
fn owned_getters_survive_a_later_set_through_a_clone() {
    let node = ast::variable(&span(), "X").unwrap();
    let alias = node.clone();
    let before = node.string(Attribute::Name).unwrap();
    alias.set_string(Attribute::Name, "Y").unwrap();
    assert_eq!(before, "X");
    assert_eq!(node.string(Attribute::Name).unwrap(), "Y");

    let ops = operators();
    let first = ops.string_at(Attribute::Operators, 0).unwrap();
    ops.clone()
        .set_string_at(Attribute::Operators, 0, "z")
        .unwrap();
    ops.clone()
        .delete_string_at(Attribute::Operators, 0)
        .unwrap();
    assert_eq!(first, "+");
}

#[test]
fn a_nodes_hash_changes_when_it_is_edited_through_a_clone() {
    // Oracle o1: the hash before differs from the hash after, and the hash
    // after equals a fresh node built with the new name. Documented on
    // `Ast`'s `Hash`: a node used as a map key must not be edited.
    let key = ast::id(&span(), "x").unwrap();
    let before = hash_of(&key);
    key.clone().set_string(Attribute::Name, "y").unwrap();
    assert_ne!(hash_of(&key), before);
    assert_eq!(hash_of(&key), hash_of(&ast::id(&span(), "y").unwrap()));
}

#[test]
fn an_element_can_be_set_from_its_own_array() {
    // Oracle o1: `a :- b; c.` with `body[0] = body[1]` is `a :- c; c.`, and
    // rebuilding the array from its own elements reversed is `a :- c; b.`.
    let node = rule("a", &["b", "c"]);
    let second = node.ast_at(Attribute::Body, 1).unwrap();
    node.set_ast_at(Attribute::Body, 0, &second).unwrap();
    assert_eq!(node.to_string(), "a :- c; c.");

    let other = rule("a", &["b", "c"]);
    let mut items: Vec<Ast> = (0..2)
        .map(|i| other.ast_at(Attribute::Body, i).unwrap())
        .collect();
    items.reverse();
    other.set_ast_array(Attribute::Body, &items).unwrap();
    assert_eq!(other.to_string(), "a :- c; b.");
}

#[test]
fn a_child_of_a_node_can_be_moved_up_into_it() {
    // Not a cycle: the value is below `self`, and `self` is not below the
    // value.
    let node = rule("a", &["b"]);
    let child = node.ast_at(Attribute::Body, 0).unwrap();
    node.set_ast(Attribute::Head, &child).unwrap();
    assert_eq!(node.to_string(), "b :- b.");
}

// ---------------------------------------------------------------------------
// Cycles
// ---------------------------------------------------------------------------

/// Oracle: clingo accepts a node as its own descendant and then
/// overflows the stack in `Display`, `Hash`, `Eq` and `deep_copy`. Every path
/// that stores a node into another rejects it with `InvalidInput`, before any
/// change; these tests never print a cyclic node.
#[test]
fn a_node_cannot_become_its_own_child_by_any_setter() {
    let bad = ErrorKind::InvalidInput;

    let unary = ast::unary_operation(&span(), UnaryOperator::Minus, &func("x", &[])).unwrap();
    assert_kind(unary.set_ast(Attribute::Argument, &unary), bad);
    assert_eq!(unary.to_string(), "-x");

    let cond = ast::conditional_literal(&span(), &pos("a"), &[]).unwrap();
    let aggregate = ast::aggregate(&span(), None, &[cond], None).unwrap();
    assert_kind(
        aggregate.set_optional_ast(Attribute::LeftGuard, Some(&aggregate)),
        bad,
    );
    assert!(
        aggregate
            .optional_ast(Attribute::LeftGuard)
            .unwrap()
            .is_none()
    );
    assert_eq!(aggregate.to_string(), "{ a }");
    // `None` is never a cycle.
    aggregate
        .set_optional_ast(Attribute::LeftGuard, None)
        .unwrap();

    let f = func("f", &[func("x", &[])]);
    assert_kind(f.set_ast_at(Attribute::Arguments, 0, &f), bad);
    assert_kind(f.insert_ast_at(Attribute::Arguments, 0, &f), bad);
    assert_kind(f.insert_ast_at(Attribute::Arguments, 1, &f), bad);
    assert_kind(f.push_ast(Attribute::Arguments, &f), bad);
    assert_kind(f.set_ast_array(Attribute::Arguments, &[f.clone()]), bad);
    assert_eq!(f.ast_array_len(Attribute::Arguments).unwrap(), 1);
    assert_eq!(f.to_string(), "f(x)");
}

#[test]
fn a_node_cannot_become_its_own_grandchild() {
    let bad = ErrorKind::InvalidInput;
    let inner = func("a", &[]);
    let outer = func("b", &[inner.clone()]);
    assert_kind(inner.push_ast(Attribute::Arguments, &outer), bad);
    assert_kind(inner.insert_ast_at(Attribute::Arguments, 0, &outer), bad);
    assert_kind(
        inner.set_ast_array(Attribute::Arguments, &[func("ok", &[]), outer.clone()]),
        bad,
    );
    assert_eq!(inner.to_string(), "a");
    assert_eq!(outer.to_string(), "b(a)");

    let rule_node = rule("h", &["b"]);
    let head = rule_node.ast(Attribute::Head).unwrap();
    // A rule below its own head atom would be a cycle too, whatever the kinds.
    let atom = head.ast(Attribute::Atom).unwrap();
    assert_kind(atom.set_ast(Attribute::Symbol, &rule_node), bad);
    assert_eq!(rule_node.to_string(), "h :- b.");
}

#[test]
fn a_cycle_through_a_shallow_copy_is_rejected() {
    // `copy` is a new top node sharing its children, so `copy` reaches
    // `inner`, and putting `copy` into `inner` closes a loop.
    let inner = func("a", &[]);
    let outer = func("b", &[inner.clone()]);
    let copy = outer.copy().unwrap();
    assert!(!copy.ptr_eq(&outer));
    assert_kind(
        inner.push_ast(Attribute::Arguments, &copy),
        ErrorKind::InvalidInput,
    );
    assert_eq!(inner.to_string(), "a");
    // The copy itself may take the original, which is not below it.
    copy.push_ast(Attribute::Arguments, &func("c", &[]))
        .unwrap();
    assert_eq!(copy.to_string(), "b(a,c)");
}

#[test]
fn a_shared_child_is_a_dag_not_a_cycle_and_is_found_from_both_parents() {
    // Oracle o1: `top(p1(s),p2(s))`, and adding `p1` to `p2` is fine.
    let shared = func("s", &[]);
    let p1 = func("p1", &[shared.clone()]);
    let p2 = func("p2", &[shared.clone()]);
    let top = func("top", &[p1.clone(), p2.clone()]);
    assert_eq!(top.to_string(), "top(p1(s),p2(s))");
    let hash = hash_of(&top);
    assert_eq!(hash, hash_of(&top.clone()));

    // `top` reaches `shared` along two paths; putting it below `shared` is a
    // cycle however it is reached.
    let bad = ErrorKind::InvalidInput;
    assert_kind(shared.push_ast(Attribute::Arguments, &top), bad);
    assert_kind(shared.push_ast(Attribute::Arguments, &p1), bad);
    assert_kind(shared.push_ast(Attribute::Arguments, &p2), bad);
    assert_kind(p1.push_ast(Attribute::Arguments, &top), bad);
    assert_eq!(top.to_string(), "top(p1(s),p2(s))");

    // Adding a sibling with the same child is not.
    p2.push_ast(Attribute::Arguments, &p1).unwrap();
    assert_eq!(p2.to_string(), "p2(s,p1(s))");
    assert_eq!(top.to_string(), "top(p1(s),p2(s,p1(s)))");
    let deep = top.deep_copy().unwrap();
    assert_eq!(deep, top);
}

#[test]
fn a_failed_whole_array_replacement_changes_nothing() {
    let inner = func("a", &[]);
    let outer = func("b", &[inner.clone()]);
    let target = func("t", &[func("keep", &[])]);
    // The last element closes a loop; the earlier ones are fine.
    assert_kind(
        inner.set_ast_array(
            Attribute::Arguments,
            &[func("x", &[]), func("y", &[]), outer.clone()],
        ),
        ErrorKind::InvalidInput,
    );
    assert_eq!(inner.ast_array_len(Attribute::Arguments).unwrap(), 0);
    assert_eq!(target.to_string(), "t(keep)");
    assert_eq!(outer.to_string(), "b(a)");
}

/// Stack for the deep-recursion threads. 512 MiB on 64-bit targets; a 32-bit
/// process has 2 to 4 GiB of address space in all, so 128 MiB there, which is
/// still more than clingo needs at these depths (8 to 64 MB at depth 100 000
/// on 64-bit, whose frames are larger).
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
const DEEP_STACK: usize = if cfg!(target_pointer_width = "64") {
    512 << 20
} else {
    128 << 20
};

/// The check must not recurse: a value thousands of levels deep, on a big
/// stack so that clingo's own recursion (release, `Display`) is not the
/// limit being tested. Building and dropping are clingo's; the check is
/// clingox's.
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
#[test]
fn the_cycle_check_walks_a_deep_value() {
    std::thread::Builder::new()
        .stack_size(DEEP_STACK)
        .spawn(|| {
            let bottom = func("bottom", &[]);
            let mut top = bottom.clone();
            for _ in 0..50_000 {
                top = func("n", &[top]);
            }
            // `top` reaches `bottom` through 50 000 levels.
            assert_kind(
                bottom.push_ast(Attribute::Arguments, &top),
                ErrorKind::InvalidInput,
            );
            assert_eq!(bottom.ast_array_len(Attribute::Arguments).unwrap(), 0);
            // An unrelated node takes the same value without complaint.
            let other = func("other", &[]);
            other.push_ast(Attribute::Arguments, &top).unwrap();
            assert_eq!(other.ast_array_len(Attribute::Arguments).unwrap(), 1);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// The check must not recurse, and this separates the two: a recursive check
/// spends a frame per level, so 100 000 levels overflow a 16 MiB stack (and
/// abort the process), while the explicit stack of the real check does not.
/// Everything runs on the one small thread, `Ast` being `!Send`, so building
/// and dropping the chain must fit in 16 MiB too; both are iterative in
/// clingo for a chain of single-argument functions.
///
/// Not on Windows: there clingo's own recursion, in building and dropping the
/// chain, is what overflows. Measured on the MSVC build, 100 000 levels need more
/// than 128 MiB on `x86_64` and more than 32 MiB on i686, well beyond any stack a
/// recursive check would overflow, so nothing separates the two there.
#[cfg(not(any(target_os = "android", target_family = "wasm", windows)))]
#[test]
fn the_cycle_check_does_not_recurse_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            let bottom = func("bottom", &[]);
            let mut top = bottom.clone();
            for _ in 0..100_000 {
                top = func("n", &[top]);
            }
            // Into its own bottom node: found only by walking all 100 000
            // levels.
            assert_kind(
                bottom.push_ast(Attribute::Arguments, &top),
                ErrorKind::InvalidInput,
            );
            assert_eq!(bottom.ast_array_len(Attribute::Arguments).unwrap(), 0);
            // Into an unrelated node: the whole value is walked and accepted.
            let other = func("other", &[]);
            other.push_ast(Attribute::Arguments, &top).unwrap();
            assert_eq!(other.ast_array_len(Attribute::Arguments).unwrap(), 1);
        })
        .unwrap()
        .join()
        .unwrap();
}

// ---------------------------------------------------------------------------
// Check order: value, then cycle, then index
// ---------------------------------------------------------------------------

/// A NUL byte is reported before an index that is out of range: the error is
/// `Nul`, where the index error is `InvalidInput` with "out of range" (or
/// "beyond the end" for an insertion) in its message.
#[test]
fn a_bad_value_is_reported_before_a_bad_index() {
    let node = operators(); // len 2
    let nul = "a\0b";
    assert_kind(
        node.set_string_at(Attribute::Operators, 99, nul),
        ErrorKind::Nul,
    );
    assert_kind(
        node.insert_string_at(Attribute::Operators, 99, nul),
        ErrorKind::Nul,
    );
    // Control: the same indices with a clean value report the index.
    let set = node
        .set_string_at(Attribute::Operators, 99, "x")
        .unwrap_err();
    assert_eq!(set.kind(), ErrorKind::InvalidInput);
    assert!(set.to_string().contains("out of range"), "{set}");
    let insert = node
        .insert_string_at(Attribute::Operators, 99, "x")
        .unwrap_err();
    assert_eq!(insert.kind(), ErrorKind::InvalidInput);
    assert!(insert.to_string().contains("beyond the end"), "{insert}");
    assert_eq!(node.to_string(), "+ - g");
}

/// A cycle is reported before an index that is out of range. Both are
/// `InvalidInput`, so the message tells them apart: a cycle says "its own
/// descendant", an index says "out of range" or "beyond the end".
#[test]
fn a_cycle_is_reported_before_a_bad_index() {
    let inner = func("a", &[]);
    let outer = func("b", &[inner.clone()]);
    let cycle = |err: Error| {
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let text = err.to_string();
        assert!(text.contains("its own descendant"), "{text}");
        assert!(!text.contains("out of range"), "{text}");
        assert!(!text.contains("beyond the end"), "{text}");
    };
    // `inner` has no elements, so every index here is out of range as well.
    cycle(
        inner
            .set_ast_at(Attribute::Arguments, 5, &outer)
            .unwrap_err(),
    );
    cycle(
        inner
            .insert_ast_at(Attribute::Arguments, 5, &outer)
            .unwrap_err(),
    );
    cycle(
        inner
            .set_ast_at(Attribute::Arguments, 0, &inner)
            .unwrap_err(),
    );
    // Control: a value that is no cycle reports the index.
    let index = inner
        .set_ast_at(Attribute::Arguments, 5, &func("c", &[]))
        .unwrap_err();
    assert_eq!(index.kind(), ErrorKind::InvalidInput);
    assert!(index.to_string().contains("out of range"), "{index}");
    let index = inner
        .insert_ast_at(Attribute::Arguments, 5, &func("c", &[]))
        .unwrap_err();
    assert!(index.to_string().contains("beyond the end"), "{index}");
    assert_eq!(inner.to_string(), "a");
}

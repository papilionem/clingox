//! `Ast`'s identity, ordering, hashing, `Display`, `copy`/`deep_copy`, and the
//! read-only attribute accessors.
//!
//! Every node comes from a real parse (`clingox::ast::parse_string`), never a
//! hand-built fake pointer: the only way to construct an `Ast` is
//! `parse_string`, not `clingo_ast_build`. None of these tests can run under
//! Miri, since every one of them calls into real clingo.
//!
//! Fixtures and expected values were checked against the Python clingo 5.8.2
//! oracle (`clingo.ast`, and `clingo._internal._lib`/`_ffi` directly for the
//! C-level facts pyclingo's own `AST` wrapper hides: raw `clingo_error_code`
//! after a mismatched accessor, and the raw node-pointer identity behind
//! `copy`/`deep_copy`). Every expected value was checked against clingo 5.8.2.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use clingox::ErrorKind;
use clingox::ast::{self, Ast, AstType, Attribute, AttributeType};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

/// Parses `program` and returns every top-level statement in source order,
/// including the implicit leading `#program base.` node clingo always
/// emits first (checked directly: even a bare `"a.\n"` yields two nodes).
fn parse(program: &str) -> Vec<Ast> {
    let mut nodes = Vec::new();
    ast::parse_string(program, |node| {
        nodes.push(node);
        Ok(())
    })
    .expect("the fixture program parses");
    nodes
}

/// The first statement of `program` that is not the implicit
/// `#program base.` node.
fn first_statement(program: &str) -> Ast {
    parse(program)
        .into_iter()
        .find(|n| n.ast_type() != AstType::Program)
        .expect("the fixture has a non-Program statement")
}

/// The first `Rule` node of `program`.
fn rule(program: &str) -> Ast {
    parse(program)
        .into_iter()
        .find(|n| n.ast_type() == AstType::Rule)
        .expect("the fixture has a rule")
}

fn hash_of(a: &Ast) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    a.hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------------------
// ast_type / has_attribute / attribute_type
// ---------------------------------------------------------------------------

#[test]
fn ast_type_matches_what_the_program_parsed_to() {
    let nodes = parse("a.\n");
    assert_eq!(nodes[0].ast_type(), AstType::Program);
    assert_eq!(nodes[1].ast_type(), AstType::Rule);
}

#[test]
fn has_attribute_and_attribute_type_agree_on_every_attribute_a_rule_has() {
    let r = rule("a.\n");
    for (attribute, expected) in [
        (Attribute::Location, AttributeType::Location),
        (Attribute::Head, AttributeType::Ast),
        (Attribute::Body, AttributeType::AstArray),
    ] {
        assert!(r.has_attribute(attribute), "{attribute:?}");
        assert_eq!(r.attribute_type(attribute), Some(expected), "{attribute:?}");
    }
}

#[test]
fn has_attribute_is_false_and_attribute_type_is_none_for_an_attribute_the_node_lacks() {
    let r = rule("a.\n");
    // Only `SymbolicTerm` has a `symbol` attribute; `Rule` does not
    // (`C(rule){A(location, location), A(head, ast), A(body, ast_array)}`,
    // control.cc:1472). Checked live: `clingo_ast_has_attribute` on a real
    // `Rule` node returns `false` for `clingo_ast_attribute_symbol`.
    assert!(!r.has_attribute(Attribute::Symbol));
    assert_eq!(r.attribute_type(Attribute::Symbol), None);
}

#[test]
fn coefficient_is_a_real_variant_no_constructor_ever_uses() {
    // `Attribute::Coefficient` is in `g_clingo_ast_attribute_names` but not
    // in any of the 46 constructors' own argument lists (control.cc:1432-
    // 1487, checked directly: no `A(coefficient, ...)` anywhere in that
    // range). Every node this clingo can produce must therefore answer
    // `false`/`None` for it.
    for node in parse("a :- b, not c, X > 1.\n") {
        assert!(
            !node.has_attribute(Attribute::Coefficient),
            "{:?}",
            node.ast_type()
        );
        assert_eq!(node.attribute_type(Attribute::Coefficient), None);
    }
}

// ---------------------------------------------------------------------------
// Typed accessors: one success path per `AttributeType`
// ---------------------------------------------------------------------------

#[test]
fn number_reads_a_boolean_constants_value() {
    // "a :- #true." oracle: body[0] is Literal(sign=0,
    // atom=BooleanConstant(1)).
    let r = rule("a :- #true.\n");
    let body0 = r.ast_at(Attribute::Body, 0).unwrap();
    let atom = body0.ast(Attribute::Atom).unwrap();
    assert_eq!(atom.ast_type(), AstType::BooleanConstant);
    assert_eq!(atom.number(Attribute::Value).unwrap(), 1);
}

#[test]
fn number_reads_a_literals_sign() {
    // Oracle: "a." -> sign 0; "not a." -> sign 1; "not not a." -> sign 2.
    assert_eq!(
        rule("a.\n")
            .ast(Attribute::Head)
            .unwrap()
            .number(Attribute::Sign)
            .unwrap(),
        0
    );
    assert_eq!(
        rule("not a.\n")
            .ast(Attribute::Head)
            .unwrap()
            .number(Attribute::Sign)
            .unwrap(),
        1
    );
    assert_eq!(
        rule("not not a.\n")
            .ast(Attribute::Head)
            .unwrap()
            .number(Attribute::Sign)
            .unwrap(),
        2
    );
}

#[test]
fn number_reads_a_functions_external_flag() {
    // "p(@f(1))." oracle: outer `p` has external=0, the argument `@f(1)` has
    // external=1 (the `@`-prefixed external-function-call syntax).
    let r = rule("p(@f(1)).\n");
    let p = r
        .ast(Attribute::Head)
        .unwrap()
        .ast(Attribute::Atom)
        .unwrap()
        .ast(Attribute::Symbol)
        .unwrap();
    assert_eq!(p.ast_type(), AstType::Function);
    assert_eq!(p.number(Attribute::External).unwrap(), 0);
    let f = p.ast_at(Attribute::Arguments, 0).unwrap();
    assert_eq!(f.ast_type(), AstType::Function);
    assert_eq!(f.number(Attribute::External).unwrap(), 1);
}

#[test]
fn symbol_reads_a_symbolic_terms_value() {
    // "q(X) :- p(X), X > 1." oracle: body[1] is Literal(Comparison(term=
    // Variable(X), guards=[Guard(comparison=0, term=SymbolicTerm(1))])).
    let r = rule("q(X) :- p(X), X > 1.\n");
    let comparison = r
        .ast_at(Attribute::Body, 1)
        .unwrap()
        .ast(Attribute::Atom)
        .unwrap();
    assert_eq!(comparison.ast_type(), AstType::Comparison);
    let guard = comparison.ast_at(Attribute::Guards, 0).unwrap();
    assert_eq!(guard.ast_type(), AstType::Guard);
    let term = guard.ast(Attribute::Term).unwrap();
    assert_eq!(term.ast_type(), AstType::SymbolicTerm);
    assert_eq!(
        term.symbol(Attribute::Symbol).unwrap(),
        clingox::Symbol::number(1)
    );
}

#[test]
fn span_reads_a_rules_location() {
    // "a.\n" oracle: begin=(line 1, column 1), end=(line 1, column 3), same
    // file both ends (the literal "a." is two characters plus the implicit
    // end-of-statement column).
    let r = rule("a.\n");
    let span = r.span(Attribute::Location).unwrap();
    assert_eq!((span.begin_line(), span.begin_column()), (1, 1));
    assert_eq!((span.end_line(), span.end_column()), (1, 3));
    assert!(span.is_single_file());
    assert_eq!(span.to_string(), format!("{}:1:1-3", span.begin_file()));
}

#[test]
fn string_reads_names() {
    let r = rule("p(@f(1)).\n");
    let p = r
        .ast(Attribute::Head)
        .unwrap()
        .ast(Attribute::Atom)
        .unwrap()
        .ast(Attribute::Symbol)
        .unwrap();
    assert_eq!(p.string(Attribute::Name).unwrap(), "p");
    let f = p.ast_at(Attribute::Arguments, 0).unwrap();
    assert_eq!(f.string(Attribute::Name).unwrap(), "f");

    // "q(X) :- p(X), X > 1." oracle: the argument of `q` is Variable("X").
    let q_arg = rule("q(X) :- p(X), X > 1.\n")
        .ast(Attribute::Head)
        .unwrap()
        .ast(Attribute::Atom)
        .unwrap()
        .ast(Attribute::Symbol)
        .unwrap()
        .ast_at(Attribute::Arguments, 0)
        .unwrap();
    assert_eq!(q_arg.ast_type(), AstType::Variable);
    assert_eq!(q_arg.string(Attribute::Name).unwrap(), "X");
}

#[test]
fn ast_reads_an_owned_child_that_outlives_the_parent() {
    let r = rule("a.\n");
    let head = r.ast(Attribute::Head).unwrap();
    drop(r);
    // `head` owns its own reference (clingo_ast_attribute_get_ast incRefs
    // before returning, control.cc:1708-1715); dropping the parent handle
    // must not invalidate it.
    assert_eq!(head.ast_type(), AstType::Literal);
    assert_eq!(head.to_string(), "a");
}

#[test]
fn optional_ast_is_some_and_none_depending_on_whether_the_bound_is_present() {
    // Oracle: a single-sided bound normalizes to `left_guard = Some`,
    // `right_guard = None` (gringo flips `> 3` into `left_guard: < 3`, never
    // producing a `right_guard` for a lone bound); a two-sided bound gives
    // both.
    let one_sided = rule(":- #count{X : p(X)} > 3.\n")
        .ast_at(Attribute::Body, 0)
        .unwrap()
        .ast(Attribute::Atom)
        .unwrap();
    assert_eq!(one_sided.ast_type(), AstType::BodyAggregate);
    assert!(
        one_sided
            .optional_ast(Attribute::LeftGuard)
            .unwrap()
            .is_some()
    );
    assert!(
        one_sided
            .optional_ast(Attribute::RightGuard)
            .unwrap()
            .is_none()
    );

    let two_sided = rule(":- 1 < #count{X : p(X)} < 3.\n")
        .ast_at(Attribute::Body, 0)
        .unwrap()
        .ast(Attribute::Atom)
        .unwrap();
    assert!(
        two_sided
            .optional_ast(Attribute::LeftGuard)
            .unwrap()
            .is_some()
    );
    assert!(
        two_sided
            .optional_ast(Attribute::RightGuard)
            .unwrap()
            .is_some()
    );
}

#[test]
fn ast_array_accessors_read_elements_string_at_string_and_array_length() {
    // "#program p(x,y)." oracle: Program.parameters is [Id("x"), Id("y")].
    let program_node = parse("#program p(x,y).\na.\n")
        .into_iter()
        .find(|n| n.ast_type() == AstType::Program && n.string(Attribute::Name).unwrap() == "p")
        .unwrap();
    assert_eq!(
        program_node.ast_array_len(Attribute::Parameters).unwrap(),
        2
    );
    let x = program_node.ast_at(Attribute::Parameters, 0).unwrap();
    let y = program_node.ast_at(Attribute::Parameters, 1).unwrap();
    assert_eq!(x.ast_type(), AstType::Id);
    assert_eq!(x.string(Attribute::Name).unwrap(), "x");
    assert_eq!(y.string(Attribute::Name).unwrap(), "y");
}

/// A theory atom definition's guard, the only place a `string_array`
/// attribute appears without a full theory-term grammar
/// (`C(theory_guard_definition){A(operators, string_array), A(term,
/// string)}`, control.cc:1462). Oracle: `{<,<=,=}` gives `operators =
/// ["<", "<=", "="]`.
fn theory_guard() -> Ast {
    let program = "#theory t {\n  term { };\n  &a/0 : term, {<,<=,=}, term, any\n}.\n";
    let definition = parse(program)
        .into_iter()
        .find(|n| n.ast_type() == AstType::TheoryDefinition)
        .unwrap();
    let atom_definition = definition.ast_at(Attribute::Atoms, 0).unwrap();
    assert_eq!(atom_definition.ast_type(), AstType::TheoryAtomDefinition);
    let guard = atom_definition
        .optional_ast(Attribute::Guard)
        .unwrap()
        .unwrap();
    assert_eq!(guard.ast_type(), AstType::TheoryGuardDefinition);
    guard
}

#[test]
fn string_array_accessors_read_elements_and_length() {
    let guard = theory_guard();
    assert_eq!(guard.string_array_len(Attribute::Operators).unwrap(), 3);
    assert_eq!(guard.string_at(Attribute::Operators, 0).unwrap(), "<");
    assert_eq!(guard.string_at(Attribute::Operators, 1).unwrap(), "<=");
    assert_eq!(guard.string_at(Attribute::Operators, 2).unwrap(), "=");
    assert_eq!(guard.string(Attribute::Term).unwrap(), "term");
}

// ---------------------------------------------------------------------------
// Failure paths: absent attribute, wrong `AttributeType`, out-of-range index
// ---------------------------------------------------------------------------

#[test]
fn a_typed_accessor_on_an_absent_attribute_is_invalid_input() {
    let r = rule("a.\n");
    // `Rule` has no `symbol` attribute.
    assert_eq!(
        r.symbol(Attribute::Symbol).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.number(Attribute::Symbol).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.string(Attribute::Symbol).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn a_typed_accessor_with_the_wrong_attribute_type_is_invalid_input_not_runtime_or_logic() {
    // `Rule.body` is an `ast_array`; every other typed accessor on it must
    // fail `InvalidInput`, never the raw C-level codes a direct call
    // produces (checked live: `clingo_ast_attribute_get_number` on an
    // `ast_array` attribute raises `clingo_error_code() == 1`, `Runtime`,
    // from `mpark::bad_variant_access`; clingox must not let that through).
    let r = rule("a.\n");
    assert_eq!(
        r.number(Attribute::Body).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.symbol(Attribute::Body).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.span(Attribute::Body).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.string(Attribute::Body).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.ast(Attribute::Body).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.optional_ast(Attribute::Body).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.string_array_len(Attribute::Body).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );

    // `Rule.head` is a required `ast`, not `optional_ast`.
    assert_eq!(
        r.optional_ast(Attribute::Head).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn ast_at_out_of_range_is_invalid_input_the_boundary_succeeds() {
    // Checked live: a raw `clingo_ast_attribute_get_ast_at` one past the end
    // raises `clingo_error_code() == 2` (`Logic`, from `std::vector::at`'s
    // `std::out_of_range`). clingox must report `InvalidInput` uniformly
    // instead.
    let r = rule("q(X) :- p(X), X > 1.\n");
    let len = r.ast_array_len(Attribute::Body).unwrap();
    assert_eq!(len, 2);
    assert!(
        r.ast_at(Attribute::Body, len - 1).is_ok(),
        "the last valid index succeeds"
    );
    assert_eq!(
        r.ast_at(Attribute::Body, len).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.ast_at(Attribute::Body, len + 100).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn string_at_out_of_range_is_invalid_input_the_boundary_succeeds() {
    let guard = theory_guard();
    let len = guard.string_array_len(Attribute::Operators).unwrap();
    assert_eq!(len, 3);
    assert!(guard.string_at(Attribute::Operators, len - 1).is_ok());
    assert_eq!(
        guard
            .string_at(Attribute::Operators, len)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
}

// ---------------------------------------------------------------------------
// Identity, `Eq`/`Ord`/`Hash`, `Display`, `Debug`
// ---------------------------------------------------------------------------

#[test]
fn ptr_eq_is_identity_not_structural_equality() {
    let a = rule("a.\n");
    let b = rule("a.\n"); // a separate parse: structurally equal, a different node
    assert_eq!(a, b);
    assert!(!a.ptr_eq(&b));
    let clone = a.clone();
    assert!(a.ptr_eq(&clone));
}

/// A clingo AST node has no state reachable except through its own pointer, so
/// `ptr_eq` being `true` on a child *is* "a mutation applied through one handle
/// is visible through the other," not merely evidence for it: once setters
/// exist, `shallow.ast(Head)?.set_number(...)` and `original_head.number(...)`
/// read the same node this test already pins. Confirmed live with pyclingo's
/// own setters: `shallow.head.atom.symbol.name = "z"` changes
/// `rule.head.atom.symbol.name` too, while the equivalent mutation through
/// `deep` does not.
#[test]
fn copy_is_shallow_children_are_shared_with_the_original() {
    let r = rule("a :- b, not c.\n");
    let shallow = r.copy().unwrap();
    assert!(!r.ptr_eq(&shallow), "the top node is new");
    assert_eq!(r, shallow);
    let original_head = r.ast(Attribute::Head).unwrap();
    let copy_head = shallow.ast(Attribute::Head).unwrap();
    assert!(
        original_head.ptr_eq(&copy_head),
        "a shallow copy shares its children with the original (astv2.cc:150-155), \
         so a future mutation through either handle would be visible through both"
    );
}

/// The deep-copy half of the same claim: no child is shared, so a future
/// mutation through `deep` would never be
/// visible through `r`. See `copy_is_shallow_children_are_shared_with_the_
/// original`'s own doc comment for the live confirmation this rests on.
#[test]
fn deep_copy_children_are_independent_new_nodes() {
    let r = rule("a :- b, not c.\n");
    let deep = r.deep_copy().unwrap();
    assert!(!r.ptr_eq(&deep), "the top node is new");
    assert_eq!(r, deep);
    let original_head = r.ast(Attribute::Head).unwrap();
    let deep_head = deep.ast(Attribute::Head).unwrap();
    assert!(
        !original_head.ptr_eq(&deep_head),
        "a deep copy does not share children with the original (astv2.cc:157-162), \
         so a future mutation through either handle would never reach the other"
    );
    assert_eq!(original_head, deep_head, "still structurally equal");
}

#[test]
fn eq_and_hash_ignore_location_matching_pyclingos_own_claim() {
    // pyclingo's own doc: "ordered structurally ignoring the location"
    // (ast.py:979-982). Checked live: two parses of the same content at
    // different positions compare equal and hash equally, while their own
    // `location` attributes differ.
    let a = rule("a.\n");
    let b = rule("\n\n   a.\n");
    assert_ne!(
        a.span(Attribute::Location).unwrap(),
        b.span(Attribute::Location).unwrap()
    );
    assert_eq!(a, b);
    assert_eq!(hash_of(&a), hash_of(&b));

    let c = rule("b.\n");
    assert_ne!(a, c);
}

#[test]
fn ord_is_consistent_with_eq_and_ignores_location() {
    let a = rule("a.\n");
    let b = rule("\n a.\n");
    assert_eq!(a.cmp(&b), Ordering::Equal);
    assert_eq!(a.partial_cmp(&b), Some(Ordering::Equal));
    let c = rule("b.\n");
    assert_ne!(a.cmp(&c), Ordering::Equal);
}

#[test]
fn display_matches_the_oracles_exact_text() {
    assert_eq!(rule("a :- b, not c.\n").to_string(), "a :- b; not c.");
    assert_eq!(first_statement("#show p/1.\n").to_string(), "#show p/1.");
    assert_eq!(
        first_statement("#external e.\n").to_string(),
        "#external e. [false]"
    );
}

#[test]
fn display_round_trips_for_a_handful_of_statement_kinds() {
    // pyclingo's own doc: "the string representation of any AST obtained
    // from parse_files and parse_string can be parsed again" (ast.py:983).
    // Checked live for each fixture below.
    for program in [
        "a.\n",
        "q(X) :- p(X), X > 1.\n",
        ":- #count{X : p(X)} > 3.\n",
        "#show p/1.\n",
        "#external e.\n",
    ] {
        let original = first_statement(program);
        let text = original.to_string();
        let reparsed = first_statement(&text);
        assert_eq!(original, reparsed, "program {program:?} printed {text:?}");
    }
}

#[test]
fn debug_shows_the_type_and_attribute_count_never_the_raw_pointer() {
    // The fallback Debug: "Ast(<type>,
    // <n> attributes)". `Rule` has exactly 3 attributes (location, head,
    // body; C(rule), control.cc:1472).
    let text = format!("{:?}", rule("a.\n"));
    assert_eq!(text, "Ast(Rule, 3 attributes)");
    assert!(
        !text.contains("0x"),
        "must never print the raw pointer (RULES 11.3): {text}"
    );
}

// ---------------------------------------------------------------------------
// Clone/Drop refcounting (negative control 2: swapped Clone/Drop)
// ---------------------------------------------------------------------------

#[test]
fn cloning_and_dropping_many_times_frees_the_node_exactly_once() {
    // Run under `cargo xtask sanitize` (ASan, LSan): a swapped Clone/Drop
    // (Clone releases, Drop acquires) either double-frees on the first extra
    // drop here (ASan abort) or leaks every clone (LSan report at exit).
    let r = rule("a.\n");
    let clones: Vec<Ast> = (0..50).map(|_| r.clone()).collect();
    for clone in &clones {
        assert_eq!(*clone, r);
    }
    drop(clones);
    // `r` is still the original reference and must still be valid.
    assert_eq!(r.ast_type(), AstType::Rule);
    assert_eq!(r.to_string(), "a.");
}

// ---------------------------------------------------------------------------
// Ord direction (swapping Less and Greater survived)
// ---------------------------------------------------------------------------

#[test]
fn ord_puts_the_nodes_in_the_order_the_oracle_gives() {
    // pyclingo 5.8.2: with a = "a.", b = "b.", c = "a :- b." (rules), `a < b`
    // is True and `b < a` False, `a < c` is True and `c < a` False, and
    // `sorted([b, c, a])` prints `['a.', 'a :- b.', 'b.']`
    // (checked directly).
    let a = rule("a.\n");
    let b = rule("b.\n");
    let c = rule("a :- b.\n");

    assert!(a < b);
    assert!(b >= a);
    assert!(b > a);
    assert_eq!(a.cmp(&b), Ordering::Less);
    assert_eq!(b.cmp(&a), Ordering::Greater);
    assert_eq!(a.partial_cmp(&b), Some(Ordering::Less));
    assert_eq!(b.partial_cmp(&a), Some(Ordering::Greater));
    assert_eq!(a.cmp(&c), Ordering::Less);
    assert_eq!(c.cmp(&a), Ordering::Greater);

    let mut nodes = [b, c, a];
    nodes.sort();
    let texts: Vec<String> = nodes.iter().map(ToString::to_string).collect();
    assert_eq!(texts, ["a.", "a :- b.", "b."].map(str::to_owned));
}

// ---------------------------------------------------------------------------
// The kind check in string_at, ast_at and ast_array_len
// ---------------------------------------------------------------------------

#[test]
fn string_at_ast_at_and_ast_array_len_on_the_wrong_kind_are_invalid_input() {
    // Checked live: each of these raises a raw `clingo_error_code() == 1`
    // (`Runtime`, `bad_variant_access`) when called directly on an
    // attribute of another kind, which clingox must not let through. The
    // index is in range for the kind the attribute really has, so only the
    // kind check can reject it.
    let r = rule("q(X) :- p(X), X > 1.\n");
    // `Body` is an `ast_array` of two elements: `string_at` is the wrong kind.
    assert_eq!(r.ast_array_len(Attribute::Body).unwrap(), 2);
    assert_eq!(
        r.string_at(Attribute::Body, 0).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    // `Head` is a single `ast`, not an array.
    assert_eq!(
        r.ast_at(Attribute::Head, 0).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.ast_array_len(Attribute::Head).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    // `Location` is neither.
    assert_eq!(
        r.ast_at(Attribute::Location, 0).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        r.ast_array_len(Attribute::Location).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );

    // `Operators` is a `string_array` of three elements: `ast_at` and
    // `ast_array_len` are the wrong kind, and so is `string_at` on a plain
    // `string`.
    let guard = theory_guard();
    assert_eq!(guard.string_array_len(Attribute::Operators).unwrap(), 3);
    assert_eq!(
        guard.ast_at(Attribute::Operators, 0).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        guard
            .ast_array_len(Attribute::Operators)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        guard.string_at(Attribute::Term, 0).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    // And an attribute the node lacks entirely.
    assert_eq!(
        guard.string_at(Attribute::Body, 0).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

// ---------------------------------------------------------------------------
// Span: end_line and the multi-line Display
// ---------------------------------------------------------------------------

#[test]
fn a_multi_line_span_reports_its_end_line_and_prints_both_lines() {
    // pyclingo 5.8.2: "a :-\n  b." gives the rule's location
    // `<string>:1:1` to `<string>:2:5`; "p(1).\n\n  q :- r,\n   s." gives the
    // last rule `3:3` to `4:6`.
    let r = rule("a :-\n  b.\n");
    let span = r.span(Attribute::Location).unwrap();
    assert_eq!((span.begin_line(), span.begin_column()), (1, 1));
    assert_eq!((span.end_line(), span.end_column()), (2, 5));
    assert!(span.is_single_file());
    assert_eq!(span.to_string(), "<string>:1:1-2:5");

    let later = parse("p(1).\n\n  q :- r,\n   s.\n").pop().unwrap();
    let span = later.span(Attribute::Location).unwrap();
    assert_eq!((span.begin_line(), span.begin_column()), (3, 3));
    assert_eq!((span.end_line(), span.end_column()), (4, 6));
    assert_eq!(span.to_string(), "<string>:3:3-4:6");
}

// ---------------------------------------------------------------------------
// Ast::string returns an owned String
// ---------------------------------------------------------------------------

#[test]
fn an_owned_string_from_ast_string_survives_every_handle_to_its_node() {
    // The type annotations pin the signature: a `&str` borrowed from the node
    // would not compile here. The churn after the drops reuses the freed
    // memory, so a string that still pointed into a freed node would read
    // garbage and fail the assertions (a clean failure, not a crash).
    let (name, operator, element): (String, String, String) = {
        let program = parse("#program pname(x).\n")
            .into_iter()
            .find(|n| n.string(Attribute::Name).is_ok_and(|name| name == "pname"))
            .unwrap();
        let name = program.string(Attribute::Name).unwrap();
        let guard = theory_guard();
        let operator = guard.string_at(Attribute::Operators, 1).unwrap();
        let element = guard.string(Attribute::Term).unwrap();
        (name, operator, element)
        // `program`, `guard` and every other handle are dropped here.
    };
    for round in 0..200 {
        let churn = parse(&format!("zzzzzzzz{round}(yyyyyyyy{round}).\n"));
        drop(churn);
    }
    assert_eq!(name, "pname");
    assert_eq!(operator, "<=");
    assert_eq!(element, "term");
}

//! `Visitor` and `walk`: the Rust shape of pyclingo's `Transformer`.
//!
//! Oracle: pyclingo 5.8.2 (`clingo.ast.Transformer` and `AST.update`).
//! The oracle shows that a node the visit returns
//! unchanged comes back as the very same object, only a changed node and its
//! ancestors are rebuilt, children are visited in constructor-table order
//! (arrays element by element, `None` skipped), and `update` keeps the
//! location. The expected visit orders below were recorded from a
//! `Transformer` whose `visit` logs `ast_type` before delegating. The tests
//! need a real clingo, so none runs under Miri.

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
    self, AggregateFunction, Ast, AstType, Attribute, BinaryOperator, CommentType,
    ComparisonOperator, LiteralSign, Span, TheoryAtomType, TheoryOperatorType, TheorySequenceType,
    UnaryOperator, Visitor, walk,
};
use clingox::{Error, ErrorKind, Result, Symbol};

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

fn parse(program: &str) -> Vec<Ast> {
    let mut nodes = Vec::new();
    ast::parse_string(program, |node| {
        nodes.push(node);
        Ok(())
    })
    .unwrap();
    nodes
}

/// One node of each of the 46 kinds, built through the constructors.
fn all_kinds() -> Vec<(AstType, Ast)> {
    let p = P::new();
    vec![
        (AstType::Id, ast::id(&p.s, "x").unwrap()),
        (AstType::Variable, ast::variable(&p.s, "X").unwrap()),
        (
            AstType::SymbolicTerm,
            ast::symbolic_term(&p.s, sym("a")).unwrap(),
        ),
        (
            AstType::UnaryOperation,
            ast::unary_operation(&p.s, UnaryOperator::Minus, &p.x).unwrap(),
        ),
        (
            AstType::BinaryOperation,
            ast::binary_operation(&p.s, BinaryOperator::Plus, &p.x, &p.one).unwrap(),
        ),
        (
            AstType::Interval,
            ast::interval(&p.s, &p.one, &p.x).unwrap(),
        ),
        (
            AstType::Function,
            ast::function(&p.s, "f", &[p.a.clone(), p.x.clone()], false).unwrap(),
        ),
        (
            AstType::Pool,
            ast::pool(&p.s, &[p.a.clone(), p.one.clone()]).unwrap(),
        ),
        (
            AstType::BooleanConstant,
            ast::boolean_constant(true).unwrap(),
        ),
        (AstType::SymbolicAtom, ast::symbolic_atom(&p.fa).unwrap()),
        (
            AstType::Comparison,
            ast::comparison(&p.x, &[p.glt.clone()]).unwrap(),
        ),
        (
            AstType::Guard,
            ast::guard(ComparisonOperator::LessEqual, &p.one).unwrap(),
        ),
        (
            AstType::ConditionalLiteral,
            ast::conditional_literal(&p.s, &p.lit, &[p.nlit.clone()]).unwrap(),
        ),
        (
            AstType::Aggregate,
            ast::aggregate(&p.s, Some(&p.guard), &[p.cl.clone()], None).unwrap(),
        ),
        (
            AstType::BodyAggregateElement,
            ast::body_aggregate_element(&[p.x.clone()], &[p.lit.clone()]).unwrap(),
        ),
        (
            AstType::BodyAggregate,
            ast::body_aggregate(
                &p.s,
                None,
                AggregateFunction::Sum,
                &[p.bae.clone()],
                Some(&p.guard),
            )
            .unwrap(),
        ),
        (
            AstType::HeadAggregateElement,
            ast::head_aggregate_element(&[p.x.clone()], &p.cl).unwrap(),
        ),
        (
            AstType::HeadAggregate,
            ast::head_aggregate(
                &p.s,
                Some(&p.guard),
                AggregateFunction::Count,
                &[p.hae.clone()],
                None,
            )
            .unwrap(),
        ),
        (
            AstType::Disjunction,
            ast::disjunction(&p.s, &[p.cl.clone()]).unwrap(),
        ),
        (
            AstType::TheorySequence,
            ast::theory_sequence(&p.s, TheorySequenceType::List, &[p.tf.clone()]).unwrap(),
        ),
        (
            AstType::TheoryFunction,
            ast::theory_function(&p.s, "g", &[p.a.clone()]).unwrap(),
        ),
        (
            AstType::TheoryUnparsedTermElement,
            ast::theory_unparsed_term_element(&["+"], &p.tf).unwrap(),
        ),
        (
            AstType::TheoryUnparsedTerm,
            ast::theory_unparsed_term(&p.s, &[p.tue.clone()]).unwrap(),
        ),
        (
            AstType::TheoryGuard,
            ast::theory_guard("<=", &p.tut).unwrap(),
        ),
        (
            AstType::TheoryAtomElement,
            ast::theory_atom_element(&[p.tut.clone()], &[p.lit.clone()]).unwrap(),
        ),
        (
            AstType::TheoryAtom,
            ast::theory_atom(&p.s, &p.fa, &[p.tae.clone()], Some(&p.tg)).unwrap(),
        ),
        (
            AstType::Literal,
            ast::literal(&p.s, LiteralSign::Negation, &p.satom_a).unwrap(),
        ),
        (
            AstType::TheoryOperatorDefinition,
            ast::theory_operator_definition(&p.s, "+", 5, TheoryOperatorType::BinaryLeft).unwrap(),
        ),
        (
            AstType::TheoryTermDefinition,
            ast::theory_term_definition(&p.s, "t", &[p.od.clone()]).unwrap(),
        ),
        (
            AstType::TheoryGuardDefinition,
            ast::theory_guard_definition(&["<="], "t").unwrap(),
        ),
        (
            AstType::TheoryAtomDefinition,
            ast::theory_atom_definition(&p.s, TheoryAtomType::Any, "p", 1, "t", Some(&p.gd))
                .unwrap(),
        ),
        (
            AstType::Rule,
            ast::rule(&p.s, &p.lit, &[p.nlit.clone()]).unwrap(),
        ),
        (
            AstType::Definition,
            ast::definition(&p.s, "k", &p.one, false).unwrap(),
        ),
        (
            AstType::ShowSignature,
            ast::show_signature(&p.s, "p", 2, true).unwrap(),
        ),
        (
            AstType::ShowTerm,
            ast::show_term(&p.s, &p.x, &[p.lit.clone()]).unwrap(),
        ),
        (
            AstType::Minimize,
            ast::minimize(&p.s, &p.one, &p.one, &[p.x.clone()], &[p.lit.clone()]).unwrap(),
        ),
        (AstType::Script, ast::script(&p.s, "python", "#x").unwrap()),
        (AstType::Program, ast::program(&p.s, "base", &[]).unwrap()),
        (
            AstType::External,
            ast::external(&p.s, &p.satom, &[p.nlit.clone()], &p.truth).unwrap(),
        ),
        (
            AstType::Edge,
            ast::edge(&p.s, &p.x, &p.one, &[p.lit.clone()]).unwrap(),
        ),
        (
            AstType::Heuristic,
            ast::heuristic(&p.s, &p.satom, &[p.nlit.clone()], &p.one, &p.one, &p.a).unwrap(),
        ),
        (
            AstType::ProjectAtom,
            ast::project_atom(&p.s, &p.satom, &[p.lit.clone()]).unwrap(),
        ),
        (
            AstType::ProjectSignature,
            ast::project_signature(&p.s, "p", 1, false).unwrap(),
        ),
        (AstType::Defined, ast::defined(&p.s, "p", 1, true).unwrap()),
        (
            AstType::TheoryDefinition,
            ast::theory_definition(&p.s, "th", &[p.td.clone()], &[p.ad.clone()]).unwrap(),
        ),
        (
            AstType::Comment,
            ast::comment(&p.s, "% hi", CommentType::Line).unwrap(),
        ),
    ]
}

macro_rules! recording_visitor {
    ($($method:ident, $kind:ident;)+) => {
        /// Overrides every `visit_*`: checks it was dispatched the right
        /// kind, logs it, and recurses.
        #[derive(Default)]
        struct Recorder {
            log: Vec<AstType>,
        }

        impl Visitor for Recorder {
            $(
                fn $method(&mut self, node: &Ast) -> Result<Ast> {
                    assert_eq!(node.ast_type(), AstType::$kind);
                    self.log.push(AstType::$kind);
                    walk(self, node)
                }
            )+
        }
    };
}

recording_visitor! {
visit_id, Id;
visit_variable, Variable;
visit_symbolic_term, SymbolicTerm;
visit_unary_operation, UnaryOperation;
visit_binary_operation, BinaryOperation;
visit_interval, Interval;
visit_function, Function;
visit_pool, Pool;
visit_boolean_constant, BooleanConstant;
visit_symbolic_atom, SymbolicAtom;
visit_comparison, Comparison;
visit_guard, Guard;
visit_conditional_literal, ConditionalLiteral;
visit_aggregate, Aggregate;
visit_body_aggregate_element, BodyAggregateElement;
visit_body_aggregate, BodyAggregate;
visit_head_aggregate_element, HeadAggregateElement;
visit_head_aggregate, HeadAggregate;
visit_disjunction, Disjunction;
visit_theory_sequence, TheorySequence;
visit_theory_function, TheoryFunction;
visit_theory_unparsed_term_element, TheoryUnparsedTermElement;
visit_theory_unparsed_term, TheoryUnparsedTerm;
visit_theory_guard, TheoryGuard;
visit_theory_atom_element, TheoryAtomElement;
visit_theory_atom, TheoryAtom;
visit_literal, Literal;
visit_theory_operator_definition, TheoryOperatorDefinition;
visit_theory_term_definition, TheoryTermDefinition;
visit_theory_guard_definition, TheoryGuardDefinition;
visit_theory_atom_definition, TheoryAtomDefinition;
visit_rule, Rule;
visit_definition, Definition;
visit_show_signature, ShowSignature;
visit_show_term, ShowTerm;
visit_minimize, Minimize;
visit_script, Script;
visit_program, Program;
visit_external, External;
visit_edge, Edge;
visit_heuristic, Heuristic;
visit_project_atom, ProjectAtom;
visit_project_signature, ProjectSignature;
visit_defined, Defined;
visit_theory_definition, TheoryDefinition;
visit_comment, Comment;}

macro_rules! types {
    ($($kind:ident),* $(,)?) => { vec![$(AstType::$kind),*] };
}

/// A visitor that changes nothing.
struct Noop;
impl Visitor for Noop {}

/// Renames every variable to `V_<name>` (through `copy`, so the original
/// variable is untouched).
struct Rename;
impl Visitor for Rename {
    fn visit_variable(&mut self, node: &Ast) -> Result<Ast> {
        let name = node.string(Attribute::Name)?;
        let renamed = node.copy()?;
        renamed.set_string(Attribute::Name, &format!("V_{name}"))?;
        Ok(renamed)
    }
}

// ---------------------------------------------------------------------------
// Identity: nothing changed, nothing rebuilt
// ---------------------------------------------------------------------------

#[test]
fn the_default_visitor_returns_every_statement_itself() {
    let program = "p(X,a) :- q(b), not r. { a: b } :- c. #count { X : p(X) } > 1 :- q. \
                   #show p/1. #const k=3. :~ p(X). [X@1,X] a; b :- c.";
    for node in parse(program) {
        let out = Noop.visit(&node).unwrap();
        assert!(out.ptr_eq(&node), "{node}");
        assert_eq!(out.to_string(), node.to_string());
    }
}

#[test]
fn walk_on_a_leaf_returns_it_unchanged() {
    let variable = ast::variable(&P::new().s, "X").unwrap();
    assert!(walk(&mut Noop, &variable).unwrap().ptr_eq(&variable));
}

// ---------------------------------------------------------------------------
// Transformation: one override, everything else defaulted
// ---------------------------------------------------------------------------

#[test]
fn only_the_changed_statement_and_its_ancestors_are_rebuilt() {
    // Oracle: `p(X,a) :- q(b).` becomes `p(V_X,a) :- q(b).`; the other
    // statements come back as the same objects; inside the changed rule the
    // untouched argument `a` and the untouched body literal are shared with
    // the original and the head is new.
    let nodes = parse("p(X,a) :- q(b). r(a). s :- t.");
    let mut out = Vec::new();
    for node in &nodes {
        out.push(Rename.visit(node).unwrap());
    }
    assert_eq!(out[1].to_string(), "p(V_X,a) :- q(b).");
    assert!(!out[1].ptr_eq(&nodes[1]));
    for i in [0, 2, 3] {
        assert!(out[i].ptr_eq(&nodes[i]), "statement {i}");
    }

    let head_before = nodes[1].ast(Attribute::Head).unwrap();
    let head_after = out[1].ast(Attribute::Head).unwrap();
    assert!(!head_after.ptr_eq(&head_before));
    let atom_before = head_before.ast(Attribute::Atom).unwrap();
    let atom_after = head_after.ast(Attribute::Atom).unwrap();
    assert!(!atom_after.ptr_eq(&atom_before));
    let symbol_before = atom_before.ast(Attribute::Symbol).unwrap();
    let symbol_after = atom_after.ast(Attribute::Symbol).unwrap();
    assert!(!symbol_after.ptr_eq(&symbol_before));
    assert!(
        symbol_after
            .ast_at(Attribute::Arguments, 1)
            .unwrap()
            .ptr_eq(&symbol_before.ast_at(Attribute::Arguments, 1).unwrap()),
        "the untouched argument is shared"
    );
    assert!(
        out[1]
            .ast_at(Attribute::Body, 0)
            .unwrap()
            .ptr_eq(&nodes[1].ast_at(Attribute::Body, 0).unwrap()),
        "the untouched body literal is shared"
    );
}

#[test]
fn a_rebuilt_node_keeps_its_location_and_the_original_is_untouched() {
    let nodes = parse("\n  p(X).");
    let rule = &nodes[1];
    let before = rule.to_string();
    let out = Rename.visit(rule).unwrap();
    assert_eq!(out.to_string(), "p(V_X).");
    assert_eq!(
        out.span(Attribute::Location).unwrap(),
        rule.span(Attribute::Location).unwrap()
    );
    assert_eq!(rule.to_string(), before);
    assert_eq!(rule.to_string(), "p(X).");
}

#[test]
fn a_visit_may_return_a_node_of_another_kind() {
    // Oracle: replacing every variable by the term `a` in `p(X,a) :- q(b).`.
    struct Replace;
    impl Visitor for Replace {
        fn visit_variable(&mut self, _node: &Ast) -> Result<Ast> {
            ast::symbolic_term(&P::new().s, Symbol::function("a", &[])?)
        }
    }
    let rule = &parse("p(X,a) :- q(b).")[1];
    assert_eq!(Replace.visit(rule).unwrap().to_string(), "p(a,a) :- q(b).");
}

#[test]
fn array_elements_are_replaced_one_for_one() {
    // Oracle: a `Transformer` turning every `SymbolicTerm` into the variable
    // `Z` prints the pool `(a;1;b)` as `(Z;Z;Z)`.
    struct ToVariable;
    impl Visitor for ToVariable {
        fn visit_symbolic_term(&mut self, node: &Ast) -> Result<Ast> {
            ast::variable(&node.span(Attribute::Location)?, "Z")
        }
    }
    let s = P::new().s;
    let terms: Vec<Ast> = [
        Symbol::function("a", &[]).unwrap(),
        Symbol::number(1),
        Symbol::function("b", &[]).unwrap(),
    ]
    .into_iter()
    .map(|symbol| ast::symbolic_term(&s, symbol).unwrap())
    .collect();
    let pool = ast::pool(&s, &terms).unwrap();
    let out = ToVariable.visit(&pool).unwrap();
    assert_eq!(out.to_string(), "(Z;Z;Z)");
    assert_eq!(out.ast_array_len(Attribute::Arguments).unwrap(), 3);
    assert_eq!(pool.to_string(), "(a;1;b)");
}

#[test]
fn a_visitor_keeps_state_between_visits() {
    struct Count(usize);
    impl Visitor for Count {
        fn visit_variable(&mut self, node: &Ast) -> Result<Ast> {
            self.0 += 1;
            Ok(node.clone())
        }
    }
    let mut count = Count(0);
    let rule = &parse("p(X,Y) :- q(X,Y,Z).")[1];
    let out = count.visit(rule).unwrap();
    assert_eq!(count.0, 5);
    assert!(out.ptr_eq(rule));
}

// ---------------------------------------------------------------------------
// Order of visits
// ---------------------------------------------------------------------------

fn recorded(program: &str, statement: usize) -> Vec<AstType> {
    let mut recorder = Recorder::default();
    recorder.visit(&parse(program)[statement]).unwrap();
    recorder.log
}

#[test]
fn children_are_visited_in_constructor_order_before_the_next_sibling() {
    assert_eq!(
        recorded("p(X,a) :- q(b), not r.", 1),
        types![
            Rule,
            Literal,
            SymbolicAtom,
            Function,
            Variable,
            SymbolicTerm,
            Literal,
            SymbolicAtom,
            Function,
            SymbolicTerm,
            Literal,
            SymbolicAtom,
            Function
        ]
    );
    assert_eq!(
        recorded("a; b :- c.", 1),
        types![
            Rule,
            Disjunction,
            ConditionalLiteral,
            Literal,
            SymbolicAtom,
            Function,
            ConditionalLiteral,
            Literal,
            SymbolicAtom,
            Function,
            Literal,
            SymbolicAtom,
            Function
        ]
    );
    assert_eq!(recorded("#show p/1.", 1), types![ShowSignature]);
    assert_eq!(recorded("#const k=3.", 1), types![Definition, SymbolicTerm]);
    assert_eq!(
        recorded(":~ p(X). [X@1,X]", 1),
        types![
            Minimize,
            Variable,
            SymbolicTerm,
            Variable,
            Literal,
            SymbolicAtom,
            Function,
            Variable
        ]
    );
    assert_eq!(recorded("a.", 0), types![Program]);
}

#[test]
fn an_absent_optional_child_is_skipped_and_a_present_one_is_visited() {
    // Oracle: no `Guard` in the first, exactly one in the second.
    let none = recorded("{ a: b } :- c.", 1);
    assert!(!none.contains(&AstType::Guard));
    assert_eq!(
        none,
        types![
            Rule,
            Aggregate,
            ConditionalLiteral,
            Literal,
            SymbolicAtom,
            Function,
            Literal,
            SymbolicAtom,
            Function,
            Literal,
            SymbolicAtom,
            Function
        ]
    );
    let some = recorded("#count { X : p(X) } > 1 :- q.", 1);
    assert_eq!(
        some.iter().filter(|kind| **kind == AstType::Guard).count(),
        1
    );
    assert_eq!(
        some,
        types![
            Rule,
            HeadAggregate,
            Guard,
            SymbolicTerm,
            HeadAggregateElement,
            Variable,
            ConditionalLiteral,
            Literal,
            SymbolicAtom,
            Function,
            Variable,
            Literal,
            SymbolicAtom,
            Function
        ]
    );
}

#[test]
fn visit_dispatches_each_of_the_46_kinds_to_its_own_method() {
    let kinds = all_kinds();
    assert_eq!(kinds.len(), 46);
    for (kind, node) in &kinds {
        let mut recorder = Recorder::default();
        let out = recorder.visit(node).unwrap();
        assert_eq!(recorder.log.first(), Some(kind), "{kind:?}");
        assert!(out.ptr_eq(node), "{kind:?}");
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[test]
fn the_first_error_stops_the_walk_and_leaves_the_tree_alone() {
    struct Fail {
        seen: usize,
    }
    impl Visitor for Fail {
        fn visit_variable(&mut self, node: &Ast) -> Result<Ast> {
            self.seen += 1;
            if self.seen == 2 {
                return Err(Error::new(ErrorKind::Callback, "second variable"));
            }
            // The first variable is replaced, so the walk has a change to
            // discard when the second one fails.
            let renamed = node.copy()?;
            renamed.set_string(Attribute::Name, "Changed")?;
            Ok(renamed)
        }
    }
    let rule = &parse("p(X,Y,Z) :- q(X).")[1];
    let mut fail = Fail { seen: 0 };
    let err = fail.visit(rule).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
    assert!(err.to_string().contains("second variable"), "{err}");
    assert_eq!(fail.seen, 2, "no later variable is visited");
    assert_eq!(rule.to_string(), "p(X,Y,Z) :- q(X).");
}

#[test]
fn a_setter_error_inside_a_visit_passes_through() {
    struct Bad;
    impl Visitor for Bad {
        fn visit_variable(&mut self, node: &Ast) -> Result<Ast> {
            node.copy()?.set_number(Attribute::Sign, 1)?; // a variable has no sign
            Ok(node.clone())
        }
    }
    let rule = &parse("p(X).")[1];
    assert_eq!(Bad.visit(rule).unwrap_err().kind(), ErrorKind::InvalidInput);
}

// ---------------------------------------------------------------------------
// Editing arrays from a visitor: walk, copy if unchanged, then edit
// ---------------------------------------------------------------------------

/// Deletes the first body literal of every rule.
struct DropFirstBodyLiteral;
impl Visitor for DropFirstBodyLiteral {
    fn visit_rule(&mut self, node: &Ast) -> Result<Ast> {
        let walked = walk(self, node)?;
        // Editing an unchanged result would edit the original tree.
        let edited = if walked.ptr_eq(node) {
            walked.copy()?
        } else {
            walked
        };
        edited.delete_ast_at(Attribute::Body, 0)?;
        Ok(edited)
    }
}

#[test]
fn deleting_a_body_literal_after_walk_leaves_the_original_alone() {
    // Changed children: `walk` already made a fresh node, which may be
    // edited directly.
    struct RenameAndDrop;
    impl Visitor for RenameAndDrop {
        fn visit_variable(&mut self, node: &Ast) -> Result<Ast> {
            Rename.visit_variable(node)
        }
        fn visit_rule(&mut self, node: &Ast) -> Result<Ast> {
            let walked = walk(self, node)?;
            assert!(!walked.ptr_eq(node));
            walked.delete_ast_at(Attribute::Body, 0)?;
            Ok(walked)
        }
    }
    // Unchanged children: `walk` returns the node itself, so the visitor
    // copies before it deletes.
    let unchanged = &parse("a :- b, c.")[1];
    let out = DropFirstBodyLiteral.visit(unchanged).unwrap();
    assert_eq!(out.to_string(), "a :- c.");
    assert_eq!(unchanged.to_string(), "a :- b; c.");
    assert!(!out.ptr_eq(unchanged));

    let changed = &parse("p(X) :- q(X), r.")[1];
    let out = RenameAndDrop.visit(changed).unwrap();
    assert_eq!(out.to_string(), "p(V_X) :- r.");
    assert_eq!(changed.to_string(), "p(X) :- q(X); r.");
}

#[test]
fn editing_the_unchanged_result_of_walk_edits_the_original() {
    // The reason for the copy above: `walk` returns the input itself when
    // nothing changed, so an edit to it is an edit to the caller's tree.
    let node = &parse("a :- b, c.")[1];
    let walked = walk(&mut Noop, node).unwrap();
    assert!(walked.ptr_eq(node));
    walked.delete_ast_at(Attribute::Body, 0).unwrap();
    assert_eq!(node.to_string(), "a :- c.");
}

// ---------------------------------------------------------------------------
// Trait shape
// ---------------------------------------------------------------------------

#[test]
fn a_visitor_can_be_used_as_a_trait_object() {
    fn run(visitor: &mut dyn Visitor, node: &Ast) -> Ast {
        visitor.visit(node).unwrap()
    }
    let rule = &parse("p(X).")[1];
    assert_eq!(run(&mut Rename, rule).to_string(), "p(V_X).");
    assert!(run(&mut Noop, rule).ptr_eq(rule));
    let boxed: Vec<Box<dyn Visitor>> = vec![Box::new(Noop), Box::new(Rename)];
    assert_eq!(boxed.len(), 2);
}

#[test]
fn an_override_can_call_walk_before_or_after_its_own_edit() {
    // Before: the visitor sees the rule with rewritten children. After: it
    // sees the original children. Both end at the same text here.
    struct Around {
        head_before_walk: Vec<String>,
    }
    impl Visitor for Around {
        fn visit_rule(&mut self, node: &Ast) -> Result<Ast> {
            self.head_before_walk.push(node.to_string());
            let walked = walk(self, node)?;
            self.head_before_walk.push(walked.to_string());
            Ok(walked)
        }
        fn visit_variable(&mut self, node: &Ast) -> Result<Ast> {
            Rename.visit_variable(node)
        }
    }
    let rule = &parse("p(X).")[1];
    let mut around = Around {
        head_before_walk: Vec::new(),
    };
    around.visit(rule).unwrap();
    assert_eq!(around.head_before_walk, ["p(X).", "p(V_X)."]);
}

// ---------------------------------------------------------------------------
// Depth
// ---------------------------------------------------------------------------

/// Stack for the deep-recursion threads. 512 MiB on 64-bit targets. A 32-bit
/// process has 2 to 4 GiB of address space in all, so 256 MiB there: the walk
/// of an unoptimised build takes about 4 KiB of stack per level on i686 (a
/// 50,000-level chain overflowed 128 MiB and fitted in 200 MiB), well above
/// what clingo itself needs at these depths (8 to 64 MB at depth 100 000 on
/// 64-bit, whose frames are larger).
const DEEP_STACK: usize = if cfg!(target_pointer_width = "64") {
    512 << 20
} else {
    256 << 20
};

/// `walk` recurses like pyclingo's `Transformer`, so it must not be the
/// weakest link: a chain as deep as clingo's own `Display` of it survives on
/// this thread is walked, rebuilt and dropped without overflowing. (Dropping a
/// deep tree is clingo's own recursion, not guarded by clingox.)
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
#[test]
fn walk_survives_a_chain_as_deep_as_clingos_own_display() {
    const DEPTH: usize = 50_000;
    std::thread::Builder::new()
        .stack_size(DEEP_STACK)
        .spawn(|| {
            let s = P::new().s;
            let mut chain = ast::variable(&s, "X").unwrap();
            for _ in 0..DEPTH {
                chain = ast::unary_operation(&s, UnaryOperator::Minus, &chain).unwrap();
            }
            // clingo's own recursion, at this depth, on this stack: the
            // bound the walk must not fall below (oracle: `---X` per level).
            let text = chain.to_string();
            assert_eq!(text.len(), DEPTH + 1);
            assert!(text.ends_with('X'));

            let out = Rename.visit(&chain).unwrap();
            assert!(!out.ptr_eq(&chain));
            let renamed = out.to_string();
            assert_eq!(renamed.len(), DEPTH + 3);
            assert!(renamed.ends_with("V_X"));
            assert_eq!(chain.to_string(), text);

            let untouched = Noop.visit(&chain).unwrap();
            assert!(untouched.ptr_eq(&chain));
            drop(out);
            drop(untouched);
            drop(chain);
        })
        .unwrap()
        .join()
        .unwrap();
}

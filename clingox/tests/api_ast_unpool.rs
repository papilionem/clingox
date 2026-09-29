//! `Ast::unpool` and the `Unpool` flags.
//!
//! Oracle: pyclingo 5.8.2, calling `clingo_ast_unpool` directly through
//! `clingo._internal._lib` (pyclingo has no wrapper that exposes the flags),
//! acquiring each node the callback receives and comparing the pointers.
//! Every expected string, count and pointer identity below was produced by
//! that session; the scenarios ("unpool basic", "unpool on
//! non-statements", "unpool: pool of pools, empty pool", "callback returning
//! error", "unpool: more statement kinds", "unpool `ConditionalLiteral` flags",
//! "empty pool => zero callbacks", "unpool result nodes are fresh but unchanged
//! parts shared"). The flags do not mean what `clingo.h` says: the
//! tests pin what was measured. The tests need a real clingo, so none of them
//! runs under Miri; the reference-count tests are the ones `ASan` and `LSan`
//! watch (`cargo xtask sanitize`).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::too_many_lines,
    reason = "a scenario reads better in one piece"
)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use clingox::ast::{self, Ast, AstType, Attribute, Unpool, Visitor};
use clingox::{Error, ErrorKind, Result};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The statements `text` parses to, including the implicit `#program base.`.
fn statements(text: &str) -> Vec<Ast> {
    let mut all = Vec::new();
    ast::parse_string(text, |statement| {
        all.push(statement);
        Ok(())
    })
    .unwrap();
    all
}

/// The first statement after `#program base.`.
fn first(text: &str) -> Ast {
    statements(text).remove(1)
}

/// Every alternative `node.unpool(what, ..)` gives, kept alive.
fn alternatives(node: &Ast, what: Unpool) -> Vec<Ast> {
    let mut all = Vec::new();
    node.unpool(what, |alternative| {
        all.push(alternative);
        Ok(())
    })
    .unwrap();
    all
}

/// The alternatives as text.
fn texts(node: &Ast, what: Unpool) -> Vec<String> {
    alternatives(node, what)
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// Whether the walk finds a `Pool` node anywhere in the tree.
fn has_pool(node: &Ast) -> bool {
    struct Find(bool);
    impl Visitor for Find {
        fn visit_pool(&mut self, node: &Ast) -> Result<Ast> {
            self.0 = true;
            Ok(node.clone())
        }
    }
    let mut find = Find(false);
    find.visit(node).unwrap();
    find.0
}

// ---------------------------------------------------------------------------
// The flags
// ---------------------------------------------------------------------------

/// The C values the constants stand for (clingo.h:4114-4120).
#[test]
fn the_flags_are_clingos_bits() {
    assert_eq!(clingox_sys::clingo_ast_unpool_type_condition, 1);
    assert_eq!(clingox_sys::clingo_ast_unpool_type_other, 2);
    assert_eq!(clingox_sys::clingo_ast_unpool_type_all, 3);
    // Behaviourally, on the node where the two flags differ: swapped
    // bits would swap these two results.
    let literal = conditional_literal();
    assert_eq!(
        texts(&literal, Unpool::CONDITION),
        ["b(1;2): c(3)", "b(1;2): c(4)"]
    );
    assert_eq!(
        texts(&literal, Unpool::OTHER),
        ["b(1): c(3;4)", "b(2): c(3;4)"]
    );
}

#[test]
fn contains_checks_every_flag_of_the_argument() {
    assert!(Unpool::ALL.contains(Unpool::CONDITION));
    assert!(Unpool::ALL.contains(Unpool::OTHER));
    assert!(Unpool::ALL.contains(Unpool::ALL));
    assert!(Unpool::CONDITION.contains(Unpool::CONDITION));
    assert!(!Unpool::CONDITION.contains(Unpool::OTHER));
    assert!(!Unpool::OTHER.contains(Unpool::CONDITION));
    assert!(!Unpool::CONDITION.contains(Unpool::ALL));
    assert!(!Unpool::OTHER.contains(Unpool::ALL));
}

#[test]
fn flags_combine_with_or() {
    assert_eq!(Unpool::CONDITION | Unpool::OTHER, Unpool::ALL);
    assert_eq!(Unpool::OTHER | Unpool::CONDITION, Unpool::ALL);
    assert_eq!(Unpool::CONDITION | Unpool::CONDITION, Unpool::CONDITION);
    assert_eq!(Unpool::ALL | Unpool::OTHER, Unpool::ALL);

    let mut flags = Unpool::CONDITION;
    flags |= Unpool::OTHER;
    assert_eq!(flags, Unpool::ALL);
    flags |= Unpool::OTHER;
    assert_eq!(flags, Unpool::ALL, "or-ing a set flag changes nothing");

    assert_ne!(Unpool::CONDITION, Unpool::OTHER);
    assert_ne!(Unpool::CONDITION, Unpool::ALL);
}

#[test]
fn flags_are_copy_hash_and_eq() {
    use std::collections::HashSet;
    let flags = Unpool::OTHER;
    let copy = flags;
    assert_eq!(flags, copy);
    let set: HashSet<Unpool> = [Unpool::CONDITION, Unpool::OTHER, Unpool::ALL, Unpool::ALL]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 3);
    assert!(set.contains(&(Unpool::CONDITION | Unpool::OTHER)));
}

/// `Debug` names the set flags in the order `CONDITION`, `OTHER`; `ALL` has no
/// name of its own.
#[test]
fn debug_names_the_set_flags() {
    assert_eq!(format!("{:?}", Unpool::CONDITION), "Unpool(CONDITION)");
    assert_eq!(format!("{:?}", Unpool::OTHER), "Unpool(OTHER)");
    assert_eq!(
        format!("{:?}", Unpool::CONDITION | Unpool::OTHER),
        "Unpool(CONDITION | OTHER)"
    );
    assert_eq!(format!("{:?}", Unpool::ALL), "Unpool(CONDITION | OTHER)");
    assert_eq!(
        format!("{:?}", Unpool::OTHER | Unpool::CONDITION),
        "Unpool(CONDITION | OTHER)",
        "the order does not depend on how the flags were combined"
    );
}

// ---------------------------------------------------------------------------
// Rules with pools
// ---------------------------------------------------------------------------

/// A pool in the head and one in the body give the cross product, head varying
/// fastest.
///
/// Oracle: "unpool basic": `p(1;2) :- q(3;4).` with all flags gives
/// `p(1) :- q(3).`, `p(2) :- q(3).`, `p(1) :- q(4).`, `p(2) :- q(4).`.
#[test]
fn a_rule_with_two_pools_gives_four_alternatives_in_order() {
    let rule = first("p(1;2) :- q(3;4).");
    assert_eq!(
        texts(&rule, Unpool::ALL),
        [
            "p(1) :- q(3).",
            "p(2) :- q(3).",
            "p(1) :- q(4).",
            "p(2) :- q(4).",
        ]
    );
}

/// On a rule, `OTHER` alone acts like `ALL` and `CONDITION` alone does
/// nothing: it calls back once, with the rule itself.
///
/// Oracle: flags 2 give the same four strings as flags 3; flags 1 give
/// `p(1;2) :- q(3;4).` with the same pointer as the input.
#[test]
fn on_a_rule_other_acts_like_all_and_condition_changes_nothing() {
    let rule = first("p(1;2) :- q(3;4).");
    assert_eq!(texts(&rule, Unpool::OTHER), texts(&rule, Unpool::ALL));

    let same = alternatives(&rule, Unpool::CONDITION);
    assert_eq!(same.len(), 1);
    assert!(same[0].ptr_eq(&rule), "the callback got the input itself");
    assert_eq!(same[0].to_string(), "p(1;2) :- q(3;4).");
}

/// The alternatives of a rule with pools in a condition: the pool in the
/// condition of a body conditional literal is unpooled by `ALL` as well.
///
/// Oracle: "unpool basic": `a :- b : c(1;2); d(5;6).` with flags 3 gives two
/// rules, `a :- b: c(1); b: c(2); d(5).` and `a :- b: c(1); b: c(2); d(6).`.
#[test]
fn a_pool_in_a_condition_becomes_two_conditional_literals() {
    let rule = first("a :- b : c(1;2); d(5;6).");
    assert_eq!(
        texts(&rule, Unpool::ALL),
        [
            "a :- b: c(1); b: c(2); d(5).",
            "a :- b: c(1); b: c(2); d(6).",
        ]
    );
    // Condition-only on the rule: nothing.
    let same = alternatives(&rule, Unpool::CONDITION);
    assert_eq!(same.len(), 1);
    assert!(same[0].ptr_eq(&rule));
}

/// Pools in the head, in a literal and in its condition: eight alternatives.
///
/// Oracle: "unpool basic": `a(1;2) :- b(3;4) : c(1;2).` gives eight
/// rules in the order below.
#[test]
fn head_literal_and_condition_pools_give_eight_alternatives() {
    let rule = first("a(1;2) :- b(3;4) : c(1;2).");
    assert_eq!(
        texts(&rule, Unpool::ALL),
        [
            "a(1) :- b(3): c(1); b(3): c(2).",
            "a(2) :- b(3): c(1); b(3): c(2).",
            "a(1) :- b(4): c(1); b(3): c(2).",
            "a(2) :- b(4): c(1); b(3): c(2).",
            "a(1) :- b(3): c(1); b(4): c(2).",
            "a(2) :- b(3): c(1); b(4): c(2).",
            "a(1) :- b(4): c(1); b(4): c(2).",
            "a(2) :- b(4): c(1); b(4): c(2).",
        ]
    );
}

// ---------------------------------------------------------------------------
// A conditional literal, where the flags differ
// ---------------------------------------------------------------------------

/// `b(1;2) : c(3;4)`, the body literal of `a :- b(1;2) : c(3;4).`.
fn conditional_literal() -> Ast {
    let rule = first("a :- b(1;2) : c(3;4).");
    let literal = rule.ast_at(Attribute::Body, 0).unwrap();
    assert_eq!(literal.ast_type(), AstType::ConditionalLiteral);
    literal
}

/// The one node type on which the flags mean what the header says.
///
/// Oracle: "unpool `ConditionalLiteral` flags", on the conditional literal
/// itself: flags 1 unpool the condition only (`b(1;2): c(3)`,
/// `b(1;2): c(4)`), flags 2 the literal only (`b(1): c(3;4)`,
/// `b(2): c(3;4)`), flags 3 both (four, literal varying slowest).
#[test]
fn a_conditional_literal_unpools_by_flag() {
    let literal = conditional_literal();
    assert_eq!(
        texts(&literal, Unpool::CONDITION),
        ["b(1;2): c(3)", "b(1;2): c(4)"]
    );
    assert_eq!(
        texts(&literal, Unpool::OTHER),
        ["b(1): c(3;4)", "b(2): c(3;4)"]
    );
    assert_eq!(
        texts(&literal, Unpool::ALL),
        ["b(1): c(3)", "b(1): c(4)", "b(2): c(3)", "b(2): c(4)"]
    );
}

/// On the enclosing rule, the same three flags do not split the way they do on
/// the literal: `CONDITION` does nothing, `OTHER` and `ALL` unpool everything.
///
/// Oracle: "unpool `ConditionalLiteral` flags": for `a :- b(1;2) : c(3;4).`
/// flags 1 give the rule itself, flags 2 and 3 give the same four rules; and
/// for `a(1;2) :- b(1;2) : c(3;4).` flags 1 give the rule, flags 2 and 3 give
/// the same eight.
#[test]
fn on_the_enclosing_rule_the_condition_flag_does_nothing() {
    let rule = first("a :- b(1;2) : c(3;4).");
    let expected = [
        "a :- b(1): c(3); b(1): c(4).",
        "a :- b(2): c(3); b(1): c(4).",
        "a :- b(1): c(3); b(2): c(4).",
        "a :- b(2): c(3); b(2): c(4).",
    ];
    assert_eq!(texts(&rule, Unpool::ALL), expected);
    assert_eq!(texts(&rule, Unpool::OTHER), expected);
    let only_condition = alternatives(&rule, Unpool::CONDITION);
    assert_eq!(only_condition.len(), 1);
    assert!(only_condition[0].ptr_eq(&rule));
    assert_eq!(only_condition[0].to_string(), "a :- b(1;2): c(3;4).");

    let both = first("a(1;2) :- b(1;2) : c(3;4).");
    assert_eq!(texts(&both, Unpool::ALL).len(), 8);
    assert_eq!(texts(&both, Unpool::OTHER), texts(&both, Unpool::ALL));
    let only_condition = alternatives(&both, Unpool::CONDITION);
    assert_eq!(only_condition.len(), 1);
    assert!(only_condition[0].ptr_eq(&both));
}

// ---------------------------------------------------------------------------
// Nothing to unpool, empty and singleton pools
// ---------------------------------------------------------------------------

/// A node without a pool calls back exactly once, with the input itself.
///
/// Oracle: `a.`, `#show p/2.` and `&t { 1 ; 2 }.` under flags 3 each give
/// one call whose node has the input's pointer.
#[test]
fn a_node_without_a_pool_calls_back_once_with_itself() {
    for source in ["a.", "#show p/2.", "&t { 1 ; 2 }."] {
        let node = first(source);
        for what in [Unpool::CONDITION, Unpool::OTHER, Unpool::ALL] {
            let all = alternatives(&node, what);
            assert_eq!(all.len(), 1, "{source} under {what:?}");
            assert!(all[0].ptr_eq(&node), "{source} under {what:?}");
        }
    }
    assert_eq!(
        texts(&first("&t { 1 ; 2 }."), Unpool::ALL),
        ["&t { 1; 2 }."]
    );
}

/// The program node and the other top-level nodes are unpooled like any
/// other.
#[test]
fn the_implicit_program_node_calls_back_once_with_itself() {
    let program = statements("a.").remove(0);
    assert_eq!(program.ast_type(), AstType::Program);
    let all = alternatives(&program, Unpool::ALL);
    assert_eq!(all.len(), 1);
    assert!(all[0].ptr_eq(&program));
}

/// An empty pool has no alternative: zero calls, and success. The rule, its
/// head literal and the pool itself all give zero.
///
/// Oracle: "empty pool => zero callbacks": `p(1;2) :- q.` with the pool's
/// `arguments` set to `[]` prints as `(1/0)() :- q.`, and rule, literal and
/// pool each unpool to zero nodes with success.
#[test]
fn an_empty_pool_gives_zero_calls() {
    let rule = first("p(1;2) :- q.");
    let literal = rule.ast(Attribute::Head).unwrap();
    let atom = literal.ast(Attribute::Atom).unwrap();
    let pool = atom.ast(Attribute::Symbol).unwrap();
    assert_eq!(pool.ast_type(), AstType::Pool);
    assert_eq!(pool.ast_array_len(Attribute::Arguments).unwrap(), 2);
    pool.set_ast_array(Attribute::Arguments, &[]).unwrap();
    assert_eq!(rule.to_string(), "(1/0)() :- q.");

    for (name, node) in [("rule", &rule), ("literal", &literal), ("pool", &pool)] {
        let mut calls = 0;
        node.unpool(Unpool::ALL, |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 0, "{name}");
    }
    // The flags do not change it.
    let mut calls = 0;
    rule.unpool(Unpool::OTHER, |_| {
        calls += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 0);
}

/// A pool with one alternative is one result.
///
/// Oracle: "empty pool => zero callbacks": `p(1;2) :- q.` with the pool's
/// arguments cut to the first gives one rule, `p(1) :- q.`.
#[test]
fn a_singleton_pool_gives_one_alternative() {
    let rule = first("p(1;2) :- q.");
    let pool = rule
        .ast(Attribute::Head)
        .unwrap()
        .ast(Attribute::Atom)
        .unwrap()
        .ast(Attribute::Symbol)
        .unwrap();
    let only = pool.ast_at(Attribute::Arguments, 0).unwrap();
    pool.set_ast_array(Attribute::Arguments, &[only]).unwrap();
    assert_eq!(texts(&rule, Unpool::ALL), ["p(1) :- q."]);
}

// ---------------------------------------------------------------------------
// Nested pools and other statement kinds
// ---------------------------------------------------------------------------

/// A pool inside a pool is flattened.
///
/// Oracle: "unpool: pool of pools, empty pool": `a((1;2);(3;4)).` gives
/// `a(1).`, `a(2).`, `a(3).`, `a(4).`; `a(();(1,2)).` gives `a(()).`,
/// `a((1,2)).`.
#[test]
fn nested_pools_are_flattened() {
    assert_eq!(
        texts(&first("a((1;2);(3;4))."), Unpool::ALL),
        ["a(1).", "a(2).", "a(3).", "a(4)."]
    );
    assert_eq!(
        texts(&first("a(();(1,2))."), Unpool::ALL),
        ["a(()).", "a((1,2))."]
    );
}

/// The statement kinds other than rules.
///
/// Oracle: "unpool: pool of pools, empty pool" and "unpool: more statement
/// kinds", flags 3 on each of the sources below.
#[test]
fn other_statement_kinds_unpool_as_measured() {
    let cases: &[(&str, &[&str])] = &[
        (
            "#show p(1;2) : q(3;4).",
            &[
                "#show p(1) : q(3).",
                "#show p(2) : q(3).",
                "#show p(1) : q(4).",
                "#show p(2) : q(4).",
            ],
        ),
        (
            "#minimize {1,(1;2) : a(1;2)}.",
            &[
                ":~ a(1). [1@0,1]",
                ":~ a(1). [1@0,2]",
                ":~ a(2). [1@0,1]",
                ":~ a(2). [1@0,2]",
            ],
        ),
        (
            "#external p(1;2).",
            &["#external p(1). [false]", "#external p(2). [false]"],
        ),
        ("#project p(1;2).", &["#project p(1).", "#project p(2)."]),
        (
            "1 { a(1;2) } 1 :- b(3;4).",
            &[
                "1 <= { a(1); a(2) } <= 1 :- b(3).",
                "1 <= { a(1); a(2) } <= 1 :- b(4).",
            ],
        ),
        (
            ":- #count{ 1;2 : q(3;4)} > 1.",
            &["#false :- 1 < #count { 1; 2: q(3); 2: q(4) }."],
        ),
        (
            "a :- 1 < (1;2) < 3.",
            &["a :- 1 < 1 < 3.", "a :- 1 < 2 < 3."],
        ),
        (
            "a :- b, not c(1;2).",
            &["a :- b; not c(1).", "a :- b; not c(2)."],
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(texts(&first(source), Unpool::ALL), *expected, "{source}");
    }
}

/// A theory atom's `1 ; 2` is a sequence, not a pool.
///
/// Oracle: `&t { 1 ; 2 }.` calls back once with the input.
#[test]
fn a_theory_atom_is_not_a_pool() {
    let node = first("&t { 1 ; 2 }.");
    let all = alternatives(&node, Unpool::ALL);
    assert_eq!(all.len(), 1);
    assert!(all[0].ptr_eq(&node));
}

/// Nodes that are not statements unpool too; no kind is checked.
///
/// Oracle: "unpool on non-statements": for `p(1;2) :- q(X;Y,Z).`, the head
/// literal and its atom give `p(1)`, `p(2)`; the pool node itself gives
/// `p(1)`, `p(2)`; the pool's first argument gives `p(1)`; the body literal
/// gives `q(X)`, `q(Y,Z)`.
#[test]
fn nodes_that_are_not_statements_unpool_too() {
    let rule = first("p(1;2) :- q(X;Y,Z).");
    let head = rule.ast(Attribute::Head).unwrap();
    let atom = head.ast(Attribute::Atom).unwrap();
    let pool = atom.ast(Attribute::Symbol).unwrap();
    let argument = pool.ast_at(Attribute::Arguments, 0).unwrap();
    let body = rule.ast_at(Attribute::Body, 0).unwrap();
    assert_eq!(head.ast_type(), AstType::Literal);
    assert_eq!(atom.ast_type(), AstType::SymbolicAtom);
    assert_eq!(pool.ast_type(), AstType::Pool);
    assert_eq!(texts(&head, Unpool::ALL), ["p(1)", "p(2)"]);
    assert_eq!(texts(&atom, Unpool::ALL), ["p(1)", "p(2)"]);
    assert_eq!(texts(&pool, Unpool::ALL), ["p(1)", "p(2)"]);
    assert_eq!(texts(&argument, Unpool::ALL), ["p(1)"]);
    assert_eq!(texts(&body, Unpool::ALL), ["q(X)", "q(Y,Z)"]);
}

// ---------------------------------------------------------------------------
// What the results look like
// ---------------------------------------------------------------------------

/// The input is not modified.
///
/// Oracle: "unpool result nodes are fresh...": `str(n)` before and after
/// are equal.
#[test]
fn the_input_is_not_changed() {
    for source in ["p(1;2).", "p(1;2) :- q(3;4).", "a :- b(1;2) : c(3;4)."] {
        let node = first(source);
        let before = node.to_string();
        let _ = alternatives(&node, Unpool::ALL);
        let _ = alternatives(&node, Unpool::CONDITION);
        assert_eq!(node.to_string(), before, "{source}");
    }
}

/// No result contains a `Pool` node.
///
/// Oracle: "unpool result nodes are fresh...": `has_pool` is `False` for
/// both results of `p(1;2).`.
#[test]
fn the_results_contain_no_pool() {
    for source in [
        "p(1;2).",
        "p(1;2) :- q(3;4).",
        "a((1;2);(3;4)).",
        "#show p(1;2) : q(3;4).",
        "1 { a(1;2) } 1 :- b(3;4).",
    ] {
        let node = first(source);
        assert!(has_pool(&node), "{source} starts with a pool");
        for alternative in alternatives(&node, Unpool::ALL) {
            assert!(!has_pool(&alternative), "{source}: {alternative}");
        }
    }
}

/// The alternatives share the input's untouched children, and keep the
/// location.
///
/// Oracle: "unpool result aliasing": for `p(1;2) :- q(r).`, `body[0]` of
/// both results has the same pointer as the input's `body[0]`, and the
/// location is kept.
#[test]
fn the_results_share_the_untouched_children_and_keep_the_location() {
    let rule = first("p(1;2) :- q(r).");
    let all = alternatives(&rule, Unpool::ALL);
    assert_eq!(all.len(), 2);
    let body = rule.ast_at(Attribute::Body, 0).unwrap();
    let location = rule.span(Attribute::Location).unwrap();
    for alternative in &all {
        assert!(!alternative.ptr_eq(&rule));
        let shared = alternative.ast_at(Attribute::Body, 0).unwrap();
        assert!(shared.ptr_eq(&body));
        assert_eq!(alternative.span(Attribute::Location).unwrap(), location);
    }
    assert!(!all[0].ptr_eq(&all[1]));
    assert_eq!(all[0].to_string(), "p(1) :- q(r).");
    assert_eq!(all[1].to_string(), "p(2) :- q(r).");
}

/// Because children are shared, editing an alternative edits the input: the
/// documented reason to `copy` first.
#[test]
fn editing_a_shared_child_of_an_alternative_edits_the_input() {
    let rule = first("p(1;2) :- q(r).");
    let all = alternatives(&rule, Unpool::ALL);
    let body = all[0].ast_at(Attribute::Body, 0).unwrap();
    let replacement = first("z.").ast(Attribute::Head).unwrap();
    // The body literal is shared; replacing its atom shows in the input.
    body.set_ast(Attribute::Atom, &replacement.ast(Attribute::Atom).unwrap())
        .unwrap();
    assert_eq!(rule.to_string(), "p(1;2) :- z.");
    assert_eq!(all[1].to_string(), "p(2) :- z.");
}

// ---------------------------------------------------------------------------
// The callback
// ---------------------------------------------------------------------------

/// The callback is called once per alternative, in order, and may borrow local
/// state.
#[test]
fn the_callback_borrows_local_state() {
    let rule = first("p(1;2;3).");
    let mut seen = Vec::new();
    let mut calls = 0;
    rule.unpool(Unpool::ALL, |alternative| {
        calls += 1;
        seen.push(alternative.to_string());
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 3);
    assert_eq!(seen, ["p(1).", "p(2).", "p(3)."]);
}

/// A callback error stops the iteration and comes back unchanged.
///
/// Oracle: "callback returning error": `p(1;2;3).`, a callback failing on
/// its second call is called twice and never a third time; `clingo_ast_unpool`
/// returns false with the callback's own error ("custom stop", code 1).
#[test]
fn a_callback_error_stops_the_iteration_and_is_returned_unchanged() {
    let rule = first("p(1;2;3).");
    let mut calls = 0;
    let err = rule
        .unpool(Unpool::ALL, |_| {
            calls += 1;
            if calls == 2 {
                return Err(Error::new(ErrorKind::InvalidInput, "custom stop"));
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(calls, 2, "never called a third time");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(err.to_string().contains("custom stop"), "{err}");

    // The first call failing stops at once.
    let mut calls = 0;
    let err = rule
        .unpool(Unpool::ALL, |_| {
            calls += 1;
            Err(Error::new(ErrorKind::Parse, "first"))
        })
        .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(err.kind(), ErrorKind::Parse, "the kind is not remapped");

    // A failure on the last call is still a failure.
    let mut calls = 0;
    let err = rule
        .unpool(Unpool::ALL, |_| {
            calls += 1;
            if calls == 3 {
                return Err(Error::new(ErrorKind::Utf8, "last"));
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(calls, 3);
    assert_eq!(err.kind(), ErrorKind::Utf8);
}

/// A callback error on the `ptr_eq` path (nothing to unpool) is returned too.
#[test]
fn a_callback_error_on_a_node_without_pools_is_returned() {
    let fact = first("a.");
    let err = fact
        .unpool(Unpool::ALL, |_| {
            Err(Error::new(ErrorKind::InvalidInput, "stop"))
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

/// A panic in the callback resumes on the caller with its payload, the
/// iteration stops, and the node and later calls still work.
#[test]
fn a_panic_in_the_callback_resumes_on_the_caller() {
    let rule = first("p(1;2;3).");
    let mut calls = 0;
    let caught = catch_unwind(AssertUnwindSafe(|| {
        rule.unpool(Unpool::ALL, |_| {
            calls += 1;
            if calls == 2 {
                std::panic::panic_any("stop here");
            }
            Ok(())
        })
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"stop here"));
    assert_eq!(calls, 2, "no callback after the panic");
    assert_eq!(rule.to_string(), "p(1;2;3).");
    assert_eq!(texts(&rule, Unpool::ALL), ["p(1).", "p(2).", "p(3)."]);
}

/// The panic wins over a later error: a callback that panics is not turned
/// into an `Err`.
#[test]
fn a_panic_in_the_callback_on_a_node_without_pools_resumes_too() {
    let fact = first("a.");
    let caught = catch_unwind(AssertUnwindSafe(|| {
        fact.unpool(Unpool::ALL, |_| panic!("no pools, still a callback"))
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"no pools, still a callback")
    );
}

// ---------------------------------------------------------------------------
// Reference counts (the tests `ASan` and `LSan` are watching)
// ---------------------------------------------------------------------------

/// Every alternative kept in a `Vec` past the call stays valid: printed and
/// compared after `unpool` has returned, and again after the input is gone.
///
/// A missing `clingo_ast_acquire` in the trampoline frees nodes clingo still
/// owns; `ASan` sees the read after the free.
#[test]
fn alternatives_kept_past_the_call_stay_valid() {
    let kept = {
        let rule = first("p(1;2) :- q(3;4).");
        let kept = alternatives(&rule, Unpool::ALL);
        assert_eq!(kept.len(), 4);
        kept
    };
    // The input has been dropped; the alternatives hold the shared children.
    let printed: Vec<String> = kept.iter().map(ToString::to_string).collect();
    assert_eq!(
        printed,
        [
            "p(1) :- q(3).",
            "p(2) :- q(3).",
            "p(1) :- q(4).",
            "p(2) :- q(4).",
        ]
    );
    for alternative in &kept {
        assert_eq!(alternative.ast_type(), AstType::Rule);
        assert_eq!(alternative.ast_array_len(Attribute::Body).unwrap(), 1);
    }
    let copies: Vec<Ast> = kept.iter().map(|a| a.deep_copy().unwrap()).collect();
    assert_eq!(
        copies.iter().map(ToString::to_string).collect::<Vec<_>>(),
        printed
    );
}

/// Dropping every node inside the callback leaves the input intact. In the
/// `ptr_eq` case the callback's node *is* the caller's: a missing acquire
/// would release the caller's own reference.
#[test]
fn dropping_every_node_inside_the_callback_leaves_the_input_intact() {
    for source in [
        "p(1;2) :- q(3;4).",
        "a.",
        "#show p/2.",
        "a :- b(1;2) : c(3;4).",
    ] {
        let node = first(source);
        let before = node.to_string();
        let mut calls = 0;
        node.unpool(Unpool::ALL, |alternative| {
            calls += 1;
            drop(alternative);
            Ok(())
        })
        .unwrap();
        assert!(calls >= 1, "{source}");
        assert_eq!(node.to_string(), before, "{source}");
        // Use it again: a use after a free shows up here.
        assert_eq!(texts(&node, Unpool::ALL).len(), calls, "{source}");
        drop(node);
    }
}

/// The `ptr_eq` case on its own, dropping the callback's node several times
/// over: each call must give its own reference.
#[test]
fn dropping_the_input_itself_from_the_callback_repeatedly_is_balanced() {
    let node = first("a.");
    for _ in 0..50 {
        node.unpool(Unpool::ALL, |alternative| {
            assert!(alternative.ptr_eq(&node));
            drop(alternative);
            Ok(())
        })
        .unwrap();
    }
    assert_eq!(node.to_string(), "a.");
}

/// A node kept from the callback, and the input dropped first, then the
/// results: each reference is released once.
#[test]
fn keeping_the_input_itself_past_the_call_and_dropping_the_original_is_balanced() {
    let node = first("a.");
    let kept = alternatives(&node, Unpool::ALL);
    drop(node);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].to_string(), "a.");
}

/// The callback may unpool another node, parse, and use setters, on the node
/// it received and on the input.
///
/// clingo has computed every alternative before the first callback, so
/// nothing the callback does can disturb the iteration.
#[test]
fn the_callback_may_unpool_parse_and_edit() {
    let rule = first("p(1;2) :- q(3;4).");
    let other = first("r(5;6).");
    let mut nested = Vec::new();
    let mut parsed = Vec::new();
    let mut outer = Vec::new();
    rule.unpool(Unpool::ALL, |alternative| {
        // Unpool a different node from inside.
        other.unpool(Unpool::ALL, |inner| {
            nested.push(inner.to_string());
            Ok(())
        })?;
        // Unpool the node the callback received.
        alternative.unpool(Unpool::ALL, |same| {
            assert!(same.ptr_eq(&alternative));
            Ok(())
        })?;
        // Parse from inside.
        ast::parse_string("z.", |statement| {
            parsed.push(statement.to_string());
            Ok(())
        })?;
        // Edit the node it received (a fresh top node, its own head slot).
        let head = alternative.ast(Attribute::Head)?;
        let z = first("z.").ast(Attribute::Head)?;
        alternative.set_ast(Attribute::Head, &z)?;
        outer.push(alternative.to_string());
        drop(head);
        Ok(())
    })
    .unwrap();
    assert_eq!(nested.len(), 8, "two calls of two, times the outer four");
    assert_eq!(nested[..2], ["r(5).", "r(6)."]);
    assert_eq!(parsed.len(), 8, "two statements per parse, four calls");
    assert_eq!(
        outer,
        ["z :- q(3).", "z :- q(3).", "z :- q(4).", "z :- q(4)."]
    );
    // The edits went to the four fresh rule nodes, not to the input.
    assert_eq!(rule.to_string(), "p(1;2) :- q(3;4).");
}

/// Results and the input can be dropped in any order.
#[test]
fn results_and_input_drop_in_any_order() {
    let rule = first("p(1;2) :- q(3;4).");
    let mut kept = alternatives(&rule, Unpool::ALL);
    let last = kept.pop().unwrap();
    drop(kept);
    drop(rule);
    assert_eq!(last.to_string(), "p(2) :- q(4).");
}

// ---------------------------------------------------------------------------
// `Unpool::NONE`
// ---------------------------------------------------------------------------

/// The empty flag set: `Debug` prints `Unpool()`, it equals no other flag, and
/// or-ing it changes nothing.
///
/// Oracle: `clingo_ast_unpool` with flags 0 (`up0.py` in the notes); pyclingo's
/// `unpool(other=False, condition=False)`.
#[test]
fn none_is_the_empty_flag_set() {
    assert_eq!(format!("{:?}", Unpool::NONE), "Unpool()");
    assert_ne!(Unpool::NONE, Unpool::CONDITION);
    assert_ne!(Unpool::NONE, Unpool::OTHER);
    assert_ne!(Unpool::NONE, Unpool::ALL);
    for flag in [Unpool::CONDITION, Unpool::OTHER, Unpool::ALL, Unpool::NONE] {
        assert_eq!(Unpool::NONE | flag, flag);
        assert_eq!(flag | Unpool::NONE, flag);
        assert!(
            flag.contains(Unpool::NONE),
            "every set contains the empty set"
        );
    }
    assert!(!Unpool::NONE.contains(Unpool::CONDITION));
    assert!(!Unpool::NONE.contains(Unpool::OTHER));
    assert!(Unpool::NONE.contains(Unpool::NONE));
    let mut flags = Unpool::NONE;
    flags |= Unpool::OTHER;
    assert_eq!(flags, Unpool::OTHER);
}

/// With no flags, `unpool` hands back the node itself, once, whatever it
/// holds: a pooled rule, a fact, a conditional literal, an empty pool, a pool.
///
/// Oracle: flags 0 gave one callback with the input's pointer in each case,
/// with text `p(1;2) :- q(3;4).`, `a.`, `b(1;2): c(3;4)`, `(1/0)().` and
/// `p(1;2)`.
#[test]
fn unpool_none_returns_the_node_unchanged() {
    let empty = first("p(1;2).");
    let empty_pool = empty
        .ast(Attribute::Head)
        .unwrap()
        .ast(Attribute::Atom)
        .unwrap()
        .ast(Attribute::Symbol)
        .unwrap();
    let pool = first("p(1;2).")
        .ast(Attribute::Head)
        .unwrap()
        .ast(Attribute::Atom)
        .unwrap()
        .ast(Attribute::Symbol)
        .unwrap();
    empty_pool.set_ast_array(Attribute::Arguments, &[]).unwrap();
    let cases: Vec<(Ast, &str)> = vec![
        (first("p(1;2) :- q(3;4)."), "p(1;2) :- q(3;4)."),
        (first("a."), "a."),
        (conditional_literal(), "b(1;2): c(3;4)"),
        (empty, "(1/0)()."),
        (pool, "p(1;2)"),
    ];
    for (node, text) in cases {
        let all = alternatives(&node, Unpool::NONE);
        assert_eq!(all.len(), 1, "{text}");
        assert!(all[0].ptr_eq(&node), "{text}");
        assert_eq!(all[0].to_string(), text);
    }
}

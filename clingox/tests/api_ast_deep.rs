//! A deeply nested term parses and drops without clingox recursing.
//!
//! Parsing itself is not recursive, but destroying a node is: clingo's own
//! release of a nested `AST` recurses once per level (as do its printers,
//! `deep_copy`, `==`, `cmp` and `hash`), so a 2 MiB stack aborts at about
//! 9 000 levels of `f(f(...))` and 20 000 of a unary minus. clingox adds no
//! recursion of its own; the documented advice is to run deep input on a
//! large-stack thread, which is exactly what this test does. It is the
//! shape of `deep_theory_terms.rs`, minus the child process: the work here
//! runs on a 512 MiB thread, far beyond where clingo needs it.
//!
//! Not run on Android and WebAssembly: a 512 MiB stack is not available
//! there (and the default WebAssembly build has no threads at all).

#![cfg(not(any(target_os = "android", target_family = "wasm")))]
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use clingox::ast::{self, Ast, AstType, Attribute};

const DEPTH: usize = 50_000;

/// `p(-(-(...a...)))` with `n` unary minuses. Checked with pyclingo 5.8.2:
/// `parse_string` accepts this at 1e6 levels; only destruction recurses.
fn deep_program(n: usize) -> String {
    format!("p({}a).\n", "-".repeat(n))
}

/// Stack for the deep-recursion threads. 512 MiB on 64-bit targets; a 32-bit
/// process has 2 to 4 GiB of address space in all, so 128 MiB there, which is
/// still more than clingo needs at these depths (8 to 64 MB at depth 100 000
/// on 64-bit, whose frames are larger).
const DEEP_STACK: usize = if cfg!(target_pointer_width = "64") {
    512 << 20
} else {
    128 << 20
};

#[test]
fn a_50_000_level_term_parses_and_drops_on_a_512_mib_thread() {
    std::thread::Builder::new()
        .stack_size(DEEP_STACK)
        .spawn(|| {
            let mut nodes: Vec<Ast> = Vec::new();
            ast::parse_string(&deep_program(DEPTH), |node| {
                nodes.push(node);
                Ok(())
            })
            .unwrap();
            assert_eq!(nodes.len(), 2, "the implicit #program base. and the fact");
            assert_eq!(nodes[1].ast_type(), AstType::Rule);

            // Walk down the nest without recursion of our own: rule head ->
            // literal atom -> symbolic atom's symbol (`p(...)`) -> first
            // argument, then 49 999 more unary operations.
            let head = nodes[1].ast(Attribute::Head).unwrap();
            let atom = head.ast(Attribute::Atom).unwrap();
            let function = atom.ast(Attribute::Symbol).unwrap();
            let mut node = function.ast_at(Attribute::Arguments, 0).unwrap();
            let mut levels = 0;
            while node.ast_type() == AstType::UnaryOperation {
                node = node.ast(Attribute::Argument).unwrap();
                levels += 1;
            }
            assert_eq!(levels, DEPTH);
            assert_eq!(node.ast_type(), AstType::SymbolicTerm, "the constant a");

            // Every handle is dropped here, on the same big stack.
            drop(node);
            drop(function);
            drop(atom);
            drop(head);
            drop(nodes);
        })
        .unwrap()
        .join()
        .unwrap();
}

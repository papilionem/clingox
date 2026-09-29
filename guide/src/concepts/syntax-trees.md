# Syntax trees

`clingox::ast` gives access to the syntax tree gringo builds from a program:
you can read it, build a new tree, change one in place, and rewrite a whole
program before it is grounded. This is what pyclingo's `clingo.ast` does, with
the same node types and attribute names.

## Reading a tree

`ast::parse_string` and `ast::parse_files` call a function once for every
top-level statement. Each one is an `Ast` node. What the node is comes from
`ast_type()`, and what it holds from typed accessors that take an `Attribute`:

```rust
use clingox::ast::{self, AstType, Attribute};

let mut rules = Vec::new();
ast::parse_string("q(X) :- p(X), X > 1.", |node| {
    if node.ast_type() == AstType::Rule {
        rules.push(node);
    }
    Ok(())
})?;

let rule = &rules[0];
assert_eq!(rule.ast(Attribute::Head)?.to_string(), "q(X)");
assert_eq!(rule.ast_array_len(Attribute::Body)?, 2);
assert_eq!(rule.ast_at(Attribute::Body, 1)?.to_string(), "X > 1");
# Ok::<(), clingox::Error>(())
```

The accessors return owned values: a `String`, a `Vec`-like array element or a
new `Ast` handle, never a borrow into the node. Asking for an attribute a node
does not have, or reading it with the wrong accessor, is an
`ErrorKind::InvalidInput` error.

`Eq`, `Ord` and `Hash` compare nodes by structure and ignore their `location`,
so two nodes parsed from the same text at different places are equal.
`Ast::ptr_eq` tells whether two handles are the very same node.

`parse_files` reads standard input for an empty list of files and for the path
`-`, as clingo does. If you edit what a parse callback delivers, edit a copy of
the statement, never the statement itself: an `#edge` directive with several
pairs is delivered as one statement per pair, and clingo deep-copies the shared
body for the later pairs only after the callback returns, so an in-place edit
leaks into the next statement. A file name in a location that is not valid
UTF-8, which only an `#include` of such a file produces, makes `Ast::span` fail
with `ErrorKind::Utf8`.

## Building a tree

There is one function for each node type, named after it: `ast::rule`,
`ast::function`, `ast::variable`, `ast::literal`, and so on, generated from
clingo's own table of node types. The arguments follow clingo's order, and
every node argument is borrowed, so the nodes you pass in stay usable:

```rust
use clingox::Symbol;
use clingox::ast::{self, LiteralSign, Span};

let span = Span::new("<example>", 1, 1, "<example>", 1, 8)?;
let literal = |name: &str| -> clingox::Result<ast::Ast> {
    let term = ast::symbolic_term(&span, Symbol::function(name, &[])?)?;
    let atom = ast::symbolic_atom(&term)?;
    ast::literal(&span, LiteralSign::NoSign, &atom)
};
let head = literal("h")?;
let rule = ast::rule(&span, &head, &[literal("a")?, literal("b")?])?;
assert_eq!(rule.to_string(), "h :- a; b.");
assert_eq!(head.to_string(), "h"); // still usable
# Ok::<(), clingox::Error>(())
```

Numbers that stand for a choice are typed: `LiteralSign`, `BinaryOperator`,
`ComparisonOperator` and six more, so a sign of 7 does not compile. A `Span` is
where a node came from in the source. Its file names are interned, and a line
or column above `u32::MAX` is refused, because clingo would silently cut it.

## Changing a tree

The setters (`set_ast`, `set_string`, `insert_ast_at`, `delete_ast_at`,
`push_ast`, `set_ast_array` and the others) take `&self`. Clones of an `Ast`
share one node, so a change made through one handle is seen through all of them,
and through every parent that holds the node:

```rust
use clingox::ast::{self, Attribute};

let mut nodes = Vec::new();
ast::parse_string("h :- a.", |node| { nodes.push(node); Ok(()) })?;
let rule = nodes[1].clone();
let alias = rule.clone();

let literal = rule.ast_at(Attribute::Body, 0)?;
let atom = literal.ast(Attribute::Atom)?;
let symbol = atom.ast(Attribute::Symbol)?;
symbol.set_string(Attribute::Name, "b")?; // a child, changed through its handle
alias.push_ast(Attribute::Body, &literal)?; // the parent, changed through a clone
assert_eq!(rule.to_string(), "h :- b; b.");
# Ok::<(), clingox::Error>(())
```

To change a tree without touching the original, copy first. `copy()` gives a
new top node with its own arrays that still shares the children, so editing an
array of the copy leaves the original alone, but editing a child changes both.
`deep_copy()` shares nothing.

The setters check before they change anything, and report the first problem as
`ErrorKind::InvalidInput` (or `Nul` for a NUL byte in a string): the attribute
must exist and have the kind the method expects, a number that stands for an
enum or a boolean must be in its range, an index must be in range, and a node
cannot be set as, or below, one of its own descendants. clingo checks none of
the last two, and a cyclic tree crashes the process the next time it is printed
or compared, so clingox refuses it.

Because a node can change through any clone, do not edit a node that is the key
of a `HashMap`, `HashSet` or `BTreeMap`: its hash and its order change with it.

## Rewriting a program

A `Visitor` has one method for each node type, and each defaults to `walk`,
which visits the node's children. Return a clone of the node you were given to
leave it alone, or another node to replace it. Only the changed nodes and their
ancestors are rebuilt; everything else is shared with the original, which is
never modified:

```rust
use clingox::Result;
use clingox::ast::{self, Ast, Attribute, Visitor};

/// Renames the predicate `p` to `q`.
struct Rename;

impl Visitor for Rename {
    fn visit_function(&mut self, node: &Ast) -> Result<Ast> {
        if node.string(Attribute::Name)? != "p" {
            return ast::walk(self, node);
        }
        // Edit a copy: the original stays as it was.
        let renamed = node.copy()?;
        renamed.set_string(Attribute::Name, "q")?;
        Ok(renamed)
    }
}

let mut program = Vec::new();
ast::parse_string("r(X) :- p(X).", |node| { program.push(node); Ok(()) })?;
let rewritten = Rename.visit(&program[1])?;
assert_eq!(rewritten.to_string(), "r(X) :- q(X).");
assert_eq!(program[1].to_string(), "r(X) :- p(X).");
# Ok::<(), clingox::Error>(())
```

`walk` replaces array elements one for one. To delete or insert elements,
call `walk` and edit the result, but `copy()` it first if it is `ptr_eq` to the
input: then nothing below it changed, so it is the original node. The rustdoc of
`Visitor` shows this.

A visitor must not edit the tree it walks, only copies of its nodes: `walk`
reads the children as it goes, so an edit in place gives a silently partial
rewrite (nothing unsafe, since every index is checked, but not what you meant).

`walk` recurses once per level of the tree and has no depth limit of its own,
like pyclingo's `Transformer`. Run it on a thread with a large stack for very
deep trees.

## Adding a tree to a control

`Control::with_program_builder` opens a session in which `ProgramBuilder::add`
takes statements as trees. The session ends when the closure returns, whatever
it returned, and the closure gives back its own result. A typical use is to
rewrite a program and ground the result:

```rust
use clingox::ast::{self, Ast, Attribute, Visitor};
use clingox::{Control, Part, Result};

/// Renames the predicate `p` to `q`.
struct Rename;

impl Visitor for Rename {
    fn visit_function(&mut self, node: &Ast) -> Result<Ast> {
        if node.string(Attribute::Name)? != "p" {
            return ast::walk(self, node);
        }
        let renamed = node.copy()?;
        renamed.set_string(Attribute::Name, "q")?;
        Ok(renamed)
    }
}

let mut ctl = Control::new()?;
ctl.with_program_builder(|builder| {
    ast::parse_string("p(1). r(X) :- p(X).", |statement| {
        builder.add(&Rename.visit(&statement)?)
    })
})?;
ctl.ground(&[Part::base()])?;
let (_, models) = ctl.solve_all()?;
clingox::testing::assert_models!(models, ["q(1) r(1)"]);
# Ok::<(), clingox::Error>(())
```

`add` copies the tree, so editing a node afterwards does not change what was
added, and adding the same node twice adds the statement twice. Statements that
come before any `#program` statement go to `base`, and the current program part
carries over from one session to the next.

The closure borrows the control, so it cannot ground, solve or add text from
inside the session, and the builder cannot leave the closure. What the closure
added before it failed is kept: an error ends the session normally, and a panic
leaves the session for the control's next call to end, so `ground` still sees
the statements.

A tree that is not a statement, or has a child of the wrong kind, is refused
with `ErrorKind::Parse` and no messages, and poisons the control like a syntax
error in `Control::add`. A statement that is well formed but wrong in meaning,
such as a second `#const` of the same name, is accepted: clingo logs it, and
`ground` fails.

## Unpooling

A pool such as `p(1;2)` stands for several statements. `Ast::unpool` calls a
closure once for each statement the pools stand for, in clingo's order:

```rust
use clingox::ast::{self, AstType, Unpool};

let mut rule = None;
ast::parse_string("p(1;2) :- q.", |node| {
    if node.ast_type() == AstType::Rule {
        rule = Some(node);
    }
    Ok(())
})?;
let mut alternatives = Vec::new();
rule.unwrap().unpool(Unpool::ALL, |node| {
    alternatives.push(node.to_string());
    Ok(())
})?;
assert_eq!(alternatives, ["p(1) :- q.", "p(2) :- q."]);
# Ok::<(), clingox::Error>(())
```

With nothing to unpool, the closure is called once with the node itself. `p(())`
holds the empty tuple, not an empty pool, so it is called back once too. An
empty pool cannot be written in program text: it only comes from a tree you
built, and it prints as `(1/0)`; unpooling such a node does not call the
closure at all. The flags follow clingo,
which differs from what its header says: `Unpool::CONDITION` alone does
something only when the node you pass is a conditional literal, and
`Unpool::OTHER` on any other node unpools everything, conditions included.
Alternatives share the parts that did not change with the node you passed, so
`copy` one before you edit it.

## What is not here

A typed view of each node type (the `Rule(Ast)` style wrappers) is not part of
clingox: the constructors, the setters and the visitor cover the same ground.

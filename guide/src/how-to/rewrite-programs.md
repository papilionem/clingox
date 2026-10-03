# Rewrite programs

Sometimes the program you ground is not the program you were given: you want to add a
condition to every rule, drop the `#show` statements, or check which predicates a
program uses before you accept it. clingox does this on clingo's own syntax trees,
the way pyclingo's `clingo.ast` does. This page shows the common tasks.
[Syntax trees](../concepts/syntax-trees.md) explains the model behind them: how nodes
are shared, copied and compared.

## The pipeline

Every rewrite has the same three steps:

1. `ast::parse_string` (or `ast::parse_files`) calls a closure once for each top-level
   statement of the program.
2. A `Visitor` rewrites the statement and returns a new tree. The original is never
   changed.
3. Inside `Control::with_program_builder`, `ProgramBuilder::add` adds the result to
   the control, as `Control::add` adds text.

Then you ground and solve as usual.

## Add a condition to every rule

This rewrite makes every rule of a module depend on an external atom `enabled`, so the
whole module can be switched on and off between solves with `assign_external`. The
visitor overrides `visit_rule`, lets `walk` handle the children, and appends the
literal to the body:

```rust
use clingox::ast::{self, Ast, Attribute, LiteralSign, Span, Visitor};
use clingox::testing::assert_models;
use clingox::{Control, Part, Result, Symbol, TruthValue};

/// Appends `literal` to the body of every rule.
struct AddCondition {
    literal: Ast,
}

impl Visitor for AddCondition {
    fn visit_rule(&mut self, node: &Ast) -> Result<Ast> {
        let walked = ast::walk(self, node)?;
        // When nothing below changed, `walk` returns `node` itself: copy it, so
        // the edit does not reach the original tree.
        let rule = if walked.ptr_eq(node) { walked.copy()? } else { walked };
        rule.push_ast(Attribute::Body, &self.literal)?;
        Ok(rule)
    }
}

let span = Span::synthetic();
let enabled = Symbol::function("enabled", &[])?;
let atom = ast::symbolic_atom(&ast::symbolic_term(&span, enabled)?)?;
let mut add_condition = AddCondition {
    literal: ast::literal(&span, LiteralSign::NoSign, &atom)?,
};

let mut ctl = Control::new()?;
let mut text = Vec::new();
ctl.with_program_builder(|builder| {
    ast::parse_string("bird(tweety). flies(X) :- bird(X).", |statement| {
        let rewritten = add_condition.visit(&statement)?;
        text.push(rewritten.to_string());
        builder.add(&rewritten)
    })
})?;
assert_eq!(
    text,
    ["#program base.", "bird(tweety) :- enabled.", "flies(X) :- bird(X); enabled."],
);

ctl.add_base("#external enabled.")?;
ctl.ground(&[Part::base()])?;
let (_, models) = ctl.solve_all()?;
assert_models!(models, [""]);

ctl.assign_external(enabled, TruthValue::True)?;
let (_, models) = ctl.solve_all()?;
assert_models!(models, ["bird(tweety) flies(tweety) enabled"]);
# Ok::<(), clingox::Error>(())
```

A few things in this example carry over to every rewrite:

- **Copy before you edit.** `walk` rebuilds only the nodes whose children changed, and
  returns the node it was given when nothing changed. Editing that node would edit the
  original tree, so the visitor copies it first (`ptr_eq` tells the two cases apart).
  `copy` copies the top node and its arrays and shares the children, which is enough
  to change the body array.
- **One node can go into many trees.** The same `literal` node is appended to every
  rule. `ProgramBuilder::add` copies each tree it adds, so later edits do not change
  the program.
- **Nodes you build need a location.** `Span::synthetic()` is a placeholder location,
  `<generated>:1:1`, for nodes that do not come from source text. Messages about those
  nodes point there.
- **Printing a tree gives program text.** `to_string()` on a statement gives it in
  clingo's syntax, with `;` between the literals of a body, which clingo also accepts.
  Printing the rewritten statements is a way to debug a rewrite.

## Rename a predicate

To rename every occurrence of a predicate, override `visit_function`, the node type of
`p(X)` and of a constant `p`, and return a renamed copy.
[Syntax trees](../concepts/syntax-trees.md#rewriting-a-program) has the complete
example. A function term inside an argument, such as `f` in `p(f(X))`, is a `Function`
node too, so check the context if a term and a predicate share a name.

## Drop statements

To leave out statements, do not add them. The parse closure sees each statement's
type, so filtering needs no visitor at all. This drops every `#show` statement, so
the models show every atom:

```rust
use clingox::ast::{self, AstType};
use clingox::testing::assert_models;
use clingox::{Control, Part};

let mut ctl = Control::new()?;
ctl.with_program_builder(|builder| {
    ast::parse_string("a. b :- a. #show b/0.", |statement| match statement.ast_type() {
        AstType::ShowSignature | AstType::ShowTerm => Ok(()),
        _ => builder.add(&statement),
    })
})?;
ctl.ground(&[Part::base()])?;
let (_, models) = ctl.solve_all()?;
assert_models!(models, ["a b"]);
# Ok::<(), clingox::Error>(())
```

To delete elements inside a statement, such as one literal of a body, call `walk`,
copy the result if it is the original node, and use `delete_ast_at`; the rustdoc of
[`Visitor`](https://docs.rs/clingox/latest/clingox/ast/trait.Visitor.html) shows it.
To replace one statement by several, as unpooling `p(1;2).` does, add each of them:
see [Unpooling](../concepts/syntax-trees.md#unpooling).

## Inspect a program without grounding it

A visitor can read without changing anything: it returns `node.clone()`, which counts
as unchanged. This one lists the predicates that a program's atoms use, as
`name/arity`:

```rust
use std::collections::BTreeSet;

use clingox::ast::{self, Ast, AstType, Attribute, Visitor};
use clingox::Result;

/// Records the predicate of every atom it visits.
#[derive(Default)]
struct Predicates(BTreeSet<String>);

impl Visitor for Predicates {
    fn visit_symbolic_atom(&mut self, node: &Ast) -> Result<Ast> {
        let term = node.ast(Attribute::Symbol)?;
        if term.ast_type() == AstType::Function {
            let name = term.string(Attribute::Name)?;
            let arity = term.ast_array_len(Attribute::Arguments)?;
            self.0.insert(format!("{name}/{arity}"));
        }
        Ok(node.clone())
    }
}

let mut predicates = Predicates::default();
ast::parse_string(
    "edge(1, 2). reach(X, Y) :- edge(X, Y). :- reach(X, X), not loop_ok.",
    |statement| predicates.visit(&statement).map(drop),
)?;
assert_eq!(
    predicates.0.into_iter().collect::<Vec<_>>(),
    ["edge/2", "loop_ok/0", "reach/2"],
);
# Ok::<(), clingox::Error>(())
```

An atom with classical negation, such as `-p(1)`, holds a unary operation around the
function, and a pool such as `p(1;2)` holds a `Pool`; a complete tool handles those
node types too. `ast_type()` and `has_attribute()` tell what a node is before you read
it, and reading an attribute a node does not have is `ErrorKind::InvalidInput`, not a
panic.

## Parse files

`ast::parse_files` reads files as `Control::load` does, and calls the closure for each
statement. Two things differ from reading the text yourself:

- clingo visits the files **from last to first**, each starting with its own
  `#program base.` statement;
- an `#include` directive is expanded during parsing, so the statements of the
  included file arrive in the stream, with locations in that file, followed by an
  extra `#program base.` statement where the included file ends. This holds for
  `parse_string` too: `a. #include "b.lp".` gives `#program base.`, `a.`, the
  statements of `b.lp`, and `#program base.`.

Statements that follow a `#program` statement belong to that part, and so do the
statements a builder adds after it. A rewrite that keeps the `#program` statements in
the stream, as a visitor does by default, keeps the parts.

## When a rewrite fails

- A syntax error in the input is `ErrorKind::Parse` from `parse_string`, with clingo's
  messages and their positions. No control is involved, so nothing is poisoned.
- An error your visitor or closure returns stops the parse and comes back unchanged.
  Statements added before it stay in the program.
- A tree that is not a statement, or has a child of the wrong kind, makes
  `ProgramBuilder::add` fail with `ErrorKind::Parse`, and poisons the control like a
  syntax error in `Control::add`.
- A setter that would break the tree, such as an index out of range or a node set
  below itself, is refused with `ErrorKind::InvalidInput` before anything changes.

## Related pages

- [Syntax trees](../concepts/syntax-trees.md) covers reading, building and changing
  trees, and unpooling.
- The [`ast`](https://docs.rs/clingox/latest/clingox/ast/index.html) module's rustdoc
  lists every node type and its constructor.
- [Errors and panics](../concepts/errors-and-panics.md) explains poisoning.

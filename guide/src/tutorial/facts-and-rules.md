# Facts and rules

A logic program has two kinds of statements. Rules say how to derive new atoms.
Facts state the data the rules work on. In a clingox program the rules are usually
fixed program text, and the facts come from your Rust data. This chapter builds a
small route finder that way.

## Rules as program text

Rules are added as text, as in the [first program](../getting-started/first-program.md).
These two rules say that a node is reachable from another if an edge leads there,
directly or through other reachable nodes:

```text
reach(X, Y) :- edge(X, Y).
reach(X, Z) :- reach(X, Y), edge(Y, Z).
```

`X`, `Y` and `Z` are variables: names that start with an uppercase letter. `edge` and
`reach` are predicates. Grounding replaces the variables with every value the facts
allow.

## Facts from Rust

Describe the facts as a Rust type, and derive `ToSymbol` for it:

```rust
use clingox::prelude::*;

#[derive(ToSymbol)]
struct Edge(i32, i32);

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base(
        "reach(X, Y) :- edge(X, Y).
         reach(X, Z) :- reach(X, Y), edge(Y, Z).",
    )?;
    ctl.add_facts([Edge(1, 2), Edge(2, 3)])?;
    ctl.ground(&[Part::base()])?;

    let Outcome::Sat(model, _) = ctl.solve_first()? else {
        panic!("the program has an answer set");
    };
    println!("{model}");
#   assert_eq!(
#       model.to_string(),
#       "Answer 1: edge(1,2) edge(2,3) reach(1,2) reach(1,3) reach(2,3)"
#   );
    Ok(())
}
```

```console
$ cargo run
Answer 1: edge(1,2) edge(2,3) reach(1,2) reach(1,3) reach(2,3)
```

`Edge(1, 2)` becomes the fact `edge(1,2)`. The derive names the predicate after the
type, in snake case, and takes the fields in declaration order. A struct with named
fields works the same way, and a struct without fields is a constant:

| Rust | Fact |
|---|---|
| `struct Edge(i32, i32)` with `Edge(1, 2)` | `edge(1,2)` |
| `struct TaskAssignment { worker: i32, task: i32 }` | `task_assignment(1,2)` |
| `struct Done;` | `done` |

To choose the name yourself, write `#[clingo(name = "link")]` above the struct.
Fields of your own derived types become nested terms: a field of type `Node` in
`Hop` gives `hop(node(1),node(2))`.

`solve_first` returns an `Outcome`: the first model found, a proof that there is
none, or an undecided search. The [next chapter](typed-results.md) matches on it in
full.

## What `add_facts` does

`add_facts` converts every value first, and adds nothing if one of them fails. It
then writes the facts as program text into a program part of their own, and grounds
that part at once. It never touches `base`, so you can call it again later, even
after solving. One case fails: a fact for an atom that an earlier step already
defined, such as `p(1)` after a solved program with the choice rule `{p(1)}.`. clingo
reports it as a `Logic` error ("redefinition of atom"), which poisons the control
(see [Errors and panics](../concepts/errors-and-panics.md)).

## Strings and constants

clingo has two kinds of text. A string is written in quotes, `"alice"`. A constant is
a bare name, `alice`. They are different values, and a rule that mentions one never
matches the other: `owner(alice, F)` does not match the fact `owner("alice", F)`.

A Rust `String` could be either, so a `String` field must say which it is:

```rust
use clingox::prelude::*;

#[derive(ToSymbol)]
struct Owner {
    #[clingo(constant)]
    user: String,
    #[clingo(string)]
    file: String,
}

fn main() -> clingox::Result<()> {
    let owner = Owner {
        user: "alice".into(),
        file: "notes.txt".into(),
    };
    println!("{}", owner.to_symbol()?);
#   assert_eq!(owner.to_symbol()?.to_string(), r#"owner(alice,"notes.txt")"#);

    let mut ctl = Control::new()?;
    ctl.add_base("admin_file(F) :- owner(alice, F).")?;
    ctl.add_facts([owner])?;
    ctl.ground(&[Part::base()])?;
    let Outcome::Sat(model, _) = ctl.solve_first()? else {
        panic!("the program has an answer set");
    };
    assert!(model.contains(sym!(admin_file("notes.txt"))?));
    Ok(())
}
```

```console
$ cargo run
owner(alice,"notes.txt")
```

Use `#[clingo(constant)]` for names that rules mention literally, such as `alice` in
`owner(alice, F)`, and `#[clingo(string)]` for free text, such as file names that may
contain spaces or capital letters.

A `String` field without either attribute does not compile, so the choice cannot be
forgotten:

```rust,compile_fail
use clingox::ToSymbol;

#[derive(ToSymbol)]
struct Owner {
    user: String,
}
```

```text
error: the `String` field `user` must say what it is in clingo: `#[clingo(string)]` for a string such as "comp13", or `#[clingo(constant)]` for a constant such as comp13; the two never match each other in rules
 --> src/main.rs:5:5
  |
5 |     user: String,
  |     ^^^^
```

For the same reason, `String` and `&str` do not implement `ToSymbol` on their own.

## Single symbols with `sym!`

For one symbol, `sym!` writes it in clingo's own syntax and checks it at compile
time. `{expr}` inserts the value of a Rust expression:

```rust
use clingox::prelude::*;

fn main() -> clingox::Result<()> {
    let start = 1;
    let fact = sym!(start_at({ start }))?;
    let text = sym!(label(-3, "a b", c, (1, 2)))?;
    println!("{fact} {text}");
#   assert_eq!(fact.to_string(), "start_at(1)");
#   assert_eq!(text.to_string(), r#"label(-3,"a b",c,(1,2))"#);

    let mut ctl = Control::new()?;
    ctl.add_facts([fact])?;
#   let (_, models) = ctl.solve_all()?;
#   assert_eq!(models[0].symbols(), [fact]);
    Ok(())
}
```

```console
$ cargo run
start_at(1) label(-3,"a b",c,(1,2))
```

A variable such as `sym!(p(X))`, a missing comma, or a number outside the 32-bit
range of clingo's numbers is a compile error. `add_facts` accepts symbols, derived
values, and references to either.

## Grounding order

Facts must be added before the rules that use them are grounded. clingo grounds each
part once and never grounds it again, so rules grounded earlier do not see facts
added later:

```rust
use clingox::prelude::*;

#[derive(ToSymbol)]
struct P(i32);

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base("q(X) :- p(X).")?;
    ctl.ground(&[Part::base()])?;

    // `base` is already grounded, so its rule never sees `p(1)`.
    ctl.add_facts([P(1)])?;
    let (_, models) = ctl.solve_all()?;
    println!("{}", models[0]);
#   assert_eq!(models[0].to_string(), "Answer 1: p(1)");
    Ok(())
}
```

```console
$ cargo run
Answer 1: p(1)
```

The model has `p(1)` but no `q(1)`. Add the facts first, then ground the parts whose
rules read them, as the route finder above does.

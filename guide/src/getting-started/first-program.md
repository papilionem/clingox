# Your first program

In this chapter you write a program that gives clingo a small logic program, and
prints every answer set clingo finds for it.

## Create the project

Create a binary project and add clingox, as the [previous chapter](installation.md)
shows:

```console
$ cargo new first-program
$ cd first-program
```

```toml
[dependencies]
clingox = { git = "https://github.com/papilionem/clingox" }
```

## The logic program

The program has two rules:

```text
a :- not b.
b :- not a.
```

The first rule says that `a` holds if there is no reason to believe `b`. The second
says the same the other way round. An answer set is a set of atoms that is
consistent with the rules and justified by them. This program has two: one with
`a` and one with `b`.

## Write the program

Replace `src/main.rs` with:

```rust
use clingox::{Control, Part};

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base("a :- not b. b :- not a.")?;
    ctl.ground(&[Part::base()])?;

    let (result, models) = ctl.solve_all()?;
    for model in &models {
        println!("{model}");
    }
    println!("{result}");
#   let lines: Vec<String> = models.iter().map(ToString::to_string).collect();
#   assert_eq!(lines, ["Answer 2: a", "Answer 1: b"]);
#   assert_eq!(result.to_string(), "SATISFIABLE");
    Ok(())
}
```

Run it:

```console
$ cargo run
Answer 2: a
Answer 1: b
SATISFIABLE
```

## What each step does

**`Control::new()`** creates a control object. It holds one logic program and does
everything with it: parsing, grounding and solving. It fails only if clingo runs out
of memory, or if the clingo library linked into the program is not the version
clingox was built for.

**`add_base`** parses program text and adds it to the part named `base`. A program
can be split into named parts with `#program` directives; text without one belongs
to `base`. `add_base(text)` is short for `add("base", &[], text)`. A syntax error
here is returned as an error of kind `Parse`, with clingo's message and the line and
column it points at.

**`ground`** replaces the variables in the rules with the values they can take.
Here there are no variables, so grounding only prepares the rules for the solver.
`Part::base()` names the `base` part. A part is grounded once; the chapter on
[solving step by step](../tutorial/solving-step-by-step.md) shows how later parts
extend a grounded program.

**`solve_all`** searches for every answer set and returns two things: the result of
the search and the models. A model is an answer set.

## What the output means

Each model prints as clingo prints an answer: `Answer`, the number of the model in
the order the search found it, and the atoms the program shows. The last line is
the result of the search. `SATISFIABLE` means at least one answer set exists.
`UNSATISFIABLE` would mean none does. `UNKNOWN` means the search stopped before it
could tell, for example because it was interrupted.

The numbers show that the search found `b` first. `solve_all` sorts the models by
their atoms, so the list starts with `a` whatever order the search took.

## One model at a time

`solve_all` copies every model before returning them. To look at each model while
the search runs, and stop when you have what you need, use `for_each_model`:

```rust
use clingox::prelude::*;

fn main() -> clingox::Result<()> {
    let mut ctl = Control::with_args(["--models=0"])?;
    ctl.add_base("a :- not b. b :- not a.")?;
    ctl.ground(&[Part::base()])?;

    let mut count = 0;
    let result = ctl.for_each_model(&[], |model| {
        println!("{model}");
        count += 1;
        Ok(ControlFlow::Continue(()))
    })?;
    println!("{count} models, {result}");
#   assert_eq!(count, 2);
#   assert!(result.is_exhausted());
    Ok(())
}
```

`--models=0` is a clingo option: find every model. Without it clingo stops after the
first one. The first program did not need it, because `solve_all` lifts the limit
for its own search and restores the control's setting afterwards. The closure returns `ControlFlow::Continue(())` to ask for the next
model, or `ControlFlow::Break(())` to stop the search. `clingox::prelude` brings in
the common types, including `ControlFlow`.

The next chapter, [Facts and rules](../tutorial/facts-and-rules.md), starts the
tutorial: it adds data from Rust to a program.

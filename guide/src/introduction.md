# Introduction

clingox is a Rust library for [clingo](https://potassco.org/clingo/), the answer set
programming (ASP) system from the Potassco project. You describe a problem as a
logic program, clingo finds its answer sets, and clingox lets a Rust program add
the program and its facts, ground and solve it, and read the answer sets back.

It is for Rust developers who want a declarative solver inside their program, for
example to check a configuration against rules or to plan a schedule, and for
clingo users who want to drive clingo from Rust. Your code needs no `unsafe`.

## A first look

```rust
use clingox::prelude::*;

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    // Pick exactly one colour, but not red.
    ctl.add_base(
        "colour(red; green; blue).
         { pick(C) : colour(C) } = 1.
         :- pick(red).
         #show pick/1.",
    )?;
    ctl.ground(&[Part::base()])?;

    let (result, models) = ctl.solve_all()?;
    for model in &models {
        println!("{model}");
    }
    println!("{result}");
#   assert_eq!(models.len(), 2);
#   assert_eq!(result.to_string(), "SATISFIABLE");
    Ok(())
}
```

The program prints two answer sets, one with `pick(blue)` and one with
`pick(green)`, and then `SATISFIABLE`.
[Your first program](getting-started/first-program.md) explains each step.

## Two layers

- **The mirror layer** follows clingo's own API, named after its Python module. If
  you know clingo, you will recognise `add`, `ground`, `solve` and
  `assign_external`, and clingo's documentation applies.
- **The typed layer** adds what Rust makes possible: facts and results as your own
  Rust types, explicit solve outcomes, and helpers for testing your rules.

[The two layers](concepts/two-layers.md) explains how they fit together.

## What is covered

clingox builds clingo 5.8.2 from source as part of your Rust build, with patches for
known clingo defects, or links a clingo installed on your system. It wraps 248 of
clingo's 254 C functions: solving, models, configuration and statistics, symbolic
and theory atoms, the backend, ground program observers, propagators, syntax trees,
clingo's own application layer and custom scripting languages. The six functions
left out, and the reason for each, are listed in `docs/dev/COVERAGE.md` in the
repository. Lua is not bound.

The test suite runs on Linux, macOS, Windows, FreeBSD, NetBSD and OpenBSD, on
Android emulators and the iOS simulator, and on WebAssembly under Node.js and in
browsers. The [platform page](reference/platforms.md) says what runs where.

clingox is independent of the Potassco project and is not the `clingo` crate on
crates.io. [Coming from the clingo crate](reference/coming-from-clingo-crate.md)
compares the two.

## Status

clingox is pre-release: pre-releases such as `508.2.0-beta.3` may change the API
between them. [Versions](concepts/versions.md) explains the version numbers, the
compatibility promise and the minimum supported Rust version.

Every Rust example in this guide runs as a test.

## Where to go next

- [Installation](getting-started/installation.md) and
  [your first program](getting-started/first-program.md) get you running.
- The tutorial, from [facts and rules](tutorial/facts-and-rules.md) on, teaches
  the typed layer, solving step by step and optimisation.
- The how-to guides answer specific tasks, such as
  [configuring the solver](how-to/configuration.md) or
  [writing a propagator](how-to/write-a-propagator.md).
- The concepts chapters explain how clingox is designed and why.
- The reference has the platform table, known issues in clingo, and pages for
  readers coming from [pyclingo](reference/coming-from-pyclingo.md) or the clingo
  crate.

The API reference for every type and function is on
[docs.rs](https://docs.rs/clingox). From a checkout, `cargo doc --open` builds the
same pages.

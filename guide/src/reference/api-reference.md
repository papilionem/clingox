# API reference

The reference for every type, function and trait of clingox is its rustdoc. This page
says where to find it, maps the crate by task, and explains the conventions the
reference uses.

## Where it is

- **On docs.rs**: [docs.rs/clingox](https://docs.rs/clingox) for the safe API and
  [docs.rs/clingox-sys](https://docs.rs/clingox-sys) for the raw declarations. The
  address `https://docs.rs/clingox/<version>` shows the release you depend on, such as
  `508.2.0-beta.4`.
- **Locally**: `cargo doc --open -p clingox` in your own project builds the reference
  for exactly the version and features in your `Cargo.lock`, and works offline.

docs.rs builds the reference with the features `derive` and `log`
([Feature flags](feature-flags.md#docsrs)).

## Methods of `Control` are on `ScopedControl`

`Control` is a type alias for `ScopedControl<'static>`, the control you create
yourself. Its methods are documented on
[`ScopedControl`](https://docs.rs/clingox/latest/clingox/struct.ScopedControl.html);
the page of the alias
[`Control`](https://docs.rs/clingox/latest/clingox/type.Control.html) lists only the
alias. The lifetime marks a control that clingo owns and lends to
`Application::main`. A function that should accept either kind takes
`&mut ScopedControl<'_>`:

```rust
use clingox::{Part, ScopedControl};

fn count_answer_sets(ctl: &mut ScopedControl<'_>, program: &str) -> clingox::Result<usize> {
    ctl.add_base(program)?;
    ctl.ground(&[Part::base()])?;
    Ok(ctl.solve_all()?.1.len())
}

let mut ctl = clingox::Control::new()?;
assert_eq!(count_answer_sets(&mut ctl, "{ a; b }.")?, 4);
# Ok::<(), clingox::Error>(())
```

## The crate by task

| Task | Start at | Guide chapter |
|---|---|---|
| create, ground and solve | `Control`, `ControlBuilder`, `Part`, `Assumption`, `SolveResult` | [Your first program](../getting-started/first-program.md) |
| read models | `SolveHandle`, `Model`, `OwnedModel`, `Outcome`, `ShowType` | [Solving step by step](../tutorial/solving-step-by-step.md) |
| Rust types as facts and results | `ToSymbol`, `FromSymbol`, `Predicate`, `sym!`, `Control::add_facts` | [Facts and rules](../tutorial/facts-and-rules.md), [Reading results as Rust types](../tutorial/typed-results.md) |
| ground terms | `Symbol`, `SymbolKind`, `Signature` | [Facts and rules](../tutorial/facts-and-rules.md) |
| bound a search | `SolveOptions`, `InterruptHandle`, `AsyncSolveHandle` | [Set a time budget](../how-to/time-budget.md) |
| options and statistics | `Configuration`, `ConfigKind`, `Statistics`, `StatsTree` | [Configure the solver](../how-to/configuration.md) |
| solve events | `SolveEventHandler`, `ExtendableModel`, `MutableStatistics` | [Solve events and user statistics](../concepts/solve-events.md) |
| the grounding | `SymbolicAtoms`, `ProgramLiteral`, `TheoryAtoms` | [Theory atoms](../concepts/theory-atoms.md) |
| errors | `Error`, `ErrorKind`, `Message`, `Location`, `MessageCode` | [Error kinds](error-kinds.md) |
| testing | module `testing`: `assert_models!`, `parse_answer` | [Test your rules](../how-to/test-your-rules.md) |
| propagators | module `propagate` | [Write a propagator](../how-to/write-a-propagator.md) |
| watching the ground program | module `observer` | [Observing the ground program](../concepts/observer.md) |
| ground rules without the grounder | module `backend` | [The backend](../concepts/backend.md) |
| syntax trees | module `ast` | [Syntax trees](../concepts/syntax-trees.md), [Rewrite programs](../how-to/rewrite-programs.md) |
| clingo's command line | module `application` | [Clingo applications](../how-to/clingo-applications.md) |
| `#script` languages | module `script` | [Custom scripting languages](../how-to/custom-scripting-languages.md) |

`use clingox::prelude::*` brings in the common types, the derives and `sym!`, and
`std::ops::ControlFlow`, which model closures return. It leaves out `Error`,
`ErrorKind`, `Message`, `Location` and the `Result` alias, so a glob import cannot
shadow your own `Result`; name them as `clingox::Error` and so on.

## Conventions in the reference

- **Errors.** Each fallible method has an "Errors" section that lists the kinds it
  returns and says which of them poison the control.
- **clingo's documentation applies.** The mirror layer follows clingo's Python
  module in its names and behaviour, so clingo's own documentation at
  [potassco.org/clingo](https://potassco.org/clingo/) describes what a call does in
  clingo. Where clingox differs, the rustdoc says so.
- **References to the repository.** Some rustdoc cites the design documents of the
  repository: "DESIGN S13" is safety rule S13 in `docs/dev/DESIGN.md`, "U18" an entry
  in `docs/dev/UPSTREAM-ISSUES.md`, and `clingo.h:<line>` a line of clingo's C header.
  `docs/dev/COVERAGE.md` maps every function of clingo's C API to the clingox item
  that wraps it.
- **Non-exhaustive enums.** `ErrorKind`, `MessageCode`, `ConfigKind` and other enums
  that follow clingo's lists are `#[non_exhaustive]`: a `match` needs a wildcard arm,
  so that a later release can add a variant.

## `clingox-sys`

`clingox-sys` declares clingo's C API under its C names and builds clingo. Its
functions are `unsafe`, and clingox wraps all but six of them; you need the crate
only for what clingox does not wrap. It also has four constants that describe the
build: `CLINGO_VERSION` (the header's version), `HAS_THREADS`, `VENDORED` and
`PATCHES`.

`clingox-sys` declares `links = "clingo"`, so it cannot be in the same build as
Potassco's `clingo-sys`, which the `clingo` crate uses: both would define the same C
symbols. [Coming from the clingo crate](coming-from-clingo-crate.md) compares the two
crates.

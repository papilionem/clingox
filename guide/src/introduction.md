# Introduction

clingox is a Rust binding to [clingo](https://potassco.org/clingo/), the answer set
programming (ASP) system from the Potassco project. It builds clingo from source as
part of your Rust build, and exposes clingo's C API through a safe Rust interface.
It covers the whole of clingo's C API except six functions (see
[Status](#status)): solving, models, configuration and statistics, symbolic and
theory atoms, the backend, ground program observers, propagators, syntax trees,
clingo's own application layer and custom scripting languages. It runs on Linux,
Android and WebAssembly (Node.js and browsers), with more platforms planned.

It offers two layers in one crate:

- **The mirror layer** follows clingo's own API, named after its Python module. If
  you know clingo, you will recognise `add`, `ground`, `solve` and
  `assign_external`, and clingo's documentation applies. Time budgets and
  interrupts (`solve_with`, `InterruptHandle`) live here too.
- **The typed layer** adds what Rust makes possible: facts and results as your own
  Rust types, explicit solve outcomes, and helpers for testing your rules.

Neither layer requires `unsafe` in your code.

clingox is independent of the Potassco project and is not the `clingo` crate on
crates.io.

## Status

clingox is pre-release and not yet published on crates.io. It wraps 248 of clingo's
254 C functions. The safe API covers adding, grounding and solving programs, models,
externals, configuration, statistics, symbolic atoms, theory atoms, the backend,
ground program observers, solve events, time budgets and async solving, propagators,
syntax trees (the AST), the application layer (`Application`, which runs clingo's own
command line with your options, printer and `main`), custom scripting languages, and
the typed layer: derives, `sym!`, `add_facts`, typed results and testing helpers.

The six functions that are not wrapped, and the reason for each, are listed in
`docs/dev/COVERAGE.md` in the repository. Lua is not bound. Until the first release,
versions are pre-releases and the API may change between them.

Chapters without a link in the table of contents are planned. Each is written
together with the API it describes, and every Rust example in this guide runs as a
test.

## How this guide is organised

- **Getting started** and the **tutorial** teach clingox from the first program on.
- **How-to guides** answer specific tasks: time budgets, testing, browsers, servers.
- **Concepts** explain how clingox is designed and why.
- **Reference** lists what you look up: platforms, known issues in clingo, and
  guides for readers coming from pyclingo or the clingo crate.

The API reference for every type and function is in the crate's documentation. Until
it is on docs.rs, build it from a checkout with `cargo doc --open`.

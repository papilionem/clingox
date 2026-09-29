# Contributing to clingox

Thank you for looking at clingox. Bug reports, fixes, documentation and tests are all
welcome. This page says how to build the project, what a good change looks like, and
what we ask of contributions that used AI tools.

By taking part you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). Security
problems go through the process in [SECURITY.md](SECURITY.md), not the public tracker.

## Where things are

- [`ARCHITECTURE.md`](ARCHITECTURE.md): a short map of the crates and the invariants
  that hold the design together. Read it first.
- [`docs/dev/DESIGN.md`](docs/dev/DESIGN.md): what clingox is, its safety rules (S1 to
  S20), and why it is built the way it is.
- [`docs/dev/RULES.md`](docs/dev/RULES.md): how changes are made, unsafe code, docs,
  style and commits.
- [`docs/dev/TESTING.md`](docs/dev/TESTING.md): the test suite and how to run it on
  Linux, Android and WebAssembly.
- [`docs/dev/UPGRADING.md`](docs/dev/UPGRADING.md): moving to a new clingo, Rust or
  Emscripten release.

## Building and testing

Clone with the submodule, because the vendored clingo source lives in it:

```sh
git clone --recurse-submodules https://github.com/papilionem/clingox
cd clingox
cargo xtask check
cargo xtask test linux
```

You need a current stable Rust (the version in `rust-version`), CMake, and a C and C++
compiler. Every check is a `cargo xtask` command, so a local run is the run that CI
performs:

| Command | What it does |
|---|---|
| `cargo xtask check` | formatting, clippy, docs, doctests, generated files, `cargo deny`, the unsafe budget, the guide build |
| `cargo xtask test linux` | the full test suite on the host, including `systest` and the compile-fail tests |
| `cargo xtask test wasm` | the suite for `wasm32-unknown-emscripten` under Node.js (`cargo xtask setup wasm` first) |
| `cargo xtask test android` | the suite on a running x86_64 Android emulator |
| `cargo xtask sanitize` | AddressSanitizer, LeakSanitizer and ThreadSanitizer runs (nightly, Linux x86_64) |
| `cargo xtask miri` | the unit tests under Miri (nightly) |

Run `cargo xtask check` and `cargo xtask test linux` before you open a pull request.
The other commands matter when your change touches the areas they cover: threads,
`unsafe` code and callbacks (sanitizers and Miri), or anything that could behave
differently on WebAssembly. The user guide is built with `mdbook build guide`.

## What a good change looks like

The full rules are in [`RULES.md`](docs/dev/RULES.md). The ones that matter most:

- **State the contract, then test it.** Say in the issue or the pull request what the
  change must do. Write the tests first and see them fail for the right reason. A
  test that still passes when you break the code on purpose is too weak.
- **`unsafe` lives in `clingox/src/raw` only.** Every `unsafe` block has a specific
  `SAFETY:` comment naming the invariant it relies on. The number of `unsafe` blocks
  is tracked in `xtask/unsafe-budget`; a change that raises it must say why.
- **Every literal, index and value that clingo would mishandle is checked in Rust**,
  and errors carry context. Nothing panics on caller input, and no panic crosses
  into C.
- **Every wrapped C function has a success test**, and a failure test if the call can
  fail. `docs/dev/COVERAGE.md` is generated from the header; its hand-written columns
  are yours to keep current.
- **Do not edit generated or vendored files by hand:** `clingox-sys/clingo`,
  `clingox-sys/src/bindings.rs` and the generated AST files. Regenerate them with
  `cargo xtask bindgen` and `cargo xtask ast-codegen`. A patch under
  `clingox-sys/patches` changes only when it fixes an upstream defect, with a test.
- **Add dependencies with `cargo add`**, never by writing a version into a manifest.
- **Gate tests that use threads.** WebAssembly builds have no threads by default: a
  test that spawns a thread or passes a thread-count option to clingo checks
  `clingox_sys::HAS_THREADS` first, and doctests avoid thread-only options.
- **Write plainly.** Short declarative sentences, no em-dashes, no filler or hype.
  Comments explain why, not what.

### Commits and the changelog

- One logical change per commit; refactors and behaviour changes go in separate ones.
- The subject is `Area: imperative summary`, at most 72 characters, no trailing period,
  for example `Symbols: add checked tuple constructor`. The body explains why, wrapped
  at 72 characters.
- User-visible changes get an entry under "Unreleased" in [`CHANGELOG.md`](CHANGELOG.md),
  which follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Reporting bugs

Open an issue with the bug report form. It asks for what a maintainer needs to
reproduce the problem:

- the clingox version, and whether clingo is vendored or a system library (and which
  version);
- the operating system, architecture and target, and the Rust toolchain;
- the smallest program that shows the problem, and what you expected instead.

If the problem is in clingo itself, say so and include the same program run through
pyclingo or the `clingo` command line. Known upstream defects and clingox's answers to
them are in [`docs/dev/UPSTREAM-ISSUES.md`](docs/dev/UPSTREAM-ISSUES.md).

A crash, an abort or memory corruption that you can reach from safe clingox code is
a security issue: see [SECURITY.md](SECURITY.md).

## AI-assisted contributions

AI-assisted contributions are welcome. Use the tools that help you, as long as the
result meets the same bar as any other contribution:

- **You are the author.** You must understand the change, have run the tests, and be
  able to explain every line and every design choice in review. "The tool wrote it"
  is not an answer to a review comment.
- **Test it yourself.** Run `cargo xtask check` and the tests that cover your change.
  Do not offer tool output as evidence that you have not read and reproduced.
- **Say so in the pull request** when a substantial part of the change, the tests or
  the description was AI-generated. You do not need to mark small completions or
  editor suggestions.
- **No AI-generated issue spam.** Do not file bug reports, vulnerability reports or
  feature requests that you have not reproduced yourself. An issue needs a program
  that fails, not a plausible-sounding description. Reports that are clearly
  unreviewed tool output will be closed.
- **Repository instructions for coding agents** are in [`AGENTS.md`](AGENTS.md). It
  points agents to the same rules described here.

## Licensing

clingox is available under either the MIT or the Apache-2.0 license, at your option
(see the README). By contributing you agree that your contribution is licensed the
same way, without additional terms.

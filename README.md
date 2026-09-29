# clingox

[![CI][ci-badge]][ci] [![Nightly][nightly-badge]][nightly]

Safe Rust bindings to [clingo](https://github.com/potassco/clingo) 5.8.2, the answer
set programming system from Potassco.

clingox builds clingo from source as part of your Rust build and exposes its C API
through a safe interface: your code needs no `unsafe`. It has two layers in one
crate. The mirror layer follows clingo's own API, so clingo's documentation applies.
The typed layer converts Rust types to facts and reads models back as Rust values.

This is an independent binding. It is not Potassco's
[`clingo`](https://crates.io/crates/clingo) crate.

| Crate | What it is |
|---|---|
| `clingox` | the safe API |
| `clingox-derive` | `#[derive(ToSymbol)]`, `#[derive(FromSymbol)]` and `sym!`, re-exported by `clingox` |
| `clingox-sys` | raw declarations of clingo's C API, and the build of clingo itself |

## Status

Pre-release. The crates are not published on crates.io yet. The safe API wraps 248 of clingo's 254 C functions. It covers adding,
grounding and solving programs, models, externals, configuration, statistics,
symbolic atoms, theory atoms, the backend, ground program observers, solve events,
time budgets, async solving, propagators, the AST (syntax trees), clingo's own
application layer (`Application`), custom scripting languages, and the typed layer.
The six functions left out, and why, are listed in
[`docs/dev/COVERAGE.md`](docs/dev/COVERAGE.md). The first public release is `508.2.0-beta.1`; pre-releases (`-beta.N`) may still
change the API between them.

## Why clingox

The [`clingo`](https://crates.io/crates/clingo) crate from Potassco already binds
clingo for Rust. clingox is a different design, and the differences below are the
reasons to choose it. What it does not do is listed after them. Every number here
comes from the repository's test records.

**A current clingo, with fixes.** The `clingo` crate binds clingo 5.6.2. clingox
builds clingo 5.8.2 from vendored source, and applies eight small patches at build
time for defects in clingo and clasp that a safe binding cannot work around from
Rust. Each patch fixes one entry of
[`docs/dev/UPSTREAM-ISSUES.md`](docs/dev/UPSTREAM-ISSUES.md) and has a test that
fails without it:

| Issue | What the patch fixes |
|---|---|
| U1 | Integer division by zero, or `INT_MIN / -1`, in program text or a term raised SIGFPE and ended the process. |
| U2 | The symbol table was destroyed at process exit while other threads could still use symbols. |
| U19 | clasp's registry of statistic types raced when controls first solved on several threads. |
| U35 | Grounding an `#external` with an arithmetic type term dereferenced a null pointer. |
| U46 | An application's control delivered no statistics or finish event in builds without threads. |
| U47 | An AST node's reference count wrapped after 2^32 clones, a use after free from safe Rust. It now aborts, like `Rc`. |
| U49 | A numeric range that ends at `INT_MAX` never finished grounding. |
| U50 | A parallel search interrupted while splitting leaked its queued guiding paths. |

The patches apply to vendored builds only. A system clingo (`>= 5.8.1, < 5.9.0` is
accepted) keeps these defects, and the guide says so. The record has 50 entries (one is a pyclingo bug), each with its evidence and what
clingox does about it. None has been reported upstream yet.

**Guards that make misuse a Rust error.** The design rules are in
[`docs/dev/DESIGN.md`](docs/dev/DESIGN.md); these are the ones that matter to a user.

- Literals are checked. A `SolverLiteral` has no public constructor from an integer,
  and program literals are validated, because clingo treats a bad literal as
  undefined behaviour (or allocates memory in proportion to it).
- Values clingo would mishandle are refused before the call: strings with a NUL
  byte, integers outside `i32`, negative arities, and the command-line options that
  make `clingo_main` end the process (`--pre`, `--print-portfolio` and others,
  checked against pyclingo on 1100 random command lines).
- An error clingo cannot recover from poisons the `Control`, so a program never
  continues on an unknown state.
- The control that an application's `main` receives is branded with a lifetime, so
  neither it nor anything borrowed from it can outlive the call. This is checked by
  compile-fail tests.
- `Application::run` contains `clingo_main`: it saves and restores the signal
  handlers clasp replaces, flushes output around the run, and allows one run at a
  time.
- Panics in your callbacks never unwind into C++. They are caught and resumed on
  your thread.

**Tested where it will run.** The full suite runs on Linux x86_64, on Android x86_64
(an emulator, API 34) and on WebAssembly (`wasm32-unknown-emscripten`) under Node.js,
Chromium and Firefox. AddressSanitizer with LeakSanitizer and ThreadSanitizer run
over the suite, and Miri runs the unit tests and the callback trampolines. In
`clingox`, `unsafe` code is confined to one internal module (`raw`), with a budget
that `cargo xtask check` enforces. What is not tested is listed under "What is not done".

**Coverage and conformance.** 248 of clingo's 254 C functions are wrapped, each with a
test, and the table is in [`docs/dev/COVERAGE.md`](docs/dev/COVERAGE.md). 264 of 304
upstream test items (clingo's C examples, its C++ and Python unit tests, and its
`.lp` fixtures) are ported and pass. The rest are 36 Lua fixtures, `variant.cc` (a
C++ utility test) and three Python items, each with its reason in
`clingox/tests/conformance/NOT_PORTED.md`.

**Checked against real programs.** The clingo crate's 13 examples and 27 integration
tests, and the test suites of four applications that use clingo (savan, aspire, dasp
and iggy), were ported to clingox and pass. A differential corpus of 700 runs (50
programs under 14 option sets) matched pyclingo 5.8.2 in models, costs, result flags
and search statistics. The guide has
pages for people coming from
[the clingo crate](guide/src/reference/coming-from-clingo-crate.md) and from
[pyclingo](guide/src/reference/coming-from-pyclingo.md).

### What is not done

- **Lua is not bound, and Python is not available.** A `#script (lua)` or
  `#script (python)` block is a parse error. You can add your own language in Rust
  with the `Script` trait.
- **Six C functions are not wrapped.** Three are complements or special cases of
  wrapped functions (`clingo_symbol_create_id` and the two `is_negative` calls), two
  are covered by Rust types (`clingo_error_string`, `clingo_warning_string`), and one
  returns a C++ object (`clingo_control_clasp_facade`). The reasons are in COVERAGE.
- **Some platforms are untested.** Safari, Android ARM64, macOS, Windows, iOS and
  Linux ARM64 have not been run. clingo 5.8 is known to misbehave on Android 11 and
  later on ARM64 devices, and that is not verified here.
- **Some upstream defects remain.** Where clingox cannot patch (a system clingo) or a
  defect is inside clasp's design, it is documented, worked around or refused, and
  the guide's [known issues](guide/src/reference/known-issues.md) page lists what
  users can still meet.
- **clingo 6 is not supported.** clingo 6 is unreleased and replaces the C API
  (255 functions became 267, 146 old names disappeared), so a port would be a new
  major line of clingox. Of the 49 issues checked against its `wip-20` branch, 14
  are still present in some form and 3 changed shape. See
  [`docs/dev/CLINGO6.md`](docs/dev/CLINGO6.md).
- **Performance is measured, with a caveat.** [`docs/dev/BENCHMARKS.md`](docs/dev/BENCHMARKS.md)
  compares clingox with the `clingo` crate and pyclingo: symbols, models and callbacks
  cost about the same as in the crate, and configuration and statistics reads by path
  cost two to three times more. The crate builds clingo 5.6.2, so rows where clingo does
  the work differ by version, not by wrapper. See the guide's
  [performance page](guide/src/reference/performance.md).
- **Not on crates.io, and pre-release.** The API may still change.

## Installation

Until the crates are published, depend on it through git:

```toml
[dependencies]
clingox = { git = "https://github.com/papilionem/clingox" }
```

The build needs Rust 1.98 or newer, CMake 3.10 or newer, and a C and C++ compiler.
The guide's [installation chapter](guide/src/getting-started/installation.md) lists
the features and how to link a clingo installed on your system.

## Example

Facts go in as Rust values, rules derive new atoms, and the results come back as Rust
values:

```rust
use clingox::prelude::*;

#[derive(ToSymbol)]
struct Asserted {
    #[clingo(constant)]
    object: String,
    #[clingo(constant)]
    property: String,
    value: i32,
}

#[derive(FromSymbol, Debug, PartialEq)]
struct Violation {
    #[clingo(string)]
    rule: String,
    #[clingo(constant)]
    culprit: String,
}

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base(
        r#"violation("floor3_enc", O) :- asserted(O, encrypted, 0), asserted(O, floor, 3)."#,
    )?;
    ctl.add_facts([
        Asserted { object: "comp13".into(), property: "encrypted".into(), value: 0 },
        Asserted { object: "comp13".into(), property: "floor".into(), value: 3 },
    ])?;
    ctl.ground(&[Part::base()])?;

    match ctl.solve_first()? {
        Outcome::Sat(model, _) => {
            let violations: Vec<Violation> = model.atoms()?;
            let expected = Violation { rule: "floor3_enc".into(), culprit: "comp13".into() };
            assert_eq!(violations, [expected]);
        }
        Outcome::Unsat => println!("the facts contradict the rules"),
        Outcome::Unknown(result) => println!("the search stopped undecided: {result}"),
    }
    Ok(())
}
```

A `String` field says whether it is a clingo constant (`comp13`) or a string
(`"floor3_enc"`), because the two never match each other in rules. A mismatch between
an atom and the type it is read into is an error, never a silently skipped atom.

## Documentation

- **The guide** in [`guide/`](guide/src/introduction.md) teaches clingox from the first
  program on. Build it with `mdbook serve guide`.
- **The API reference** is the crate documentation. Until it is on docs.rs, build it
  with `cargo doc --open`.

Every Rust example in the guide and in this README runs as a test.

## Versions

The version number carries the clingo version it contains:
`{clingo major}{clingo minor, 2 digits}.{clingo patch}.{our release}`.

| clingox | contains |
|---|---|
| `508.2.0` | clingo 5.8.2, first release |
| `508.2.1` | clingo 5.8.2, a fix or addition on our side |
| `509.0.0` | clingo 5.9.0 |

clingo changes its API in minor releases, so each clingo minor version is a new
major version here, and `cargo update` never moves you across one. To use another
clingo version, pick the clingox release whose number names it. The three crates
always share the same number.

Within `508.2.x` the Rust API has no breaking change, because Cargo treats patch
releases as compatible. A breaking change waits for a new clingo version or happens
between pre-releases.

## Platforms

The full test suite runs on Linux x86_64, on Android x86_64 (on an emulator), and on
WebAssembly (`wasm32-unknown-emscripten`) under Node.js, Chromium and Firefox. Safari,
Android ARM64, macOS, Windows, iOS and Linux ARM64 are not tested yet. The guide's
[platform page](guide/src/reference/platforms.md) has the details.

## Compatibility note

Like every `-sys` crate for clingo, `clingox-sys` declares that it links the native
`clingo` library. Cargo allows only one such crate in a dependency graph, so clingox
cannot be used together with Potassco's `clingo-sys`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. clingo and the code it bundles are under their own permissive
licenses; their notices ship with the vendored source.

[ci]: https://github.com/papilionem/clingox/actions/workflows/ci.yml
[ci-badge]: https://github.com/papilionem/clingox/actions/workflows/ci.yml/badge.svg?branch=main
[nightly]: https://github.com/papilionem/clingox/actions/workflows/nightly.yml
[nightly-badge]: https://github.com/papilionem/clingox/actions/workflows/nightly.yml/badge.svg?branch=main

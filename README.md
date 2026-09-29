# clingox

[![crates.io][crates-badge]][crates]
[![docs.rs][docs-badge]][docs]
[![CI][ci-badge]][ci]
[![License][license-badge]](#license)
[![MSRV][msrv-badge]][msrv]

Safe Rust bindings to [clingo](https://github.com/potassco/clingo) 5.8.2, the answer
set programming system from Potassco.

clingox builds clingo from source as part of your Rust build and exposes its C API
through a safe interface: your code needs no `unsafe`. It has two layers in one
crate. The mirror layer follows clingo's own API, so clingo's documentation applies.
The typed layer converts Rust types to facts and reads models back as Rust values.

This is an independent binding. It is not Potassco's
[`clingo`](https://crates.io/crates/clingo) crate.

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

## Installation

```sh
cargo add clingox@508.2.0-beta.1
```

`508.2.0-beta.1` is the first release, and a pre-release: the API may still change
before `508.2.0` (see [Versions](#versions)).

The build needs Rust 1.98 or newer, CMake 3.10 or newer, and a C and C++ compiler.
The first build compiles clingo, which takes a few minutes; later builds reuse it.
The guide's [installation chapter][installation] lists the features and how to link
a clingo installed on your system instead.

| Crate | What it is |
|---|---|
| `clingox` | the safe API |
| `clingox-derive` | `#[derive(ToSymbol)]`, `#[derive(FromSymbol)]` and `sym!`, re-exported by `clingox` |
| `clingox-sys` | raw declarations of clingo's C API, and the build of clingo itself |

## Why clingox

The [`clingo`](https://crates.io/crates/clingo) crate from Potassco already binds
clingo for Rust. clingox is a different design, and the differences below are the
reasons to choose it.

**A current clingo, with fixes.** The `clingo` crate binds clingo 5.6.2. clingox
builds clingo 5.8.2 from vendored source, and applies eight small patches at build
time for defects in clingo and clasp that a safe binding cannot work around from
Rust. Each patch fixes one entry of [`docs/dev/UPSTREAM-ISSUES.md`][upstream] and has
a test that fails without it:

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

The patches apply to vendored builds only. A system clingo (`>= 5.8.1, < 5.9.0`)
keeps these defects, and the guide says so. The record has 50 entries (one is a
pyclingo bug), each with its evidence and what clingox does about it.

**Guards that make misuse a Rust error.** The design rules are in
[`docs/dev/DESIGN.md`][design]; these are the ones that matter to a user.

- Literals are checked. A `SolverLiteral` has no public constructor from an integer,
  and program literals are validated, because clingo treats a bad literal as
  undefined behaviour (or allocates memory in proportion to it).
- Values clingo would mishandle are refused before the call: strings with a NUL
  byte, integers outside `i32`, negative arities, and the command-line options that
  make `clingo_main` end the process (`--pre`, `--print-portfolio` and others).
- An error clingo cannot recover from poisons the `Control`, so a program never
  continues on an unknown state.
- The control that an application's `main` receives is branded with a lifetime, so
  neither it nor anything borrowed from it can outlive the call. Compile-fail tests
  check this.
- `Application::run` contains `clingo_main`: it saves and restores the signal
  handlers clasp replaces, flushes output around the run, and allows one run at a
  time.
- Panics in your callbacks never unwind into C++. They are caught and resumed on
  your thread.

**Tested where it will run.** The test suite runs in CI on Linux (x86_64, ARM64,
32-bit x86, and ARMv7 under qemu), macOS (ARM64 and x86_64), Windows with MSVC
(x64, ARM64 and 32-bit x86), FreeBSD, NetBSD and OpenBSD, on Android emulators
(x86_64 and 32-bit x86) and the iOS simulator, and on WebAssembly under Node.js,
Chromium, Firefox and WebKit. Some of these jobs are experimental; the guide's
[platform table][platforms] says which, and what each one runs. AddressSanitizer
with LeakSanitizer and ThreadSanitizer run over the suite on Linux, and Miri runs
the unit tests and the callback trampolines.
In `clingox`, `unsafe` code is confined to one internal module (`raw`), with a
budget that `cargo xtask check` enforces.

**Coverage and conformance.** 248 of clingo's 254 C functions are wrapped, each with a
test; the table is in [`docs/dev/COVERAGE.md`][coverage]. 264 of 304 upstream test
items (clingo's C examples, its C++ and Python unit tests, and its `.lp` fixtures)
are ported and pass. The rest are 36 Lua fixtures, `variant.cc` (a C++ utility
test) and three Python items, each with its reason in
`clingox/tests/conformance/NOT_PORTED.md`.

**Checked against real programs.** The clingo crate's 13 examples and 27 integration
tests, and the test suites of four applications that use clingo (savan, aspire, dasp
and iggy), were ported to clingox and pass. The guide has pages for readers coming
from [the clingo crate][from-crate] and from [pyclingo][from-pyclingo].

### What is not done

- **Lua is not bound, and Python is not available.** A `#script (lua)` or
  `#script (python)` block is a parse error. You can add your own language in Rust
  with the `Script` trait.
- **Six C functions are not wrapped.** Three are complements or special cases of
  wrapped functions (`clingo_symbol_create_id` and the two `is_negative` calls), two
  are covered by Rust types (`clingo_error_string`, `clingo_warning_string`), and one
  returns a C++ object (`clingo_control_clasp_facade`). The reasons are in COVERAGE.
- **Some platforms are not tested.** Android on ARM (ARM64 and ARMv7) is built in
  CI but its tests are not run yet. iOS devices, Safari and the Windows `-gnu`
  targets are not tested. clingo 5.8 is known to give wrong results on ARM64
  Android 11 and later; see [known issues][known-issues].
- **Some upstream defects remain.** Where clingox cannot patch (a system clingo) or a
  defect is inside clasp's design, it is documented, worked around or refused. The
  guide's [known issues][known-issues] page lists what users can still meet.
- **clingo 6 is not supported.** clingo 6 is unreleased and replaces the C API
  (146 function names of clingo 5.8 are gone, and 158 are new), so a port would be a new
  major line of clingox. See [`docs/dev/CLINGO6.md`][clingo6].
- **Performance has a measured cost.** Against clingo's C++ API on the same clingo
  5.8.2, grounding and solving take the same time, and small calls (creating or
  reading a symbol, stepping to the next model) cost 20 to 200 ns more each.
  Reading configuration and statistics by path costs two to four times as much. The
  guide's [performance page][performance] and
  [`docs/dev/BENCHMARKS.md`][benchmarks] have the numbers, including a comparison
  with the `clingo` crate and pyclingo.

## Versions

The version number carries the clingo version it contains: `508.2.x` contains clingo
5.8.2, and `509.0.x` will contain clingo 5.9.0. clingo changes its C API in minor
releases, so each clingo minor version is a new major version of clingox, and
`cargo update` never moves you across one. Within `508.2.x` the Rust API has no
breaking change; pre-releases (`-beta.N`) may still change it between them, and
`cargo update` can move a `508.2.0-beta.1` requirement to a later pre-release, so
write `=508.2.0-beta.1` if you need it to stay put. The three crates always share
the same number.

The minimum supported Rust version is 1.98. It can rise in any release, and the
changelog says when it does. The guide's [versions chapter][versions] has the
details.

## Documentation

- [The guide][guide] teaches clingox from the first program on.
- [The API reference][docs] on docs.rs.
- [`CHANGELOG.md`](CHANGELOG.md) lists what changed in each release.

Every Rust example in the guide and in this README runs as a test.

## Compatibility note

Like every `-sys` crate for clingo, `clingox-sys` declares that it links the native
`clingo` library. Cargo allows only one such crate in a dependency graph, so clingox
cannot be used together with Potassco's `clingo-sys`.

## Citing

If you use clingox in research, please cite clingo: M. Gebser, R. Kaminski,
B. Kaufmann and T. Schaub, "Multi-shot ASP solving with clingo", *Theory and
Practice of Logic Programming* 19(1), 27-82, 2019,
[doi:10.1017/S1471068418000054](https://doi.org/10.1017/S1471068418000054).
[`CITATION.cff`](CITATION.cff) has the metadata for clingox itself.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. clingo and the code it bundles are under their own permissive
licenses, listed in
[`clingox-sys/THIRD-PARTY-LICENSES`](clingox-sys/THIRD-PARTY-LICENSES).

Security problems go through the process in [SECURITY.md](SECURITY.md).
Contributions are welcome; [CONTRIBUTING.md](CONTRIBUTING.md) says how to build and
test.

[crates]: https://crates.io/crates/clingox
[crates-badge]: https://img.shields.io/crates/v/clingox.svg
[docs]: https://docs.rs/clingox
[docs-badge]: https://img.shields.io/docsrs/clingox
[ci]: https://github.com/papilionem/clingox/actions/workflows/ci.yml
[ci-badge]: https://github.com/papilionem/clingox/actions/workflows/ci.yml/badge.svg?branch=main
[license-badge]: https://img.shields.io/crates/l/clingox.svg
[msrv]: https://papilionem.github.io/clingox/concepts/versions.html#minimum-supported-rust-version
[msrv-badge]: https://img.shields.io/crates/msrv/clingox.svg
[guide]: https://papilionem.github.io/clingox/
[installation]: https://papilionem.github.io/clingox/getting-started/installation.html
[platforms]: https://papilionem.github.io/clingox/reference/platforms.html
[known-issues]: https://papilionem.github.io/clingox/reference/known-issues.html
[performance]: https://papilionem.github.io/clingox/reference/performance.html
[versions]: https://papilionem.github.io/clingox/concepts/versions.html
[from-crate]: https://papilionem.github.io/clingox/reference/coming-from-clingo-crate.html
[from-pyclingo]: https://papilionem.github.io/clingox/reference/coming-from-pyclingo.html
[upstream]: docs/dev/UPSTREAM-ISSUES.md
[design]: docs/dev/DESIGN.md
[coverage]: docs/dev/COVERAGE.md
[clingo6]: docs/dev/CLINGO6.md
[benchmarks]: docs/dev/BENCHMARKS.md

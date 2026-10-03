# Instructions for AI coding agents

clingox is a safe Rust binding to clingo 5.8.2. This file is for coding agents and for
people directing them. It adds to [`CONTRIBUTING.md`](CONTRIBUTING.md); read that and
[`ARCHITECTURE.md`](ARCHITECTURE.md) first. When these files and a task disagree, say
so and ask instead of choosing.

## Layout and layering

- `clingox-sys`: raw declarations (`src/bindings.rs`, generated) and the build of the
  vendored clingo (`clingo/`, a submodule, and `patches/`).
- `clingox/src/raw`: the only place `unsafe` may appear. One thin wrapper per C
  function and the shared primitives.
- `clingox/src` (everything else): the safe API. `#![deny(unsafe_code)]`. It calls
  `raw`, never `clingox_sys`.
- `clingox-derive`: proc macros. `xtask`: all repository tasks. `systest`: ABI check.
- Design and rules: `docs/dev/DESIGN.md` (safety rules S1 to S20), `docs/dev/RULES.md`,
  `docs/dev/TESTING.md`.

## Commands

Use `cargo xtask`, not bare `cargo`, for builds and checks (it sets up ccache and the
feature flags):

```sh
cargo xtask check          # fmt, clippy, docs, doctests, generated files, deny, shellcheck, unsafe budget
cargo xtask test linux     # full suite on the host, including systest and compile-fail
cargo xtask test wasm      # wasm32-unknown-emscripten under Node.js (setup wasm first)
cargo xtask test android   # x86_64 emulator, needs a running device
cargo xtask sanitize       # ASan/LSan/TSan, nightly, Linux x86_64
cargo xtask miri           # unit tests under Miri, nightly
cargo xtask bindgen        # regenerate clingox-sys/src/bindings.rs
cargo xtask ast-codegen    # regenerate the generated AST files
cargo xtask coverage       # regenerate docs/dev/COVERAGE.md, keeping the notes
```

One test file: `cargo test -p clingox --test <name>`. Run `cargo xtask check` and
`cargo xtask test linux` before you call a change done, and report what you ran.

## Safety rules

- Every `unsafe` block has a specific `SAFETY:` comment that names the invariant. If
  you cannot name it, do not write the block. Do not add `unsafe` outside `raw`.
- The `unsafe` count in `xtask/unsafe-budget` only goes up in a change that adds a
  block and says why.
- Validate every literal, index and count in Rust before it reaches clingo. Never let a
  panic cross into C: callbacks go through the trampoline guard.
- WebAssembly has no threads by default. A test that spawns a thread, or passes `-t N`,
  `--parallel-mode` or a similar option to clingo, checks `clingox_sys::HAS_THREADS` or
  runs inline. Doctests must not use thread-only options either.
- Add tests first and confirm they fail for the right reason. Do not edit a test to make
  it pass; if it looks wrong, stop and explain.

## Do not hand-edit

- `clingox-sys/clingo` (the vendored upstream source).
- `clingox-sys/src/bindings.rs`, `clingox/src/ast/generated.rs` and
  `clingox/src/raw/ast_generated.rs`: regenerate them with the commands above.
- `clingox-sys/patches/*.patch`, unless you are fixing an upstream defect, with a
  test that fails without the patch.
- The hand-written columns of `docs/dev/COVERAGE.md` are yours to keep current, but
  the rows and the summary come from the generator.
- Dependencies: use `cargo add` and `cargo upgrade`. Never type a version into a
  manifest.

## Commits and changelog

- One logical change per commit, subject `Area: imperative summary` (at most 72
  characters, no trailing period), body explains why.
- User-visible changes get a line under "Unreleased" in `CHANGELOG.md`.
- Follow the writing rules in `docs/dev/RULES.md` section 6: plain prose, no
  em-dashes, no filler. Comments explain why, never what.
- Do not rewrite shared history: no force pushes to shared branches.
- If a substantial part of a pull request was written by an AI tool, the pull request
  says so (see `CONTRIBUTING.md`).

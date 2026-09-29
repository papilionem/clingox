# Architecture

This is a short map of clingox for people who want to change it. [`docs/dev/DESIGN.md`](docs/dev/DESIGN.md)
has the full reasoning and the numbered safety rules (S1 to S20) that this page refers
to; [`CONTRIBUTING.md`](CONTRIBUTING.md) says how to build and test.

clingox is a safe Rust binding to clingo 5.8.2, the answer set programming system.
The whole clingo C API is reachable, and code that uses clingox needs no `unsafe`.

## Codemap

The workspace has three published crates and two internal ones.

**`clingox-sys/`** declares clingo's C API and builds clingo itself.
- `src/bindings.rs` is the generated declaration of every function in `clingo.h`.
  It is committed and regenerated with `cargo xtask bindgen`; a stale copy fails
  `cargo xtask check`.
- `build.rs` builds the vendored clingo with CMake, or finds a system clingo, and checks
  the version either way. `build/patch_applier.rs` applies `patches/` to a copy of the
  source; each patch fixes an upstream defect and has a test in `clingox/tests`.
- `clingo/` is the upstream source, a git submodule pinned by `clingo.commit`.

**`clingox/`** is the safe API.
- `src/raw/` is the only module that contains `unsafe`. One thin wrapper per C
  function, plus the primitives they share (`call`, `fill_vec`, `c_str`, trampolines,
  the process guards). Nothing outside it names `clingox_sys`.
- `src/control.rs`, `solve.rs`, `model.rs`, `atoms.rs`, `stats.rs`, `config.rs`,
  `theory.rs`, `backend.rs`, `observer.rs`, `propagate.rs`: the public types, one area
  each. `Control` is the centre; the others are views or callbacks that hang off it.
- `src/ast/` is the syntax-tree layer; `generated.rs` is written by
  `cargo xtask ast-codegen` from clingo's own constructor table.
- `src/application.rs` and `script.rs` wrap clingo's application main and custom
  scripting languages, which are process-global (see below).
- `src/symbol.rs`, `convert.rs`, `facts.rs`: the typed layer that turns Rust values
  into symbols and back. `clingox-derive` generates the trait impls.
- `src/testing.rs` holds helpers for testing logic programs.

**`clingox-derive/`** is the proc macro crate for `ToSymbol`, `FromSymbol` and `sym!`. Generated code calls only `clingox::__private`.

**`xtask/`** is every repository task: `check`, the test runs per target, sanitizers,
Miri, code generation, the coverage table and the conformance recount. It is not
published. **`systest/`** checks the declared ABI against the real header with ctest.

## The layering

```
user code
   |
clingox   (safe API: Control, Model, Symbol, ...)      #![deny(unsafe_code)]
   |
clingox::raw   (unsafe: one wrapper per C function)   the only unsafe code
   |
clingox-sys   (declarations, build of clingo)
   |
clingo, clasp, gringo (C++)
```

Each layer only calls the one below it. The safe layer never sees a raw pointer. The
raw layer never makes a policy decision about the public API; it turns C conventions
into Rust ones (an error flag into `Result`, a size query into a `Vec`) and nothing
more.

## Invariants

These hold everywhere, and most changes are judged against them.

- **`unsafe` lives in `raw` only.** No other module of the library contains it. Every
  block has a `SAFETY:` comment naming the specific invariant. The count is tracked in
  `xtask/unsafe-budget`, and only a change that adds a block raises it.
- **Every value clingo would mishandle is checked in Rust first.** Literals, indices,
  thread counts, arities, string contents and the like are validated before the call,
  because clingo often reports success, or crashes, on bad input (S17,
  `docs/dev/UPSTREAM-ISSUES.md`).
- **Callbacks never unwind into C.** Every trampoline catches panics and turns them
  into an error state that clingo reports back to the caller, who gets the panic or
  error unchanged after the call returns. A callback failure is never passed to a
  clingo path that ends the process (S8, S9).
- **Poisoning.** After an error that leaves clingo's state unreliable, the `Control`
  answers only `Poisoned` instead of continuing on a truncated program (S3).
- **Handles do not outlive what they borrow.** Models, solve handles, atoms and
  statistics are lent from their `Control`; the borrow checker, not a runtime check,
  stops a use after the search moved on (S5, S6). Compile-fail tests pin this.
- **The application control is branded.** `Application::main` receives a control that
  clingo owns. Its type carries an invariant lifetime brand, so it cannot be kept,
  swapped for an owned control, or sent past the call (S19).
- **Process containment.** `clingo_main` and script registration write process-wide
  state, install signal handlers and can end the process. clingox allows one run at
  a time, restores the signal handlers, refuses what cannot be contained, and freezes
  script registration before the first control exists (S18, S20).
- **Threads.** `Control` is `Send` but not `Sync`; lent views are not `Send`. Symbols
  live in a table that is never freed (S12).

## Where tests live

- Unit tests sit in `clingox/src`, next to the code, and run under Miri where they use
  `unsafe`. Trampolines are tested with fake context pointers.
- `clingox/tests/` holds integration tests against the real library: one area per
  file (`api_*.rs`), regression tests for upstream defects and patches
  (`patch_*.rs`, `regression_*.rs`), and child-process tests for anything that can end
  the process.
- `clingox/tests/conformance*` ports upstream clingo's own tests; each ported file keeps
  its origin header, and `NOT_PORTED.md` lists what is left out and why.
- `clingox/tests/ui/` and `ui_pass/` are trybuild cases: programs that must not
  compile, and programs that must.
- `clingox-sys/tests/` covers the raw layer and the patches; `systest/` covers the ABI.
- Doctests, and the Rust code in the guide and the README, run as tests too.

Test policy, and how to run each suite on Linux, Android and WebAssembly, is in
[`docs/dev/TESTING.md`](docs/dev/TESTING.md).

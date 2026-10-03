# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Version numbers follow the
scheme in the guide's [Versions](https://papilionem.github.io/clingox/concepts/versions.html)
chapter: `508.2.x` contains clingo 5.8.2.

## Unreleased

## [508.2.0-beta.3] - 2026-10-03

### Added

- Entry cursors for the configuration and statistics trees: `Configuration::root`
  and `entry`, `Statistics::root` and `entry`, `MutableStatistics::root` and
  `entry`, the entry types `ConfigEntry` and `StatsEntry` with their `children`
  iterators, and `PathSegment`. An entry holds clingo's key for one place in the
  tree, so a step costs one to three calls into clingo instead of resolving a
  path from the root on each read: a configuration walk takes 26.8 us instead of
  77.6 us. The path methods are unchanged.
- The guide's ten planned chapters: setting a time budget, testing rules,
  running in the browser and on Android, using clingox in a server, rewriting
  programs, safety and threads, the API reference, feature flags and error kinds.
  Each was checked against the code; the server chapter recommends a separate
  process for programs from clients, since the grounding guard bounds the size of
  the ground program, not time or memory.
- The performance figures in the guide and in `docs/dev/BENCHMARKS.md` were
  measured again, against the `clingo` crate, pyclingo and clingo's C++ API.

### Changed

- A blocking solve (`Control::solve`, `Control::solve_with` without a timeout and
  `Control::solve_with_events`) now runs in clingo's blocking mode and starts no
  thread when nothing can interrupt it: no `InterruptHandle` is alive and no
  timeout is set. The callbacks of such a solve with one solver thread (the
  event handler, the logger, propagators) now run on the calling thread, as in
  clingo and pyclingo, where they used to run on a thread clasp started. With a
  live `InterruptHandle` or a timeout nothing changes, and with several solver
  threads a callback can still come from any of them. A
  trivial multi-shot `Control::solve` takes 5.1 us instead of 35.5 us (median,
  pinned to one core). The `Send` bounds are unchanged.
- Platform support: Linux x86 (32-bit) and ARMv7, Windows ARM64 and x86 (32-bit),
  NetBSD, OpenBSD, Android x86 (32-bit) and ARMv7, and the iOS simulator are now
  ordinary CI jobs, whose failure fails the run, after passing in every run that
  finished since the first release. Android ARM64 stays experimental: it is built
  but not run, since the ARM64 runner has no hardware virtualisation.

### Fixed

- A timeout could be lost when it passed before the search started: the
  interrupt that keeps it acted only on a running search. About 1 in 5 000
  zero-timeout calls of `Control::solve_with_events` and of the yielding
  calls with a timeout (`solve_first_with` and the others) then ran to their
  end. The timeout thread now retries until the search accepts the interrupt or
  the call returns.
- Displaying a symbol that holds a string with invalid UTF-8, which only a string
  read from a file can, panicked (`to_string`, `{}`, and printing a model that
  contains it): `Display` returned an error. It now writes U+FFFD for the invalid
  bytes, as `Symbol::as_string` does. `Control::add_facts` with such a symbol
  reported `BadAlloc`, which poisons; it now reports `Utf8`, adds nothing and does
  not poison, since writing the text lossily would add a different fact.
- Documentation: the 508.2.0-beta.2 notes said no string read is lossy. Only
  syntax tree text reads return `ErrorKind::Utf8` for invalid UTF-8; string
  symbols and symbol and signature names replace it with U+FFFD, as their
  documentation says. A test now covers the string symbol case.
- Documentation: three clingo defects are now recorded. A backend writer
  (`Control::register_backend_writer`) never reports a failed write, so on a
  full disk the file is short or empty without an error (U52). With
  `BackendWriterKind::REIFY`, `reify_steps` alone has no effect and `reify_sccs`
  also adds step numbers (U53). A term nested tens of thousands of levels deep in
  program text overflows the stack when it is added or grounded (U54).
- U14: on WebAssembly the vendored build no longer asks Emscripten's stubbed
  `getrusage` for CPU time, so debug test binaries and doctests stop printing
  `warning: unsupported syscall: __syscall_getrusage` (45,325 times in one run of
  the test suite). CPU time stays 0. `cargo xtask test wasm` now fails on any
  unsupported-syscall report.
- U53: the vendored build patches clingo so that a backend writer's
  `BackendWriterKind::reify_steps` adds step numbers and `reify_sccs` no longer
  does; the reified output now matches the `clingo` command line's for each
  option. A system clingo keeps the defect.

## [508.2.0-beta.2] - 2026-09-30

508.2.0-beta.1 was tagged but not published: crates.io rejected a crate keyword longer than 20 characters.

The first public release: safe bindings to clingo 5.8.2, covering 248 of its 254
C functions, with a vendored build that patches eight clingo defects.

### Added

The workspace has three crates: `clingox` (the safe API), `clingox-derive` (the derive
macros and `sym!`, re-exported by `clingox`) and `clingox-sys` (raw declarations of
clingo's C API, and the build of clingo itself).

**Raw bindings and build (`clingox-sys`)**

- Raw bindings to the complete clingo 5.8.2 C API (254 functions), with clingo built
  from vendored source or linked from an installation on the system.
- Version checks at build time and again at run time: the linked clingo must be
  5.8.1 or newer within 5.8, the versions whose C API the bindings match. The
  vendored clingo is 5.8.2.
- `HAS_THREADS`, `VENDORED` and `PATCHES` say how clingo was built, and dependents'
  build scripts see them as `DEP_CLINGO_THREADS` and `DEP_CLINGO_PATCHES`.
  `CLINGO_THREADS` tells the build whether a system clingo has threads. Thread
  support comes from how clingo was built, not from the target.
- On Windows (MSVC) the linker prints a harmless `/GL` warning for every binary that
  links the vendored clingo; the known issues say how to silence it. clingox keeps
  `/GL`, which makes clingo 3 to 3.5% faster on larger programs.
- Windows: clingo's output and a printer callback's now stay in order, because C
  stdio is flushed around a run as it is on Unix.
- Vendored MSVC builds compile clingo with `/EHsc`. Without it MSVC compiles clingo's
  `try` and `catch` without unwinding, and any exception clingo throws and catches
  itself (a syntax error, an error returned from a callback) hung or crashed the
  process. The MSVC libraries are also linked unbundled, since bundled into the rlib
  they left it without a symbol table. A clingo installed on a Windows system must
  have been built with `/EHsc` too.
- The vendored build applies small patches (listed below) to a copy of the clingo
  source, never to the submodule. A system library gets none. The patch applier
  also works on a checkout with CRLF line endings.

**Solving (`clingox`)**

- `Control` creates a control, adds and grounds programs, and solves them. It has
  builder-style construction (`Control::builder`, `Control::with_args`) with
  arguments, a user logger, a message limit and the shortcuts `threads` and `seed`.
  With the default feature `log`, clingo's messages are also sent to the `log`
  crate under the target `clingox`.
- Solving in several forms: `solve_yield` lends one model at a time through a
  `SolveHandle`; `for_each_model` runs a closure on every model; `solve_first`,
  `solve_optimal` and `solve_all` return owned models (and `Outcome`, which tells an
  unsatisfiable program from an undecided search); `solve_with` takes
  `SolveOptions` (assumptions and a timeout); `solve_async` returns an
  `AsyncSolveHandle`. A model cannot outlive the search step that produced it, and
  a forgotten handle is safe: the control closes the search at its next call or
  when it is dropped.
- `InterruptHandle` stops the running solve call from any thread, and an interrupt
  never reaches a later solve call.
- Multi-shot solving: externals (`assign_external`, `release_external`, by symbol or
  by program literal), `load` and `load_aspif`, `cleanup`, `remove_minimize`,
  `update_project`, `is_conflicting`, `get_const`, unsatisfiable cores
  (`SolveHandle::core`), and `add_facts`, which adds facts from Rust values in a
  part of their own.
- `Model` reads a model's number, cost and priorities, proven optimality, kind
  (stable model or brave/cautious enumeration state), symbols by `ShowType`,
  atoms and shown symbols of one predicate as Rust values, the three-valued
  `Consequence` of a literal, and its thread id. `Model::context` gives a
  `SolveControl` that narrows the rest of the current step with `add_clause`.
- Solve events: `SolveEventHandler` reacts to clingo's four events (`on_model`,
  `on_unsat`, `on_statistics`, `on_finish`) through `solve_with_events`,
  `solve_yield_with_events` and `solve_async_with_events`. `ExtendableModel` adds
  symbols to a model, and `MutableStatistics` writes user statistics.
- Configuration and statistics by path, with introspection (`description`, `len`,
  `element`, `has_key`, `kind`), and `StatsTree`, an owned snapshot of the
  statistics.
- Symbolic atoms and theory atoms (`TheoryAtoms`, `TheoryAtom`, `TheoryElement`,
  `TheoryTerm`), with program literals and facts or externals by signature.
- `Symbol`, `SymbolKind`, `Sign` and `Signature` build, inspect, print, parse and
  order symbols as clingo does.
- One `Error` type with an `ErrorKind`. Parse errors carry clingo's messages with
  their positions, and errors clingo cannot recover from poison the `Control`.
  `Control` is `Send` (never `Sync`).

**Typed layer (`clingox`, `clingox-derive`)**

- `ToSymbol`, `FromSymbol` and `Predicate` convert Rust values to and from symbols,
  with range-checked integers, and `#[derive(ToSymbol)]` and `#[derive(FromSymbol)]`
  with `#[clingo(name = "..")]`, `#[clingo(string)]` and `#[clingo(constant)]`.
- `sym!` builds a symbol from clingo's term syntax with `{expr}` splices, and
  rejects variables, bad commas and out-of-range numbers at compile time.
- `clingox::testing` has `parse_answer` and `assert_models!`.

**Backend and observers (`clingox`)**

- `Control::with_backend` adds ground directives directly, bypassing the grounder:
  rules, weight rules, minimize constraints, projection, externals, assumptions,
  heuristics, acyclicity edges, and theory terms, elements and atoms.
- `GroundProgramObserver` watches the ground program as clingo builds it, and
  `Control::register_backend_writer` dumps it in reify, aspif or smodels format.
  `GroundingLimit` and `LimitedObserver` bound the size of a grounding.

**Propagators (`clingox`)**

- `Propagator` (`init`, `propagate`, `undo`, `check`, `decide`, all with `&self`)
  registered with `register_propagator`, with `PropagateInit`, `PropagateControl`,
  `Assignment`, `Trail`, watches, clauses (`ClauseType`), weight constraints,
  minimize terms, `CheckMode` and `UndoMode`. All 17 assignment functions and all 8
  propagate-control functions are wrapped. `Propagator: Send + Sync` because clasp
  can call a propagator for several threads at once.
- `SolverLiteral` cannot be built from a raw integer. Every method that takes one
  validates it against the control's own assignment first.

**Syntax trees and programs (`clingox`)**

- `clingox::ast`: reference-counted `Ast` handles with structural equality and
  ordering, typed read accessors and checked setters and array editors, one
  constructor per node type (generated from clingo's own tables by
  `cargo xtask ast-codegen`), `parse_string` and `parse_files`, a `Visitor` that
  rewrites a tree by identity, `Ast::unpool`, and `Span`.
- `Control::with_program_builder` adds statements as `Ast` values.

**Applications and scripting (`clingox`)**

- `application::Application` runs clingo's own command line from Rust, with your
  own `main`, options (`register_options`, `validate_options`), model printer and
  logger. The run is contained: one run at a time per process, signal dispositions
  restored, and command lines that would end the process from inside clingo refused.
  C's standard output is flushed around the run on every platform, Windows
  included, so clingo's output and your printer's appear in order.
- `script::Script` and `script::register` implement custom scripting languages for
  `#script (name) ... #end.` blocks and `@name(..)` terms.

**Tooling, tests and documentation**

- The test suite runs in CI on Linux (x86_64, ARM64, 32-bit x86, ARMv7 under qemu),
  macOS (ARM64 and x86_64), Windows with MSVC (x64, ARM64, 32-bit x86), FreeBSD,
  NetBSD and OpenBSD, on Android emulators (x86_64 and 32-bit x86, and ARMv7 through
  the emulator's ARM translation), the iOS simulator, and WebAssembly under Node.js,
  Chromium, Firefox and WebKit. Android ARM64 is built but not yet run. The guide's
  [platform page](https://papilionem.github.io/clingox/reference/platforms.html)
  says what each job runs and which are experimental.
- `cargo xtask` runs the checks: lints, the `unsafe` budget, the guide build and
  doctests (`check`), tests per target (`test`), Miri on the `unsafe` primitives,
  sanitizer runs (`sanitize`), API compatibility (`semver`) and the conformance
  recount (`conformance-count`).
- A conformance suite ports 264 of 304 upstream test items (clingo's C examples,
  its C++ and Python unit tests and its `.lp` fixtures); the rest and their reasons
  are in `clingox/tests/conformance/NOT_PORTED.md`.
- The guide (getting started, tutorial, concepts, how-to pages and reference,
  including pages for people coming from the clingo crate and from pyclingo),
  published at <https://papilionem.github.io/clingox/>. Every Rust block in the
  guide and the README runs as a test.
- `CITATION.cff` gives the metadata for citing clingox, and points to the paper to
  cite for clingo.
- Design rules, test strategy, the coverage table, the record of upstream defects
  (`docs/dev/UPSTREAM-ISSUES.md`) and the check of those defects against clingo 6
  (`docs/dev/CLINGO6.md`).
- A criterion benchmark suite for clingox's own layer (`clingox/benches/layer.rs`)
  and its results against the `clingo` crate, pyclingo and clingo's own C++ API
  (`docs/dev/BENCHMARKS.md`). On the same clingo 5.8.2 as the C++ API, grounding and
  solving take the same time, and small calls cost 20 to 200 ns more each. Queries that cannot fail no longer reset clingo's
  error state before each call, which makes reading symbols, atoms and statistics
  two to nine times faster; no check was removed.
- Releases start from a version tag. The release workflow checks the tag against the
  manifests and this changelog, runs the full CI matrix, publishes the three crates
  (through crates.io trusted publishing once they exist), and attaches the `.crate`
  files with SHA-256 checksums and build provenance attestations to the GitHub
  Release.

### Fixed

Defects in clingo 5.8.2, fixed by patches in the vendored build. A system clingo
keeps each defect, and the documentation says where it matters. The numbers refer
to `docs/dev/UPSTREAM-ISSUES.md`.

- U1: dividing the smallest integer by -1, taking `x \ 0` in a term, or matching a
  linear term such as `-X+0` against a domain value no longer kills the process.
- U2: clingo's symbol table is never destroyed, so symbols stay valid while the
  process exits.
- U19: clasp's registry of statistic types is thread-safe, so controls that first
  solve and read statistics on several threads at once no longer race.
- U35: grounding an `#external` whose type term is a non-linear arithmetic term,
  such as `[X\2]` or `[|X|]`, no longer crashes the process.
- U46: the control of `Application::main` delivers `on_statistics` and `on_finish`
  on WebAssembly without atomics, as an owned control does.
- U47: clingo aborts when the reference count of an AST node would wrap around,
  which safe code could reach by forgetting four billion clones.
- U49: a numeric range that ends at `INT_MAX` no longer runs forever.
- U50: the guiding paths that clasp's parallel search had split off and not yet
  handed out are freed when it is stopped in splitting mode.

### Security

These guards keep safe code from reaching undefined behaviour or unbounded
resource use in clingo. They are checked by clingox before clingo is called, or
by the way an API is shaped.

- Program literals reject `0` and any magnitude above 2^28 - 1
  (`ProgramLiteral::MAX_MAGNITUDE`): clingo 5.8.2 allocated about 19 GB before
  reporting an error for a huge one (U22).
- Solver literals, decision levels and trail offsets are validated against the
  control's own assignment, and a further call after `Flow::Stop` is refused.
- AST setters refuse a node as its own descendant (U29), an index out of range
  (U33), a value out of range for an enum or boolean attribute (U31), a NUL byte,
  and a negative arity (U48). Line and column numbers above `u32::MAX` are refused
  rather than truncated (U30).
- `TheoryElement::condition` copies clingo's shared scratch buffer instead of
  borrowing it (U24), and syntax tree text reads never return a lossy conversion:
  non-UTF-8 text is `ErrorKind::Utf8`. String symbols and symbol and signature
  names replace invalid UTF-8 with U+FFFD (corrected after the release; the notes
  first said every string read).
- Solve-event handlers never report failure to clingo for any event, because
  clingo aborts the process for three of the four (U25) and corrupts clasp for
  the model event of an async parallel search (U26). The handler's error or
  panic is kept and returned or resumed by the call that owns the search.
- An error or panic that stops grounding partway, or a failed `load`, poisons the
  control: clingo keeps the truncated program and would answer from it silently
  (U23, U44). `Propagator::propagate`, `undo`, `check` and `decide` failures do not
  poison; `init` failures do.
- `Application::run` restores clasp's signal dispositions before any user `Drop`
  code runs (U38), refuses a second concurrent run and a second `--mode=clasp`
  run (U51), and reads grouped short options the way clasp does.
- `script::version` takes the registry lock (U41), and script registration must
  happen before the process creates a control or runs an application.
- Statistics key names that clingo's own path syntax cannot address again are
  refused, so a user statistics tree can always be read back.
- Deep theory terms are resolved, printed, cloned, compared and dropped without
  recursion on the Rust stack, and the AST cycle check is iterative. Where clingo
  itself recurses (dropping, printing, copying or comparing a very deep syntax
  tree) the limit is documented (U34).

[508.2.0-beta.2]: https://github.com/papilionem/clingox/releases/tag/v508.2.0-beta.2

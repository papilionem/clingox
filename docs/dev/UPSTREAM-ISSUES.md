# Upstream issues

Problems found in clingo, clasp, gringo, libpotassco or the toolchains while building
clingox. They are not clingox bugs, but clingox users are exposed to them, so each
one is recorded with its evidence, its impact, what clingox does about it, and its
status. Each entry also says whether upstream already knows ("Upstream tracker"),
checked against every issue in potassco/clingo, potassco/clasp and
potassco/libpotassco on 2026-09-27. Nothing has been reported upstream yet. The
user-facing summary is `guide/src/reference/known-issues.md`.

**On every clingo upgrade** (UPGRADING §1.5), re-check each open entry against the
new release: an upstream fix may make a clingox workaround unnecessary, and a
workaround may break.

Status values: **patched** (fixed by a build-time patch in `clingox-sys/patches/`,
vendored builds only; RULES §8), **open** (affects users, no clingox mitigation yet), **mitigated**
(clingox works around it), **documented** (behaviour users must know, nothing to
fix in clingox), **decision pending** (a mitigation is being chosen). Every patched
entry has a "Remove when" line naming the upstream change that makes its patch
unnecessary.

## Index

Every entry, in numeric order. U32 is a pyclingo bug and is noted in section 10. The last column is the result of checking the entry against clingo 6 (wip-20); [CLINGO6.md](CLINGO6.md) has the method and the evidence, and each entry repeats its line.

| # | Issue | Status | clingo 6 (wip-20) |
|---|---|---|---|
| U1 | Integer division traps abort the process | patched | fixed |
| U2 | The symbol table is destroyed at process exit | patched | not applicable |
| U3 | The error-state guard spells a macro emcc does not define | checked, harmless | not applicable |
| U4 | An interrupted search can report "unsatisfiable, exhausted" | mitigated | still present |
| U5 | Integers in program text wrap silently | mitigated | fixed |
| U6 | Error kinds differ from the header's documentation | mitigated | still present |
| U7 | A parse error leaves the control unable to ground | mitigated | fixed |
| U8 | Interrupts are queued for the next search | mitigated | still present |
| U9 | A rejected option value corrupts the option | mitigated | not reproduced |
| U10 | Some options exist only in threaded builds | documented | by design, unchanged |
| U11 | A shared global in clasp is written by every control | documented as benign | fixed |
| U12 | clasp's termination flag is not atomic | documented | still present at a different site |
| U13 | Android's C library lacks `canonicalize_file_name` | mitigated | fixed |
| U14 | No CPU time on WebAssembly | documented | still present |
| U15 | WebAssembly exceptions and LTO | mitigated | unchanged |
| U16 | Playwright's WebKit does not run on Fedora | documented | not applicable |
| U17 | The C API changes in minor releases, without a SOVERSION change | mitigated | still present |
| U18 | clingo 5.8 misbehaves on ARM64 Android 11 and later | open | unverified |
| U19 | clasp's registry of statistic types is not thread-safe | patched | fixed |
| U20 | One reader of clasp's shared optimum skips the generation check | recorded, not patched | still present |
| U21 | A clause-count statistic in clasp's shared implication graph is not atomic | documented as benign | fixed |
| U22 | An out-of-range literal makes clingo allocate memory in proportion to it | worked around in clingox | still present on the backend path |
| U23 | A failed `load` leaves the control unable to parse anything again | worked around in clingox | fixed |
| U24 | `clingo_theory_atoms_element_condition` reuses one scratch buffer for every element | worked around in clingox | still present, in a changed shape |
| U25 | The solve-event callback aborts the process instead of failing, for three of its four events | documented | fixed for the model and statistics events |
| U26 | An exception from the model callback of an async parallel search corrupts clasp | worked around in clingox | not reproduced |
| U27 | A 36-byte leak in `NonGroundParser::aspif_theory_` | documented | not applicable |
| U28 | `PropagateControl::add_watch` races the master solver's own assignment word | documented as benign | not reproduced |
| U29 | The AST setters accept a node as its own descendant, and the process crashes later | mitigated | not applicable |
| U30 | Locations are truncated to 32 bits without an error | mitigated | fixed |
| U31 | Enum-valued AST numbers are not validated | mitigated | still present |
| U32 | `StrSequence.__setitem__` calls a function that does not exist (pyclingo) | pyclingo bug | not applicable |
| U33 | Six AST array editors do not check their index | mitigated | not applicable |
| U34 | Deeply nested syntax trees overflow the stack when released, printed, copied or compared | documented | still present |
| U35 | Grounding an `#external` with an arithmetic type term crashes the process | patched | fixed |
| U36 | The unpool flags do not do what the header says | documented | not applicable |
| U37 | The program builder tolerates misuse and loses statements silently | mitigated | changed |
| U38 | `clingo_main` installs signal handlers and never removes them | mitigated | changed |
| U39 | A failing `validate_options` callback makes `clingo_main` return 0 | documented | fixed |
| U40 | A `printer` callback that reads a model segfaults when the application has no `main` | mitigated | changed |
| U41 | The script registry is unsynchronised and its `free` callback crashes at exit | mitigated | still present for NULL members |
| U42 | `clingo_configuration_array_at` does not check its offset | mitigated | still present |
| U43 | clingo's help formatter swallows `%` and the next character in an option description | mitigated | still present, changed |
| U44 | A failed `#script` block leaves the rest of the program queued in the parser | mitigated | fixed |
| U45 | A script that answers `main` takes over every later default run in the process | documented | fixed |
| U46 | The application's control delivers no statistics or finish event without threads | patched | fixed |
| U47 | An AST node's reference count wraps around from the C API | patched | not applicable |
| U48 | A huge `#project` arity allocates until memory runs out | mitigated | fixed |
| U49 | A numeric range that ends at `INT_MAX` never ends | patched | fixed |
| U50 | A parallel search stopped in splitting mode leaks its queued guiding paths | patched | not checked |
| U51 | clasp opens its input once per process, so a second `--mode=clasp` run answers for the first run's file | refused in clingox | not checked |

## 1. Crashes and undefined behaviour

### U1. Integer division traps abort the process

- **What:** gringo computes `INT_MIN / -1` and `INT_MIN % -1` without a check, and
  the term parser does not check `%` (clingo's `\`) for a zero divisor. On x86 this
  raises SIGFPE and kills the process. Signed overflow in `+`, `-`, `*`, unary minus
  and power is undefined behaviour in the same function, but does not trap on x86.
- **Evidence:** `libgringo/src/input/groundtermparser.cc:44-50` (checks only `DIV`),
  `libgringo/src/term.cc:829-860` (`eval`, only an `assert`), and two more sites that
  compute the same trapping division inline rather than through `eval`:
  `LinearTerm::match` (`term.cc:1681-1694`) and `GLinearTerm::match`
  (`term.cc:648-659`), reached when a linear term such as `-X+0` is matched against
  a domain value (`gringo/domain.hh`, and the dependency analysis of
  `gringo/ground/dependency.hh`). Crashing inputs:
  - the term `1\0` through `clingo_parse_term`, which also kills clingo's own Python
    module (exit 136, reproduced 2026-09-27);
  - the terms `(-2147483647-1)/-1` and `-2147483648/-1`;
  - programs such as `p(X/Y) :- X=-2147483647-1, Y=-1.`;
  - `p(-2147483648). q(X) :- p(X), p(-X+0).` (crashes in `LinearTerm::match`) and
    `q(-2147483648). p(-Y+0) :- p(-2147483648), q(Y).` (crashes in
    `GLinearTerm::match`), reproduced on the patched build and on unpatched
    upstream 5.8.2 (exit 136, same signal).

  Found by fuzzing in under a minute; the two match sites by code review.
- **Impact:** any program or term text a user does not control can abort the whole
  process. It is a signal, not a Rust panic, so it cannot be caught. The two match
  sites mean any rule shaped like `t(X) :- p(X), p(-X+0).` crashes as soon as `p`'s
  domain holds `INT_MIN`, which is ordinary program text, not only crafted input.
- **clingox:** patched (`clingox-sys/patches/U1-division-traps.patch`). A new
  `Gringo::isUndefined(op, x, y)` treats division and modulo by zero, and
  `INT_MIN / -1` and `INT_MIN \ -1`, as undefined; the ground-term parser, the
  constant folding in `BinOpTerm::simplify` and `BinOpTerm::eval`, and now
  `LinearTerm::match` and `GLinearTerm::match`, all use it. The term parser fails
  with "parsing failed"; the grounder logs "operation undefined" and drops the rule
  instance for the `eval` sites, as clingo does for `1/0`; the two match sites treat
  the trapping case as "no match" (no `int` solves `m * X = c` when that needs an
  `X` outside the range of `int`), an ordinary failed match with no message.
  Scanning text in Rust first would not have been a sound substitute, because the
  values can arise during grounding. `clingox/tests/patch_u1_division.rs` proves it,
  including the two match-site programs above.
- **Platform note:** on x86, `INT_MIN / -1` and `INT_MIN % -1` trap, so
  rejecting them loses nothing there. On AArch64 and other targets whose `sdiv`
  does not signal on these operands, the pre-patch code returned `INT_MIN` and `0`
  respectively instead of crashing; the patch now treats these as undefined on every
  platform. The values were undefined behaviour in C++ regardless of platform, so
  the change is defensible, but it does change the answer set on non-x86 targets for
  programs that used to rely on the AArch64 result. Not run on AArch64 hardware.
- **Upstream tracker:** not reported. clingo#100 (2018, fixed) covered `\0` in *program* text; the same division through `clingo_parse_term`, `INT_MIN` divided by -1 anywhere, and the two term-matcher sites, are new.
- **Status:** patched. Vendored builds only; a system library keeps the bug.
- **Remove when:** the clingo release that clingox binds checks division and modulo in
  the term parser, `BinOpTerm` and both `match` sites (`Gringo::isUndefined` or an
  equivalent); a fix is not reported upstream yet.
- **clingo 6 (wip-20):** Fixed (run). Numbers are arbitrary precision and division goes through `Util::check_div`; both match-site programs run clean. The `clingo_parse_term`-style path was not exercised.

### U2. The symbol table is destroyed at process exit

- **What:** clingo's global symbol table is a C++ static with a destructor. When
  `main` returns, the static destructors run while other threads may still be using
  symbols. Borrowed symbol text (`&'static str`) then points into freed memory, and
  creating a symbol inserts into a destroyed table.
- **Evidence:** `libgringo/src/symbol.cc:92-117` (`UniqueConstruct<T>::arr_`). In
  a stress test, a worker thread using symbols while `main` returned segfaulted in 144
  of 200 runs; read-only use returned corrupted text in 39 of 300 runs.
- **Impact:** only programs whose threads still use clingox when `main` returns.
  It contradicts DESIGN S12 ("symbols live in a global table that is never freed").
- **clingox:** patched (`clingox-sys/patches/U2-leak-symbol-table.patch`).
  `UniqueConstruct<T>`'s table is a function-local reference to a heap object that
  is never destroyed, so symbols stay valid until the process has ended. It stays
  reachable from a static, so LeakSanitizer does not report it.
  `clingox/tests/patch_u2_symbol_table_at_exit.rs` proves it with 80 child processes
  that use symbols while `std::process::exit` runs.
- **Upstream tracker:** not reported. clingo#203 is related but different: symbols are never freed while the process runs (memory growth); clingo 6 moves symbols into reference-counted library objects. The destruction at exit is new.
- **Status:** patched. Vendored builds only; a system library keeps the bug.
- **Remove when:** the clingo release that clingox binds no longer destroys the symbol
  table at exit (clingo 6 has no such table); a fix is not reported upstream yet.
- **clingo 6 (wip-20):** Not applicable. Symbols are reference counted and there is no global symbol table with a static destructor.

### U3. The error-state guard spells a macro emcc does not define

- **What:** clingo keeps its last-error state in thread-local variables, except under
  `#ifdef EMSCRIPTEN`, where it uses plain globals (`libclingo/src/control.cc:41,
  113-120`). This looks as if the threaded Emscripten build shares one
  error state between threads.
- **Evidence:** emcc does not define `EMSCRIPTEN`. `em++ -dM -E` from the pinned
  emsdk 6.0.10 defines `__EMSCRIPTEN__`, and with `-pthread` also
  `__EMSCRIPTEN_PTHREADS__`, but never `EMSCRIPTEN`, and neither clingo's CMake files
  nor emsdk's CMake toolchain add it (checked 2026-09-27, with the compile flags of
  the WASM build). Both WASM builds therefore already take the `thread_local`
  branch, which is correct for both.
- **Impact:** none today. The branch is dead code: it would matter only if a build
  defined `EMSCRIPTEN` itself, and then only for a threaded build.
- **clingox:** none needed. A patch for this was tried and removed once the premise
  proved wrong; it changed nothing in any build clingox makes.
- **Upstream tracker:** not reported. Worth a note upstream that the guard spells a
  macro emcc does not define.
- **Status:** checked, not a bug with emsdk 6.0.10; harmless today.
- **clingo 6 (wip-20):** Not applicable. The error state is `thread_local` unconditionally, with no `EMSCRIPTEN` guard.

### U18. clingo 5.8 misbehaves on ARM64 Android 11 and later

- **What:** clasp and gringo store flags in the high bits of pointers. Android 11+ on
  ARM64 tags pointers (Memory Tagging Extension, pointer authentication), so
  grounding produces no rules and solving returns empty answer sets.
- **Evidence:** reported upstream, not reproduced here. Our Android tests run on an
  x86_64 emulator, which has no pointer tagging.
- **Impact:** clingox on real ARM64 Android phones, the most common Android
  hardware.
- **clingox:** none yet. The x86_64 emulator results do not cover this.
- **Upstream tracker:** known: clingo#475 (2023) and clingo#540 (2025). A maintainer
  wrote that clingo 5.8 has problems with MTE and PAC on ARM and that clingo 6 fixes
  them. clingo 6 is not released yet.
- **Status:** open. Before Android ARM64 is claimed as supported, it must be tested
  on a real device or a tagged-memory ARM64 image, and a workaround or clingo 6 is
  needed.
- **clingo 6 (wip-20):** Unverified. It needs ARM64 Android; the maintainers say it is fixed, but that was not confirmed.

### U22. An out-of-range literal makes clingo allocate memory in proportion to it

- **What:** `clingo_control_assign_external` and `clingo_control_release_external`
  accept any `clingo_literal_t`. Given one far beyond the program's atoms, clingo
  first grows its internal tables to that size, and only then reports "Id out of
  range". clasp cannot represent a variable of 2^30 or more (`varMax`,
  `clasp/clasp/literal.h:48`), yet the tables grow anyway.
- **Evidence:** pyclingo 5.8.2 in a subprocess.
  - With `i32::MAX`, `-i32::MAX` and `i32::MIN`, each call grew the process to
    about 19 GB over about 14 seconds, then returned the error.
  - Afterwards the control was usable.
  - With the address space capped below about 20 GB, the process aborted in glibc
    instead.
- **Impact:** a stall and an out-of-memory kill on ordinary machines, from a
  single call with a bad literal. clingox has no way to pass a raw
  literal without `ProgramLiteral::from_raw`, which checks the range.
- **clingox:** `ProgramLiteral::from_raw` rejects magnitudes of 2^28 or more
  (`ProgramLiteral::MAX_MAGNITUDE`, chosen to match clasp's atom range
  rather than its wider variable range). Its documentation warns that
  in-range literals far beyond the program's atoms still cost about 9 bytes
  per unit of magnitude. Proven by `clingox/tests/api_literal_limits.rs`. No
  patch: a reasonable literal never triggers it, and a bound check belongs
  upstream.
- **Not only `assign_external`/`release_external`:** the same shape of
  allocation is reachable through `clingo_control_load_aspif`, for an atom
  or literal named directly in raw aspif text, and through the backend's own
  functions (`clingo_backend_rule` and the rest), for one built by
  `ProgramLiteral::from_raw`/`Atom` accepted values that are in-range but far
  beyond the program's actual atoms. `Control::load_aspif`
  and the backend's own doc comments now cross-reference this entry.
- **Upstream tracker:** not searched yet.
- **Status:** worked around in clingox.
- **clingo 6 (wip-20):** Still present on the backend path (run). Head atom 5e8 allocates 495 MB before failing. `assign_external` and `release_external` are gone. Issue draft ready.

### U24. `clingo_theory_atoms_element_condition` reuses one scratch buffer for every element

- **What:** the function does not return a stable array owned by the theory
  atoms table, the way `clingo_theory_atoms_term_arguments`,
  `..._element_tuple` and `..._atom_elements` do. It returns a span into
  `DomainData::tempLits_`, one scratch `std::vector<Lit_t>` member that
  *every* call to `clingo_theory_atoms_element_condition`, for any element of
  any theory atom, clears and refills, and which reallocates as it grows.
- **Evidence:** `libgringo/src/output/literals.cc:1551-1560`
  (`DomainData::elemCond`, backing the C function via
  `libclingo/src/control.cc:536-545`), against
  `libgringo/gringo/output/output.hh:130-167`, where `tempLits_` is declared
  and cleared. By contrast `termArgs`, `elemTuple` and `atomElems`
  (`literals.cc:1549-1567`) return `.terms()`/`.elements()` straight from the
  per-term, per-element or per-atom object the theory data table already
  owns, which is stable for the grounding's lifetime; only the condition
  accessor goes through the shared scratch buffer, because it needs to
  translate the element's own literals into aspif literals at call time
  (`getCondition(value)` mapped through `Literal::uid`). Checked directly
  against clingo 5.8.2, both by ctypes on the Python module's C API:
  the condition calls for a 1-literal and a 3-literal element returned the
  same pointer, with the 1-literal call's contents overwritten, and by
  clingox's own regression test
  (`clingox/tests/theory_and_load_regressions.rs::theory_element_condition_survives_later_calls_that_reuse_clingos_scratch_buffer`),
  which reliably aborted with a use-after-free before the workaround below
  (eight elements with growing conditions were needed to reproduce it every
  time in a plain debug build; three did not always show a wrong value
  without a sanitizer).
- **Impact:** a binding that returns this array as a borrow tied to the
  control's lifetime, the natural Rust shape for the other three accessors,
  hands out a reference that a later call on any other element of any theory
  atom invalidates: a use-after-free reachable from safe code, not only from
  holding two conditions at once but even from clingox's own `Debug` for a
  theory element, which itself calls `condition()`.
- **clingox:** `TheoryElement::condition` copies the literals into an owned
  `Vec<ProgramLiteral>` inside the raw wrapper, immediately after the call
  that fills the scratch buffer and before any other clingo call can reuse
  or reallocate it. No borrow of the buffer is ever handed to the safe layer.
- **Upstream tracker:** not searched yet.
- **Status:** worked around in clingox.
- **clingo 6 (wip-20):** Still present, in a changed shape (static). The span now points into a `static thread_local` buffer cleared on each call. Issue draft ready.

### U25. The solve-event callback aborts the process instead of failing, for three of its four events

- **What:** `ClingoSolveEventHandler`, the internal C++ object every
  `clingo_solve_event_callback_t` trampoline goes through, treats a `false`
  return from the callback differently per event. For `on_model`, `false`
  takes the safe path (`throw ClingoError()`, converted by
  `GRINGO_CLINGO_CATCH` into clingo's ordinary error state, which the caller
  sees as `Err`). For `on_unsat`, `on_statistics` and `on_finish`, `false`
  instead calls `clingo_terminate`, an unconditional `std::_Exit(1)` with no
  unwinding, no destructors and no error reported to the caller: the process
  ends as if it had crashed.
- **Evidence:** `libclingo/src/control.cc:1988-2020`
  (`ClingoSolveEventHandler::on_model/on_unsat/on_finish`), `:178-181`
  (`clingo_terminate`, `std::_Exit(1)`). Clingo's own C++ convenience wrapper
  has the identical hazard: `clingo.hh`'s `CLINGO_CALLBACK_CATCH` returns
  `false` uniformly for any exception from any event's callback, so a plain
  C++ user of `Control::solve` with a `SolveEventHandler` subclass hits the
  same abort the first time an `on_unsat`, `on_statistics` or `on_finish`
  override throws.
- **Impact:** any binding whose trampoline maps a callback error to `false`
  uniformly across all four event kinds (the natural first design, and the
  one clingo's own C++ wrapper uses) aborts the host process on the first
  user error or panic from three of the four events, indistinguishable from
  a crash and impossible to catch or report from inside that process.
- **clingox:** the trampoline returns `false` only for `on_model`'s
  `Err`/panic (clingo's own safe, throw-and-catch path); for `on_unsat`,
  `on_statistics` and `on_finish` it stores the error or panic in the usual
  slot, sets `*goon = false` and returns `true`, so clingo never calls
  `clingo_terminate`. Proven under test with a child-process harness,
  because a regression here does not fail normally: it kills the whole test
  binary (`clingox/tests/api_solve_events.rs::an_err_or_panic_from_on_unsat_
  on_statistics_or_on_finish_never_aborts_the_process`).
- **Upstream tracker:** not reported; clingo's own C++ wrapper has the same
  behaviour, so this reads as intended (if easy to get wrong) design rather
  than a bug to file without checking with the maintainers first.
- **Status:** documented; worked around in the trampoline.
- **clingo 6 (wip-20):** Fixed for the model and statistics events (run); the `unsat` event was not exercised, and its adapter still does not catch exceptions.

### U26. An exception from the model callback of an async parallel search corrupts clasp

- **What:** `on_model`'s `false` return is meant to be clingo's *safe* path
  among the four solve events (U25): `ClingoSolveEventHandler::on_model`
  turns it into `throw ClingoError()`, caught and converted to an ordinary
  error state, with no `clingo_terminate`. For an async, multi-threaded
  search, though, taking that path still leaves clasp's own internal state
  inconsistent: the next update on the control (`release_external`,
  `assign_external`, `ground`, `with_backend`, `cleanup`) reads out of
  bounds.
- **Evidence:** pyclingo 5.8.2 reproduction (`async_=True`, an
  `on_model` callback that raises on the second model, `release_external`
  right after); under valgrind, "Invalid read of size 4" /
  "Invalid read of size 1" inside `Clasp::Asp::LogicProgram::
  doUpdateProgram`, reached through `ClingoControl::assignExternal ->
  update -> cleanup`, at "an unallocated block of size 2,238,640". The
  blocking form (`solve_yield=False`, no `async_`) and the yielding form
  are clean; only `async_=True` (also `clingox`'s own blocking
  `solve_with_events`, which runs in async mode) shows it. Reproduced
  independently in clingox itself under AddressSanitizer as a
  heap-buffer-overflow in `Clasp::Assignment::value`, reached from
  `ClingoControl::cleanup` through `release_external`
  (`clingox/tests/parallel_handler_failure.rs`).
- **Impact:** any binding that takes `on_model`'s documented safe path
  (`false`, an ordinary error) for an async parallel search corrupts clasp's
  own state, not just clingox's: the corruption is reached through plain
  pyclingo, with no clingox code involved at all.
- **clingox:** the trampoline never
  returns `false` to clingo for any solve event, including the model event
  (`raw::events::stop`); a model-event failure is stored in the usual slot,
  `*goon` is set to `false`, and the trampoline returns `true`, exactly as
  the other three events already did for U25. Proven by
  `clingox/tests/parallel_handler_failure.rs` under AddressSanitizer.
- **Upstream tracker:** not reported.
- **Status:** worked around in clingox.
- **clingo 6 (wip-20):** Not reproduced (run), though the exact 5.8.2 sequence cannot be run because `release_external` is gone.

### U27. A 36-byte leak in `NonGroundParser::aspif_theory_`

- **What:** LeakSanitizer reports a 36-byte leak attributed to
  `Gringo::Input::NonGroundParser::aspif_theory_` while resolving deeply
  nested theory terms in a stress test.
- **Evidence:** LSan output collected alongside AddressSanitizer runs
  (`detect_leaks=1`); the allocation site names
  `aspif_theory_`, an internal parser buffer, not any clingox-owned
  allocation.
- **Impact:** a fixed, small (36-byte) per-process leak; not proportional to
  program size or repeated calls in the cases observed, so not a practical
  concern for a normal process lifetime.
- **clingox:** no workaround: the allocation is entirely inside gringo's own
  parser, with clingox never holding or freeing the pointer.
- **Upstream tracker:** not reported.
- **Status:** documented, not mitigated.
- **clingo 6 (wip-20):** Not applicable. `aspif_theory_` does not exist; the parser was rewritten.

### U29. The AST setters accept a node as its own descendant, and the process crashes later

- **What:** `clingo_ast_attribute_set_ast`, `_set_optional_ast`, `_set_ast_at` and
  `_insert_ast_at` return success when the value is the node itself or has it below
  it. Nothing else in the AST code guards against a cycle, so `Display`, `Hash`,
  `Eq`, `Ord` and `deep_copy` then recurse without end and overflow the stack.
- **Evidence:** pyclingo 5.8.2: `f.arguments.insert(0, f)`, `rule.head = rule`, and
  an indirect cycle `a.x = b; b.x = a` each report success, and the next `str()`,
  `hash()`, `==` or `copy.deepcopy()` of the node kills the process with SIGSEGV.
  Merely acquiring and releasing the node is fine;
  dropping a cyclic node leaks it.
- **Impact:** with setters that take `&self` and clones that share one node, a
  cycle is reachable from safe Rust, so the process could be crashed without
  `unsafe`. From Python it is an ordinary crash on a mistake.
- **clingox:** the four `ast`-valued setters, `push_ast` and `set_ast_array` refuse
  such a value with `ErrorKind::InvalidInput` before anything changes. The check
  is an iterative depth-first search with a set of visited node pointers, so it is
  linear in the distinct nodes below the value and cannot overflow the stack
  itself. It is skipped only for nodes clingox has just created and not handed out
  (`Visitor`'s rebuilt copy).
- **Upstream tracker:** not reported.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Not applicable. The AST is immutable; there are no setters, so no cycles.

### U33. Six AST array editors do not check their index

- **What:** `clingo_ast_attribute_set_string_at`, `_delete_string_at`,
  `_insert_string_at`, `_set_ast_at`, `_delete_ast_at` and `_insert_ast_at`
  (`libclingo/src/control.cc:1733-1805`) index the underlying `std::vector` with
  `operator[]`, `begin() + index` or `insert(begin() + index)` and report success.
  The read side (`get_*_at`) uses `at` and reports a logic error.
- **Evidence:** pyclingo 5.8.2 driven at the C level:
  `set_ast_at` at `len` reports success and writes past the end; `delete_ast_at`
  at `len + 5` reports success and empties the vector; `insert_ast_at` at
  `len + 5` and `set_ast_at` at 1000000 kill the process. No error code is set in
  any of these.
- **Impact:** an out-of-range index is undefined behaviour, and in practice a
  crash or silent corruption.
- **clingox:** every one of the six checks the index against a fresh length query
  first (`index < len`; `index <= len` for the inserts) and reports
  `ErrorKind::InvalidInput`, with a test for each.
- **Upstream tracker:** not reported.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Not applicable. The array editors are gone.

### U34. Deeply nested syntax trees overflow the stack when released, printed, copied or compared

- **What:** `AST` nodes (`libgringo` `astv2.cc`) are destroyed, printed, deep-copied,
  compared, ordered and hashed by recursion with one call frame per level of
  nesting. `clingo_ast_release` on the root of a deep tree recurses through
  `SAST::clear`, so even dropping a node can exhaust the stack. Parsing does not
  recurse per level.
- **Evidence:** measured on 2026-09-28. On a thread with a 2 MiB stack, dropping
  a parsed node aborts with a stack overflow at about 9 000 levels of
  `f(f(...))` (5 000 survives), and about 20 000 levels of `p(-(-(...)))` or
  `p(+(+(...)))` (9 000 survives). Parsing a million-level `f(` term succeeds.
  `to_string`, `deep_copy`, `==`, `cmp` and `hash` recurse the same way.
  Parenthesised nesting such as `((((1))))` creates no nodes and is fine. The
  same shape as the deep theory terms of U27.
- **Impact:** a process abort, not an error, for input that nests thousands of
  levels deep. Only generated or untrusted input reaches such depths.
- **clingox:** no workaround; clingox adds no recursion of its own. `Ast`,
  `parse_string` and `parse_files` document the limit and advise a large-stack
  thread, and the known issues chapter of the guide lists it. A test parses and
  drops a 50 000-level term on a 512 MiB thread.
- **Upstream tracker:** not reported upstream yet.
- **Status:** documented, not mitigated.
- **clingo 6 (wip-20):** Still present (run). Nesting depths of 20000 and more die with SIGSEGV on an 8 MiB stack in the normal solve path. Issue draft ready.

### U35. Grounding an `#external` with an arithmetic type term crashes the process

- **What:** `ExternalHeadAtom::rewriteArithmetics` calls `rewriteArithmetics` on the
  atom and on the type term and discards the returned replacement. For a term that
  is not linear (`X\2`, `X**2`, `X/2`, `X+X`) or a unary `|X|` or `~X`, that call
  moves the term's operands into the term it registers in the arithmetics map and
  returns the auxiliary variable that must take its place. Discarded, the original
  term stays in the statement with null children, and the next traversal
  dereferences them.
- **Evidence:** `libgringo/src/input/aggregates.cc:2629-2632`. Every other caller
  puts the result in place with `Term::replace` (`input/literals.cc:91`,
  `FunctionTerm::rewriteArithmetics` at `term.cc:2572-2577`). `BinOpTerm` moves
  `left_` and `right_` at `term.cc:2125-2127`, `UnOpTerm` (other than `NEG`) at
  `term.cc:1879-1886`. gdb: `BinOpTerm::collect` (`term.cc:2085`) dereferences the
  null `left_`, reached from `ExternalHeadAtom::collect`, `Statement::check` and
  `ClingoControl::ground`. Linear terms survive because `simplify` folds `X+1`,
  `X*2`, `-X` into a `LinearTerm` or a `NEG`, which rewrite in place and return
  `nullptr`. Crashing inputs (pyclingo 5.8.2, exit 139):

  ```
  f(1..3). #external e(X) : f(X). [X\2]
  f(1..3). #external e(X) : f(X). [X**2]
  f(1..3). #external e(X) : f(X). [|X|]
  ```

  The same happens with `/ & ? ^`, `~`, `X+X`, `X-X`, `X*X`, the operand-swapped
  forms and any nesting whose outermost node is such an operator, with or without a
  condition and with an unbound variable, and in a `#program p(t).` part. 43 of 132
  programs swept crash, all of them `#external`. Through the AST the atom symbol
  crashes the same way (`clingo.ast.External`); `ProjectHeadAtom` and
  `HeuristicHeadAtom` carry the same discarded call for their atom but are not
  reachable, since the parser and the AST layer only build function atoms there.
- **Impact:** any program text can kill the process with SIGSEGV, which cannot be
  caught. Where it does not crash, an arithmetic type term is not an error either:
  `ExternalStatement::report` skips every type other than `true`, `false`, `free`
  and `release` (`ground/statements.cc:600-606`, an upstream `TODO`), so `[X+1]` and
  `[1]` are ignored silently.
- **clingox:** patched (`clingox-sys/patches/U35-external-rewrite-arithmetics.patch`).
  All three calls are wrapped in `Term::replace`, as everywhere else. A type term
  that grounds now declares nothing, as `[X+1]` does, and an unbound variable is
  reported as `unsafe variables`, as for `#external e(X). [X+2]`. Nothing else
  changes: an unrecognised type stays silently skipped.
  `clingox/tests/regression_external_type_crash.rs` runs 37 programs in child
  processes, 27 of which die with SIGSEGV without the patch.
- **Upstream report (draft, not reported upstream yet):** *Grounding `#external` crashes (SIGSEGV)
  when the type term is an arithmetic term.* `f(1..3). #external e(X) : f(X).
  [X\2]` segfaults in clingo 5.8.2 (also `[X**2]`, `[|X|]`, `[X+X]`). In
  `ExternalHeadAtom::rewriteArithmetics` (`libgringo/src/input/aggregates.cc`) the
  results of `atom_->rewriteArithmetics(...)` and `type_->rewriteArithmetics(...)`
  are dropped. `BinOpTerm` and `UnOpTerm` move their operands out and return the
  replacement variable, so the statement keeps a term with null children and
  `Statement::check` dereferences it. Wrapping both calls in `Term::replace`, as
  the other callers do, fixes it (same two lines in `ProjectHeadAtom` and
  `HeuristicHeadAtom`). Related: `ExternalStatement::report` silently ignores a
  type other than `true`, `false`, `free` and `release`.
- **Upstream tracker:** not reported.
- **Status:** patched. Vendored builds only; a system library keeps the bug.
- **Remove when:** the clingo release that clingox binds wraps the results of the three
  `rewriteArithmetics` calls in `ExternalHeadAtom`, `ProjectHeadAtom` and
  `HeuristicHeadAtom` in `Term::replace`; a fix is not reported upstream yet.
- **clingo 6 (wip-20):** Fixed (run). The three crashing programs ground and solve.

### U36. The unpool flags do not do what the header says

- **What:** `clingo_ast_unpool_type_condition` and `_other` (`clingo.h:4114-4120`)
  are documented as "unpool the conditions of conditional literals" and "unpool
  everything else". In `astv2_unpool.cc:184-253`, `condition` acts only when the
  node passed to `clingo_ast_unpool` is itself a conditional literal, and is a
  no-op on every other node; `other` on any other node unpools everything,
  conditions of the conditional literals it contains included.
- **Evidence:** pyclingo 5.8.2: a rule with pools under
  `condition` alone comes back unchanged, one callback with the input node.
- **clingox:** `Unpool` keeps clingo's bits and the rustdoc states the measured
  behavior.
- **Upstream tracker:** not reported.
- **Status:** documented.
- **clingo 6 (wip-20):** Not applicable. `clingo_ast_unpool` is replaced by `clingo_ast_rewrite`; the semantics of the new flags are unverified.

### U37. The program builder tolerates misuse and loses statements silently

- **What:** `clingo_program_builder_add` and `_end` succeed without a `_begin`.
  `clingo_control_ground` and `clingo_control_solve` run on a control whose
  program builder is still open and see an empty program, and a text
  `clingo_control_add` during an open session drops part of the session's
  statements. None of these reports an error.
- **Evidence:** driven at the C level.
- **Impact:** a session left open, for example by a panic between `begin` and
  `end`, silently loses statements.
- **clingox:** the open session has its own flag, and every entry point ends a
  leftover session before it does anything else (`Control::finish_search`, and
  the control's `Drop`). The closure form makes an unbalanced `begin` and `end`
  unreachable from safe code.
- **Upstream tracker:** not reported.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Changed. The builder is replaced by `clingo_program_new/add/free`; misuse behaviour is unverified.

### U38. `clingo_main` installs signal handlers and never removes them

- **What:** `clingo_main` installs clasp's handler for SIGINT, SIGTERM, SIGUSR1,
  SIGUSR2, SIGQUIT, SIGHUP and SIGXCPU (and SIGALRM when `--time-limit` is given), and
  leaves them installed after it returns (`libpotassco/src/application.cpp:117-121`,
  signal list `clasp/src/clasp_app.cpp:137-143`). The handler calls
  `Application::getInstance()->processSignal(sig)`, and `getInstance()` is a global
  that `clingo_main` clears when it returns (`application.cpp:72-75, 113`). A signal
  that arrives later dereferences null. A signal caught while the run is active ends
  the process with `_exit` (`Application::exit`, `application.cpp:167-171`).
- **Evidence:** dispositions read with libc `signal()` before and after one
  `clingo_main` call in pyclingo 5.8.2's process: the seven signals changed to
  clasp's handler; SIGXFSZ, set to `SIG_IGN` by the process, was kept; the SIGALRM
  handler stays installed after a normal `--time-limit` run (the pending alarm is
  cancelled, the handler is not). Delivering SIGTERM after a normal return killed
  the process with SIGSEGV (exit -11). A SIGINT sent from a callback during the run
  printed clasp's summary (`UNKNOWN`, `INTERRUPTED : 1`) and ended the process with
  exit code 1 without returning. `--time-limit=1` behaves the same. The Rust probe is
  `clingox/tests/application_signals.rs`.
- **Impact:** a library that calls `clingo_main` and lives on is crashed by the next
  Ctrl-C or SIGTERM. Even with the handlers restored, a signal delivered to another
  thread in the few instructions between `clingo_main` clearing the singleton and the
  caller restoring the handlers runs clasp's handler with a null instance. The
  `_exit` paths cannot be turned into an error return by a caller.
- **clingox:** `Application::run` saves the nine dispositions with `sigaction`
  before the call, and writes them back after it with the signals blocked on the
  calling thread, on every path including a panic. The window on other threads
  cannot be closed from outside clingo and is documented. The `_exit` paths are
  documented on `run` and in the guide. The restore runs the moment `clingo_main`
  returns, before any user `Drop` code (a SIGTERM during such a `Drop` would otherwise
  crash the process). A suggested upstream fix: restore the
  previous handlers in `Application::run`'s exit path and make the handler
  tolerate a null instance.
- **Upstream tracker:** not reported upstream yet.
- **Status:** mitigated (the restore), with the residual window and the `_exit` paths
  documented.
- **clingo 6 (wip-20):** Changed (run). The SIGTERM handler is still installed and not restored; a later SIGTERM is now swallowed silently instead of crashing. Issue draft ready.

### U40. A `printer` callback that reads a model segfaults when the application has no `main`

- **What:** with `clingo_application_t.printer` set and `main` NULL, calling
  `clingo_model_symbols`, `clingo_model_contains` or `clingo_model_is_true` from
  the printer crashes inside clasp. Only `clingo_model_number`, `_cost` and
  `_optimality_proven` work. With a `main` callback (which makes clingo call
  `keepProgram()` or `enableProgramUpdates()`, `libclingo/src/clingocontrol.cc:297-303`)
  the same calls work.
- **Evidence:** pyclingo 5.8.2 with an `Application` that overrides `print_model`
  but not `main`: `symbols(shown)`, `symbols(atoms)` and `contains` crash at 1, 2 and
  4 threads; gdb shows the fault in `Clasp::Asp::LogicProgram::getLiteral` reached
  through `ClingoModel::atoms`, so `ClingoModel::lp()` dereferences a logic program
  that the default main does not keep.
- **Impact:** a segmentation fault from an ordinary, documented use of the printer
  callback, in any C or C++ program and in pyclingo.
- **clingox:** `Application::print_model` refuses to run without a `main`
  callback (`ErrorKind::InvalidInput`, before anything starts), and the test
  `application_printer_raw` pins the crash with a raw `clingo_main`, so it fails when
  clingo is fixed and the refusal can be lifted. Measured: with a `main`
  present, `symbols` (every `ShowType`), `contains`, `atoms`, `is_consequence`,
  `context().symbolic_atoms()` and `context().add_clause(..)` all work inside the
  printer at one and four threads; `Model::extend` inside the printer changes the
  default printer's output, so it is not exposed there.
- **Upstream tracker:** not reported upstream yet.
- **Status:** mitigated in clingox by the refusal.
- **clingo 6 (wip-20):** Changed (run). No segfault: without `main`, reading the model in `print_model` fails with code 1 "not in solving mode". Issue draft ready.

### U41. The script registry is unsynchronised and its `free` callback crashes at exit

- **What:** `clingo_register_script` appends to a plain `std::vector` (`g_scripts()`,
  `libclingo/src/scripts.cc:64-66`) with no lock, while every grounding reads it. A
  script is consulted for `callable` and `call` once one of its `#script` blocks has
  executed, and that flag is never reset (`scripts.cc:29-56`). Registering the same
  name twice succeeds and then every `execute` runs twice. `call`, `callable` and
  `main` of the `clingo_script_t` are called without a null check
  (`libclingo/src/control.cc:2341, 2348`), so a script registered with a NULL
  `callable` crashes in `ground`; only `execute` and `free` are checked. A non-NULL
  `free` runs during static destruction at process exit.
- **Evidence:** raw `_lib` calls from pyclingo 5.8.2: a script with NULL `callable`
  died with SIGSEGV in `ground`; a script with a `free` callback crashed the
  interpreter at exit (core dump) and with `free = NULL` the exit was clean; a second
  registration of `foo` made `code2.` execute twice. The registry race is read from
  the source, not reproduced.
- **Impact:** registration while another thread grounds is a data race on a vector;
  a `free` callback runs when Rust's runtime may already be gone; misuse of the
  struct is a crash rather than an error.
- **ThreadSanitizer findings:** the
  "script has run a block" flag is written by `execute` and read by `callable` and
  `call` without a lock (`scripts.cc:29-56`); two controls on two threads that
  execute `#script` blocks of one language race on it. `clingo_script_version` also
  walks the same vector while a registration on another
  thread may reallocate it (SIGSEGV in 3 of 3 runs);
  clingox takes its registry lock for `script::version`. The flag race cannot be
  closed from outside clingo; it is documented on `Script`, and a narrow
  ThreadSanitizer suppression for exactly that flag is added only if the suite's
  sweep hits it (as for U28).
- **More measurements (pyclingo 5.8.2 at the C level):**
  registration succeeds while a control exists and even from inside a script's own
  `execute`, and the existing control sees the new language at once; a NULL `name`
  segfaults `clingo_register_script`; every name registers, including `""` and names
  the lexer cannot read, and only lowercase identifiers are reachable from a program;
  two threads grounding two controls were inside the same script's `call` at once
  (clingo takes no lock, so a script must be `Sync`); script callbacks ran only on the
  thread that called `add`, `ground` or `clingo_main`, never on a solver thread
  (`-t4`, sixteen models).
- **clingox:** registration is refused once any `Control` was created or an
  application ran (a sticky flag set before the first `clingo_control_new` or
  `clingo_main`, DESIGN S20), is serialised by a mutex, refuses duplicate names,
  installs every member except `free`, and leaks the script data on purpose.
- **Upstream tracker:** not reported upstream yet.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Still present for NULL members (run). A script with a NULL `callable` or `main` crashes in ground. The registry is per library now. Issue draft ready.

## 2. Wrong or misleading results

### U4. An interrupted search can report "unsatisfiable, exhausted"

- **What:** clingo 5.8.2 sometimes reports an interrupted search of a satisfiable
  program as unsatisfiable and exhausted.
- **Evidence:** `{a;b}.` reported this in 1,460 of 20,000 runs with an interrupting
  thread, and in 12 of 20,000 early cancels. Reproduced with the Python module.
- **clingox:** an interrupted result is never conclusive. `SolveResult` reports it as
  unknown, and the raw flags stay available under a name that says they are not
  trustworthy after an interrupt (DESIGN S13).
- **Upstream tracker:** not reported. clingo#202 (2020, fixed) concerned JSON output of interrupted solving, not the result flags.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Still present (run). 5000 async solves with a racing interrupt gave one "unsatisfiable, exhausted" result. Issue draft ready.

### U5. Integers in program text wrap silently

- **What:** clingo reads `2147483648` in program text as a wrapped 32-bit value, with
  no error.
- **clingox:** conversions from Rust integers are range-checked and fail with
  `ErrorKind::Conversion`.
- **Upstream tracker:** clingo#89 (closed): clingo supports only 32-bit integers. The silent wrap of larger literals is not reported separately.
- **Status:** mitigated for values that come from Rust; documented for program text.
- **clingo 6 (wip-20):** Fixed (run). `p(2147483648).` is kept as 2147483648.

### U30. Locations are truncated to 32 bits without an error

- **What:** `clingo_location_t` carries `size_t` lines and columns, but the AST
  stores them as `unsigned` (`conv(clingo_location_t)`, `control.cc:191`), so a
  larger value is silently cut to its low 32 bits.
- **Evidence:** pyclingo 5.8.2: a location with line `2**40` reads back as line 0.
- **Impact:** a location written and read back is not the location that was
  written, with no diagnostic.
- **clingox:** `Span::new` rejects any line or column above `u32::MAX` with
  `ErrorKind::InvalidInput`, so a `Span` always survives a round trip through
  clingo.
- **Upstream tracker:** not reported.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Fixed (run). A position at line 2**40 prints in full.

### U31. Enum-valued AST numbers are not validated

- **What:** the number attributes that stand for an enum or a boolean (`literal.sign`,
  `guard.comparison`, the operator and aggregate function types, `function.external`
  and the others) accept any integer in `clingo_ast_build` and
  `clingo_ast_attribute_set_number`. `Display` then prints something wrong, and
  code that switches on the value may misbehave.
- **Evidence:** pyclingo 5.8.2: a literal with sign 7 or -1
  prints as if it had no sign, a guard with comparison 99 prints as blank, a
  function with `external` 5 prints as `@f`, a binary operation with operator 99
  prints without an operator.
- **Impact:** a wrong program text with no error; whether a program builder
  rejects such a node is not tested yet.
- **clingox:** the constructors take typed enums (`LiteralSign` and eight more) and
  `bool`, and `Ast::set_number` checks the domain of these attributes and reports
  `ErrorKind::InvalidInput`. Plain numbers (`priority`, `arity`) are unrestricted,
  as in clingo.
- **Upstream tracker:** not reported.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Still present (run). Literal signs 7 and -1 are accepted. Issue draft ready.

### U39. A failing `validate_options` callback makes `clingo_main` return 0

- **What:** when the application's `validate_options` callback returns false, clingo
  prints the callback's message and `Try '--help' for usage information`, and then
  returns exit code **0**, the code for success. `ClaspAppBase::validateOptions` calls
  `setExitCode(0)` before `ClingoApp::validateOptions` invokes the callback
  (`clasp/src/clasp_app.cpp:176`, `libclingo/src/clingo_app.cc:69`). A `register_options`
  failure gives 1 and a `main` failure gives 65.
- **Evidence:** pyclingo 5.8.2 through the raw library with `clingo_set_error` and
  a `validate_options` returning false: message printed, `Try '--help'`, exit 0.
  `register_options` false gave exit 1; `main` false gave exit 65; a `parse` callback
  that fails gives clingo's own text and exit 1, and the message the callback set is
  discarded.
- **Impact:** a script that checks the exit code of an application with invalid
  options sees success.
- **clingox:** none needed for correctness. `Application::validate_options`
  returns the callback's own error from `run`, as do `register_options` and an
  option's `parse` closure, so the exit code is not needed to detect the failure.
  The raw exit code is pinned by `clingox/tests/application_options_raw.rs`.
- **Upstream tracker:** not reported upstream yet.
- **Status:** documented, not mitigated.
- **clingo 6 (wip-20):** Fixed (run). A failing `validate_options` callback gives code 65 and its message.

### U43. clingo's help formatter swallows `%` and the next character in an option description

- **What:** the description given to `clingo_options_add` and
  `clingo_options_add_flag` goes through a printf-like formatter when `--help` is
  printed. A `%` and the character after it are consumed, so `100% sure` prints as
  `100 sure`, and a description that ends in `%` loses it. A doubled `%%` prints one.
- **Evidence:** pyclingo 5.8.2 through the raw library, `--help` with options whose
  descriptions contain `%`.
- **Impact:** an option description with a percentage is displayed wrongly.
- **clingox:** `Options::add` and `Options::add_flag` double every `%` before the
  description reaches clingo, so it is shown literally.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Still present, changed (run). `100% sure and 50%` prints `100 sure and 50` in `--help=3`. Issue draft ready.

### U44. A failed `#script` block leaves the rest of the program queued in the parser

- **What:** when a script's `execute` callback fails during `clingo_control_add` or
  `clingo_control_load`, the call returns false, but the remainder of the input after
  the failing block stays queued in the control's parser, and the **next** `add` or
  `load` on that control parses it. The same happens for a block of an unregistered
  language (`bar support not available`). Only the text before the failing block is
  discarded.
- **Evidence:** pyclingo 5.8.2 through the raw library (cases `exec_leftover` and
  `unknown_leftover`): `add('#script (foo) A #end. a.
  #script (foo) B #end. b.')` with `execute` failing returns false after `A` only;
  with `execute` succeeding, `add('z.')` returns true, `B` runs during that call and
  the model is `{a, b, z}`. The same program loaded from a file with an unregistered
  language gives the same model after a following `add`.
- **Impact:** a control that survives the failure grounds text the caller was told
  had been rejected, silently.
- **clingox:** an error from `execute` poisons the control whatever its kind;
  `Control::add` poisons the unknown-language case (its `Runtime` error is reported as
  `Parse`), and `Control::load` poisons on every failure of `clingo_control_load`
  once the file is confirmed open, keeping the error's kind (the
  same rule as `load_aspif`). This also closes a defect in plain `load`: a syntax
  error or unknown language in a loaded file left the rest of the file queued.
- **Upstream tracker:** not reported upstream yet.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Fixed (run). A failing `#script` block discards the rest of the text.

### U45. A script that answers `main` takes over every later default run in the process

- **What:** `ClingoControl::main` asks the scripts for `callable("main")` only when
  the application has no `main` callback, and only scripts that have executed a block
  in the process are asked; that flag is never reset. Once one script says `main` is
  callable and has run a block, every later default-main run in the process is taken
  over, also over files without a `#script` block and after the control that ran the
  block is gone.
- **Evidence:** pyclingo 5.8.2 at the C level (`main_two_runs`, `main_prior_control`):
  a second `clingo_main` over a file without a block reached the script's `main`, and
  so did a run after a control outside the run had executed a block.
- **Impact:** a library that registers a scripting language with a `main` changes the
  behaviour of unrelated runs in the same process.
- **clingox:** none possible beyond documentation; `Script::main` and the guide say
  so, and a script that must not take over answers `callable("main")` with false.
- **Upstream tracker:** not reported upstream yet.
- **Status:** documented, not mitigated.
- **clingo 6 (wip-20):** Fixed (static). Scripts are per library; there is no process-wide flag.

### U46. The application's control delivers no statistics or finish event without threads

- **What:** `ClingoApp::onEvent` passes clasp's `StepReady` event to
  `ClingoControl::onFinish` only inside `#if CLASP_HAS_THREADS`
  (`libclingo/src/clingo_app.cc:182-189`). `onFinish` is what delivers
  `clingo_solve_event_type_statistics` and `_finish` to a solve event handler
  (`libclingo/src/control.cc:2007-2015`). The library control's twin,
  `ClingoLib::onEvent` (`libclingo/src/clingocontrol.cc:1025-1029`), has no guard.
  In a build without threads (WebAssembly without atomics) a handler on the
  control that `clingo_main` passes to `main` therefore gets model and unsat events
  but never statistics or finish, in any solve mode.
- **Evidence:** the same program and handler on `wasm32-unknown-emscripten` under
  Node (2026-09-29): an owned control, yield search, logged `model, stats, finish`;
  the application's control logged `model` only for a yield search closed with
  `close()`, and `model, model` with a result of SATISFIABLE for two blocking
  `solve_with_events` calls. With threads the guard is true and both controls agree.
  No double delivery is possible when the guard goes: the application's control
  passes no event handler to `ClaspFacade::solve` (`ClingoSolveFuture`,
  `clingocontrol.cc:923-924`), so `StepReady` reaches it only through the shared
  context's handler, once per step (`ClaspFacade::stopStep`, `clasp_facade.cpp:915`,
  reports under `!solved()`), and `onFinish` clears the handler it delivered to
  (`clingocontrol.cc:349-353`); threaded builds run exactly this code today.
- **Impact:** a handler that relies on `on_finish` or `on_statistics` gets nothing on
  the application's control on such builds. `on_model`, `on_unsat` and the result of
  the `solve` call are unaffected. clingox also relies on the finish event to end a
  search left open at the end of `main`.
- **clingox:** patched (`clingox-sys/patches/U46-app-finish-without-threads.patch`),
  which removes the guard. Vendored builds only; a system library keeps the defect.
- **Upstream report (draft, not reported upstream yet):** *The application's control never sends
  statistics or finish events in builds without threads.* `ClingoApp::onEvent`
  guards the `StepReady` to `onFinish` forwarding with `CLASP_HAS_THREADS`, while
  `ClingoLib::onEvent` does not; a solve event handler on the `clingo_main` control
  then never sees `clingo_solve_event_type_statistics` or `_finish`. Removing the
  `#if` fixes it, and `onFinish` is idempotent per handler.
- **Upstream tracker:** not reported upstream yet.
- **Status:** patched. Vendored builds only; a system library keeps the defect.
- **Remove when:** the clingo release that clingox binds forwards `StepReady` to
  `onFinish` in `ClingoApp::onEvent` without the `CLASP_HAS_THREADS` guard; a fix is
  not reported upstream yet.
- **clingo 6 (wip-20):** Fixed (static). There is no `CLASP_HAS_THREADS` guard around the statistics and finish events.

### U47. An AST node's reference count wraps around from the C API

- **What:** `AST::refCount_` is a 32-bit `unsigned` (`libclingo/clingo/astv2.hh:125`)
  and `AST::incRef` is an unchecked increment (`libclingo/src/astv2.cc:238`).
  `clingo_ast_acquire`, the getters `clingo_ast_attribute_get_ast`,
  `_get_optional_ast` and `_get_ast_at` (`control.cc:1616`, `1694`, `1712`, `1767`)
  and every `SAST` copy go through it. `clingo_ast_release` deletes the node when
  the count is zero (`control.cc:1618-1622`). A caller that acquires one node
  2^32 times without releasing brings the count to zero; the next release frees
  the node while other handles are alive.
- **Evidence:** found by adversarial testing (2026-09-29): forget `u32::MAX`
  clones of a node in safe Rust, then clone and drop once. Debug build: the
  surviving handle prints another node after 77 s. AddressSanitizer: heap use
  after free in `AST::type()` from `to_string`, freed by `Ast::drop`.
- **Impact:** a use after free reachable from a program that leaks billions of
  handles. Other counts: the only other reference counts in libclingo and
  libgringo are `std::shared_ptr`s that no API call can copy without bound.
- **clingox:** patched (`clingox-sys/patches/U47-ast-refcount-overflow.patch`):
  `incRef` aborts when the count is at its maximum, as `std::rc::Rc` does on
  overflow. Widening the counter would only move the limit. Vendored builds only;
  the rustdoc of `Ast` states the limit for a system library.
- **Upstream report (draft, not reported upstream yet):** *`AST::incRef` overflows silently.*
  `clingo_ast_acquire` can be called 2^32 times on one node, which wraps the
  32-bit count to zero, and the next `clingo_ast_release` deletes a node that
  handles still point to. An overflow check that aborts (or reports an error
  where the C API can) closes it.
- **Upstream tracker:** not reported upstream yet.
- **Status:** patched. Vendored builds only; a system library keeps the defect.
- **Remove when:** the clingo release that clingox binds checks `AST::incRef` for
  overflow; a fix is not reported upstream yet.
- **clingo 6 (wip-20):** Not applicable. `clingo_ast_t` is a handle over `shared_ptr`; there is no 32-bit counter.

### U48. A huge `#project` arity allocates until memory runs out

- **What:** clingo reads the arity of a `#project p/N.` signature as an unsigned
  number, so `-1` is 4294967295, and `NongroundProgramBuilder::project`
  (`libclingo/src/programbuilder.cc:346-355`) builds one variable per argument.
  The text form `#project p/4294967295.` is a `bad_alloc` in pyclingo; adding a
  `project_signature` node with arity `-1` or 100 000 000 through the program
  builder is killed by the operating system at 4 GB.
- **Evidence:** found by adversarial testing (2026-09-29): an arity of `-1` on `#project`
  and a large arity through the program builder both exhaust memory. An arity of `-1` on `#show`, `#defined` and a
  theory atom definition is harmless.
- **Impact:** a memory exhaustion from a single small statement.
- **clingox:** the four arity attributes (`project_signature`, `show_signature`,
  `defined`, `theory_atom_definition`) refuse a negative value in the generated
  constructors and in `set_number` (`InvalidInput`). A huge positive arity stays
  accepted, because the limit is clingo's own and text input reaches it too; the
  constructors document it. No patch.
- **Upstream report (draft, not reported upstream yet):** *`#project p/N.` allocates N variables
  before it checks anything.* A large arity, such as `#project p/4294967295.`, makes
  the parser run out of memory; a check against a sane maximum, or a lazily built
  projection term, avoids it.
- **Upstream tracker:** not reported upstream yet.
- **Status:** mitigated (negative arities); open (huge arities).
- **clingo 6 (wip-20):** Fixed (run). Large `#project` arities run in 10 MB; a negative AST arity is still accepted by the constructor.

### U49. A numeric range that ends at `INT_MAX` never ends

- **What:** `RangeBinder::next` (`libgringo/src/ground/literals.cc:62-66`) runs a
  range `L..U` as `current_ <= end_ && ... current_++` on an `int`. With `U` equal
  to `INT_MAX` the increment overflows, so the comparison stays true and the range
  restarts: `p(2147483646..2147483647).` grounds forever, allocating until the
  process is killed.
- **Evidence:** found by adversarial testing (2026-09-29); the same text in pyclingo
  does not finish.
  The other range sites in libgringo do not increment: `RangeMatcher::next` only
  compares (`literals.cc:81-115`), `IntervalSet` works on bounds it never steps,
  and the interval solver (`IESolver`, `term.cc`) computes on `slack_t`, wider than
  `int`.
- **Impact:** a program can stall the grounder and exhaust memory with one fact.
- **clingox:** patched (`clingox-sys/patches/U49-range-binder-int-max.patch`): the
  binder empties the range after it has produced `INT_MAX`. Vendored builds only.
- **Upstream report (draft, not reported upstream yet):** *A range ending at `INT_MAX` is
  infinite.* `RangeBinder::next` increments an `int` past `INT_MAX`, which is
  undefined behaviour and in practice wraps, so `p(2147483646..2147483647).` never
  finishes grounding. Stopping after producing `INT_MAX` fixes it.
- **Upstream tracker:** not reported upstream yet.
- **Status:** patched. Vendored builds only; a system library keeps the defect.
- **Remove when:** the clingo release that clingox binds stops `RangeBinder::next` from
  incrementing past `INT_MAX`; a fix is not reported upstream yet.
- **clingo 6 (wip-20):** Fixed (run). `p(2147483646..2147483647).` gives both atoms and finishes.

### U50. A parallel search stopped in splitting mode leaks its queued guiding paths

- **What:** in splitting mode (`--parallel-mode=N,split`) an idle thread asks for work
  and another thread answers in `ParallelHandler::handleSplitMessage` (`clasp/src/parallel_solve.cpp:842-849`):
  it allocates a `LitVec`, fills it in `Solver::split` and hands it to
  `SharedData::pushWork`, whose `workQ` owns it until a thread takes it in
  `requestWork`. `SharedData::reset` and `clearQueue` delete what is left in the
  queue, but `reset` runs at the start of the next search, and `SharedData` has no
  destructor. A search that is interrupted or terminated between the push and the
  pop ends with the path still queued, and `~ParallelSolve` does `delete shared_`
  without freeing it.
- **Evidence:** `cargo xtask sanitize` reported 32 bytes in 2
  allocations (LeakSanitizer, direct in `ParallelSolve::handleMessages`, indirect in
  `Solver::split`) in `application_printer_threads`, where a failing model printer at
  `-t4` makes clingox interrupt the search; intermittent. Reproduced with
  `clingox/tests/patch_u50_parallel_split_leak.rs`: 1500 rounds of a cancelled
  `--parallel-mode=8,split` search leaked 4424 bytes in 220 allocations without the
  patch and nothing with it.
- **Impact:** a few bytes per stopped search, not a growing leak; it shows only under
  a leak detector.
- **clingox:** patched (`clingox-sys/patches/U50-parallel-split-leak.patch`): a
  `~SharedData()` that calls `clearQueue()`. Vendored builds only.
- **Upstream report (draft, not reported upstream yet):** *Guiding paths queued by a split leak when
  a parallel search stops.* `ParallelSolve::SharedData` empties `workQ` in `reset` but
  has no destructor, so paths pushed and not popped before the search ended are never
  freed. A destructor that calls `clearQueue()` fixes it.
- **Upstream tracker:** not reported upstream yet.
- **Status:** patched. Vendored builds only; a system library keeps the defect.
- **Remove when:** the clingo release that clingox binds frees the queued guiding paths
  when `ParallelSolve::SharedData` is destroyed; a fix is not reported upstream yet.
- **clingo 6 (wip-20):** Not checked. The issue was found after the clingo 6 check.

### U51. clasp opens its input once per process, so a second `--mode=clasp` run answers for the first run's file

- **What:** in `--mode=clasp` clasp reads a CNF, OPB or similar file it opens through
  `ClaspAppBase::getStream` (`clasp/src/clasp_app.cpp:408-420`), which keeps the stream
  in a function-local static that is opened on first use and never reset. A later run
  in the same process reads the earlier run's stream, not its own file.
- **Evidence:** `clingox/tests/regression_clasp_mode_second_run.rs`: a first run on a
  satisfiable CNF that failed with exit code 128 (a command-line error after clasp
  opened the file), then a second on an unsatisfiable CNF returned 30 (all models
  found) instead of 20. pyclingo 5.8.2 behaves the same.
- **Impact:** a silently wrong answer, with a normal exit code.
- **clingox:** `Application::run` keeps a process-wide flag that is set when a run
  with `--mode=clasp` is about to be parsed (after the register-callback re-check
  passes, so a run refused there does not count); a later run with `--mode=clasp` returns
  `ErrorKind::InvalidInput` before anything starts. The first such run is allowed.
  A suggested upstream fix: make the stream a member of the application object
  instead of a static.
- **Upstream tracker:** not reported upstream yet.
- **Status:** refused in clingox.
- **clingo 6 (wip-20):** Not checked.

## 3. Error reporting

### U6. Error kinds differ from the header's documentation

- **What:** clingo reports an unknown command-line option as a logic error, although
  `clingo.h` documents a runtime error. An unknown statistics key and reading a
  statistics map as a value are also logic errors.
- **Impact:** in clingox's model, logic errors poison the `Control`.
- **clingox:** statistics paths are checked on the Rust side first and fail with a
  non-poisoning `Runtime` error. The option behaviour is pinned by a test.
- **Upstream tracker:** not reported.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Still present (run). An unknown option in `clingo_control_new` still gives code 2 (logic).

### U7. A parse error leaves the control unable to ground

- **What:** after `clingo_control_add` fails with a syntax error, the same control
  cannot ground (seen in the WASM spike).
- **clingox:** a parse error poisons the `Control` (DESIGN S3).
- **Upstream tracker:** not reported.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Fixed (run). Adding, grounding and solving work after a syntax error.

### U8. Interrupts are queued for the next search

- **What:** an interrupt sent while no search runs, during grounding, or after a
  search finished while its handle is still open, is queued and ends the next search
  as soon as it starts.
- **Evidence:** `clasp/src/clasp_facade.cpp:322, 474-478`.
- **clingox:** a lock-guarded phase machine only lets `InterruptHandle` reach clingo
  while a search is running (DESIGN S13). It was verified with about 400,000 racing
  solves.
- **Upstream tracker:** not reported as a bug; the queuing is clasp's intended behaviour (`clasp_facade.cpp:322`).
- **Status:** mitigated.
- **clingo 6 (wip-20):** Still present (run). An interrupt before a search ends the next search as interrupted; this is clasp's intended queuing.

### U23. A failed `load` leaves the control unable to parse anything again

- **What:** `clingo_control_load` on a file that does not exist fails with
  "parsing failed", as expected. But the control keeps an error flag from its
  logger: every later `clingo_control_add` or `load` on that control fails with
  "parsing failed", and grounding reports "grounding stopped because of errors".
  This holds even for valid programs.
- **Evidence:** reproduced with pyclingo 5.8.2:

  ```python
  ctl.load('/nonexistent')        # RuntimeError: parsing failed
  ctl.add('base', [], 'a.')       # RuntimeError: parsing failed
  ctl.ground([('base', [])])      # RuntimeError: grounding stopped because of errors
  ```

- **Impact:** one bad path breaks a long-lived control for good.
- **clingox:** `Control::load` opens the file itself before calling clingo, and
  reports a missing or unreadable file as `ErrorKind::Runtime` without touching
  clingo. A file removed between that check and clingo's own open still
  triggers the bug. That race is documented on `Control::load`.
- **Upstream tracker:** not searched yet.
- **Status:** worked around in clingox.
- **clingo 6 (wip-20):** Fixed (run). Parsing, grounding and solving work after a failed `parse_files`.

## 4. Configuration

### U9. A rejected option value corrupts the option

- **What:** libpotassco changes an option while parsing a value it then rejects: 74
  of 125 option values probed were left changed. On wasm32, `solve.models` is left at
  0.
- **clingox:** `Configuration::set` restores the previous value after a rejected set.
- **Remaining case:** clasp assigns every tester option its default before parsing a
  tester value (`clasp_options.cpp:984-992`), and the C API cannot unassign an
  option. A tester option that was unassigned therefore reads its default after a
  rejected set. This is documented on `Configuration::set`.
- **Upstream tracker:** not reported. clingo#196 (2020, fixed) concerned option strings not being copied, a different problem.
- **Status:** mitigated, with that one documented exception.
- **clingo 6 (wip-20):** Not reproduced (run), for the one option tried (`solve.models`).

### U10. Some options exist only in threaded builds

- **What:** `solve.parallel_mode`, `global_restarts`, `distribute` and `integrate`
  are defined only when clasp is built with threads
  (`clasp/clasp/cli/clasp_cli_options.inl:519`).
- **Upstream tracker:** not applicable (by design).
- **Status:** documented (this is clasp's design, not a bug).
- **clingo 6 (wip-20):** By design, unchanged.

### U42. `clingo_configuration_array_at` does not check its offset

- **What:** the offset is not compared with the array's size. On `-t 3`,
  `solver` has 3 elements, and `array_at` answers for offsets 0 to 4 without an
  error (3 and 4 give a key past the end); 100 fails with a runtime error; an
  offset of `2**40` succeeds and returns the key of element 0, because the
  offset is truncated. The header documents the access past the end only as a
  note for entries such as the solver configuration ("can be accessed past
  their actual size to add subentries"), not the wrap-around.
- **Evidence:** pyclingo 5.8.2, C level (`clingo._internal._lib`), `-t 3`:
  `array_size(solver)` is 3; `array_at(solver, 0)` and `(solver, 2)` succeed;
  `(solver, 3)` and `(solver, 4)` succeed with a key; `(solver, 100)` fails with
  error code 1; `(solver, 2**40)` succeeds and returns the key of element 0.
  `array_size` and `array_at` on a non-array (`""`, `solve`, `solver.0`,
  `solver.0.seed`) fail with code 1.
- **clingox:** `Configuration::element` checks `index < len` itself and returns
  `ErrorKind::InvalidInput`, so a caller never receives a key or path past the
  end. Growing the array is done with `Configuration::set`.
- **Upstream tracker:** not reported upstream yet.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Still present (run). An offset of 100 fails and 2**40 returns element 0's key. Issue draft ready.

## 5. Concurrency inside clasp

### U11. A shared global in clasp is written by every control

- **What:** every `LogicProgram` in the process shares the static
  `Clasp::Asp::trueAtom_g`, and controls on different threads write to it.
  ThreadSanitizer reports these as data races.
- **Evidence:** `clasp/src/logic_program.cpp:169, 172, 1017`. A source review found that
  every write stores the value already present, or a flag that is never read for
  this atom. 33,600 concurrent solves matched a single-threaded reference with no
  difference.
- **clingox:** no lock (DESIGN S12).
  `xtask/tsan-suppressions.txt` suppresses exactly this global
  (`race:Clasp::Asp::trueAtom_g`) and the similar write to `config_def_s`
  (`race:Clasp::ReduceParams::prepare`) for `cargo xtask sanitize`.
- **Upstream tracker:** not reported.
- **Status:** documented as benign. It is formally a data race in C++, and worth
  reporting upstream.
- **clingo 6 (wip-20):** Fixed (static). `trueAtom_g` no longer exists in the bundled clasp.

### U12. clasp's termination flag is not atomic

- **What:** `SequentialSolve::term_` is a `volatile int`, written by an interrupting
  thread and read by the solver thread. With several solver threads, an interrupt
  also resets `ParallelSolve`'s `syncT` timer, a plain struct of doubles, while the
  solver threads start and lap it.
- **Evidence:** ThreadSanitizer; the timer race in
  `interrupt_races` under `cargo xtask sanitize` (`Timer::reset` in
  `ParallelSolve::SharedData::postMessage` from `doInterrupt`, against
  `Timer::start` in `ParallelSolve::beginSolve`).
- **clingox:** the `SAFETY:` comment on `ControlPtr` (`raw/interrupt.rs`) states
  both. Every clingo binding shares these races. `xtask/tsan-suppressions.txt`
  suppresses them as `race:Clasp::SequentialSolve::doInterrupt` and
  `race:Clasp::mt::ParallelSolve::doInterrupt`.
- **Upstream tracker:** not reported.
- **Status:** documented.
- **clingo 6 (wip-20):** Still present at a different site (run under ThreadSanitizer: 3 warnings). Issue draft ready.

### U19. clasp's registry of statistic types is not thread-safe

- **What:** clasp registers each kind of statistics object in the global vector
  `StatisticObject::types_s` the first time it is used, from function-local statics
  in `registerValue`, `registerMap` and `registerArray` (one per template instance).
  `registerType` appends without a lock, so two threads that first use two different
  kinds at once append at the same time, and `tid()` on another thread indexes the
  vector while it may be reallocating: a data race and a possible use-after-free.
- **Evidence:** `clasp/clasp/statistics.h:150-153` (`registerType`),
  `clasp/src/statistics.cpp:41, 56` (`types_s`, `tid()`). ThreadSanitizer reported it
  in `api_async` in 3 of 10 runs, and in 20 of 20 child processes
  of `clingox/tests/patch_u19_statistics_registry.rs` on the unpatched build.
- **User statistics:** `clingo_statistics_map_add_subkey` and
  `clingo_statistics_array_push`, which `MutableStatistics::add_map_key` and
  `push_array` wrap for user-defined entries added from `on_statistics`, register
  their new entry's kind through this exact same `registerValue`/`registerMap`/
  `registerArray` path, patched or not. The text above is about
  clasp's own statistic kinds, registered implicitly by clasp itself; a program
  that adds its own kinds from several solver threads, or from `on_statistics` on
  several controls at once, hits the identical race, now for kinds the
  application chose rather than clasp. `clingox/tests/api_statistics_writing.rs`
  registers user statistics from 1, 2 and 8 solver threads and from several
  controls at once under `cargo xtask sanitize`.
- **Impact:** any program that solves on several controls, or with several solver
  threads, from different threads at once, early in the process, before every kind
  of statistic has been registered. A crash is rare, since the vector reallocates
  only a few times, but possible.
- **clingox:** patched
  (`clingox-sys/patches/U19-statistics-type-registry.patch`). The registry becomes a
  fixed array of 1024 entries (clasp has a few dozen kinds; ids are 16 bits);
  `registerType` takes a mutex in threaded builds and aborts with a message on
  overflow; `tid()` reads a slot without a lock, which is sound because a reader only
  holds ids returned after their slot was written, and the thread-safe
  initialization of the statics that hold them orders the write before the read. No
  ThreadSanitizer suppression covers it. `patch_u19_statistics_registry.rs` proves it
  under `cargo xtask sanitize`.
- **Threads-off builds:** the mutex is still guarded by
  `#if CLASP_HAS_THREADS`, kept on purpose rather than dropped. With the `threads`
  feature off, `clingox-sys/build.rs` sets `CLASP_BUILD_WITH_THREADS=OFF`, and
  clasp's own `CMakeLists.txt` then sets `CMAKE_CXX_STANDARD` to 98 for clasp's own
  targets (clingox-sys does not override it); a threads-off
  build compiles `clasp/src/statistics.cpp` with `-std=gnu++98`, under which
  `<mutex>` does not compile with this toolchain. So the pre-patch race in
  `registerType` remains in that one configuration: two threads that call into
  clingox concurrently before every statistic kind has registered can still race,
  even though nothing in this build uses clasp's own threads: the race is in
  clasp's global registry, and any two `Control`s the embedding program happens to
  run on different OS threads at the same time reach it. Documented in the guide's
  known issues.
- **Upstream tracker:** not reported; no issue in potassco/clasp or potassco/clingo
  mentions the registry, `registerType` or a statistics race (searched 2026-09-27).
- **Status:** patched. Vendored builds only; a system library keeps the bug.
  Even a vendored, patched build keeps the race with the `threads` feature off; see
  above.
- **Remove when:** the clingo release that clingox binds guards `registerType` with a
  lock (or otherwise makes the type registry thread-safe); a fix is not reported
  upstream yet.
- **clingo 6 (wip-20):** Fixed (run under ThreadSanitizer). Eight threads, each with its own control, gave 0 warnings; the registry no longer exists.

### U20. One reader of clasp's shared optimum skips the generation check

- **What:** in a control with several solver threads, clasp shares the best bound
  found so far through `SharedMinimizeData`. It keeps two buffers: `setOptimum`
  writes the inactive one under a lock, then bumps the atomic generation counter
  `gCount_`. Most readers use this like a seqlock: they read the generation, read
  the bound, and retry if the generation moved. `UncoreMinimize::initLevel` reads
  `upper(level)` level by level with no such check, so in principle the bounds of
  different levels can come from different generations.
- **Evidence:**
  - The retrying readers are in `clasp/src/minimize_constraint.cpp`: `DefaultMinimize`
    (around lines 501 to 553) and `UncoreMinimize::integrate` and `valid` (around
    831 to 835 and 1064 to 1069).
  - Readers without the check: `UncoreMinimize::initLevel` (around 870 and 880),
    and, less important, `init` (765) and the `maxBound` path of `valid` (1059).
  - ThreadSanitizer flagged `initLevel` in every child process of an early version
    of the U19 test (4 controls, 8 threads, optimisation).
- **Impact:** only multi-threaded optimisation with the core-guided strategies
  (`--opt-strategy=usc`). The value read steers which level is optimised next and
  the fixing weight, so a mixed-generation read could, in principle, fix a level
  wrongly. Each value is one aligned 8-byte word, so a single read does not tear on
  x86-64 or arm64. No wrong result has been observed:
  - the child processes above passed their checks;
  - a differential run compared
    multi-threaded `usc`, `usc,oll`, `usc,k` and `bb` (4 and 8 threads) with a
    single-threaded reference on random three-level weighted instances. It found 0
    mismatches in 1,968 runs (1,476 with a core-guided strategy), in optimum,
    cost and optimality, on 2026-09-27.
- **clingox:** not patched. A mismatch would mean patching `initLevel` to retry like
  the other readers; the record is clean, so a ThreadSanitizer suppression scoped to
  the racing readers is used instead, together with a multi-threaded optimisation
  test. A clean record does not prove the read harmless. If a wrong optimum is ever
  seen with several threads and `usc`, this is the first suspect; the workaround is
  `--opt-strategy=bb` or one thread.

  `clingox/tests/threads_optimisation.rs` repeats the differential check (4 and 8
  solver threads, `--opt-strategy=bb` and `usc`, a few multi-level weighted set
  cover programs, each compared against a single-threaded reference, many times).
  Its three smaller programs never reached `initLevel` racing in 20 runs; its largest
  program (5 priority levels, so `usc` moves through several levels per solve,
  calling `initLevel` more than once) reached it in roughly half of a handful of
  quick repeated runs. With the `initLevel` suppression line removed and only that
  program run under ThreadSanitizer, the race is reported in
  `Clasp::UncoreMinimize::initLevel` (`minimize_constraint.cpp:880`) in some but not
  all runs, which is expected of a timing-dependent race. So the suppression is not
  vacuous, and the differential record above still holds: no wrong optimum, only the
  race ThreadSanitizer already expects.

  The scope is wider than `initLevel`. The test's programs have real conflicts, so
  solver threads also race on shared state beyond it:
  - `DefaultMinimize::integrateBound` (`clasp/src/minimize_constraint.cpp`):
    the same seqlock-style read of `SharedMinimizeData` that `initLevel` uses,
    just from the retrying side (`updateBounds`). ThreadSanitizer flags a
    correctly-retrying read exactly like a non-retrying one; it has no notion
    of the generation check that makes a torn read here harmless.
  - `UncoreMinimize::valid`, its "maxBound" path (`minimize_constraint.cpp:1059`):
    the other unchecked reader, found in the same evidence as `initLevel`. It
    needed the largest of the test's programs (5 priority levels) to show up,
    which is also the program that reliably reaches `initLevel` itself.
  - `Enumerator::optimize` and `Enumerator::commitUnsat`
    (`clasp/src/enumerator.cpp`): `ParallelSolve::commitUnsat` only takes the
    model lock when `Enumerator::unsatType()` reports `unsat_sync`
    (`clasp/clasp/enumerator.h`), and the only `Enumerator` this clasp ships
    reports `unsat_cont` during optimisation, never `unsat_sync`. So these
    calls run unlocked across solver threads whenever a branch is proven
    infeasible during multi-threaded optimisation, which needs real conflicts
    to happen at all.

  All four are suppressed in `xtask/tsan-suppressions.txt` alongside
  `initLevel`, on the same reasoning: the differential record (including the
  test's many repeated runs) shows no wrong optimum or cost (the models
  themselves were not compared), so these are accepted, watched risks rather
  than patched. If a wrong result is ever seen, any of the five is a suspect, not
  only `initLevel`.
- **Upstream tracker:** not reported; no issue in potassco/clasp mentions `UncoreMinimize` or a race (searched 2026-09-27).
- **Status:** recorded, not patched. The suppressions and the test are in the tree;
  the accepted risk covers the five sites listed above.
- **clingo 6 (wip-20):** Still present (static). `UncoreMinimize::initLevel` still reads the shared optimum unchecked. Issue draft ready (source reading only).

### U21. A clause-count statistic in clasp's shared implication graph is not atomic

- **What:** `ShortImplicationsGraph::add`, called when a solver thread learns a
  binary or ternary clause and shares it, increments a plain `uint32&` counter
  (`bin_[learnt]` or `tern_[learnt]`) with no lock and no atomic, while several
  solver threads call it at once. The actual list the clause is added to is
  designed for this: in a threaded build its shared blocks use
  `Clasp::mt::atomic<uint32>` and `Clasp::mt::atomic<Block*>`
  (`clasp/clasp/shared_context.h`, the `Block` type). The plain increment
  looks like an oversight next to that design, not an intentional lock-free
  scheme.
- **Evidence:** found by `clingox/tests/threads_optimisation.rs` (U20):
  ThreadSanitizer reported a 4-byte write race at the same address from two
  solver threads, both in `ShortImplicationsGraph::add`
  (`clasp/src/shared_context.cpp:252`), with 4 and 8 solver threads on the
  weighted set cover programs. Existing thread tests use programs too small to
  make two solver threads learn and share a short clause at the same instant,
  which is why this had not shown up before.
- **Impact:** `numBinary()` and `numTernary()` (and `numLearnt()`, their sum)
  can undercount by a lost increment when two threads race. They feed one
  boolean heuristic check (`heuristics.cpp:39`, "are there any binary
  clauses") and one size report (`solver.cpp:382`); a rare undercount does not
  change the boolean unless the true count is exactly 1, and otherwise only
  skews a size figure. The clause itself is still added correctly, through the
  atomic-based `Block`; only its own count of itself can be lost.
- **clingox:** no code change. `xtask/tsan-suppressions.txt` suppresses
  exactly this function (`race:Clasp::ShortImplicationsGraph::add`), following
  the same reasoning as U11: a bounded, low-consequence miscount, not a
  correctness hazard, in the same relationship to clasp's actual (atomic)
  clause-sharing design that U11's benign writes have to `trueAtom_g`.
- **Upstream tracker:** not reported; no issue in potassco/clasp mentions
  `ShortImplicationsGraph` or a clause-count race (searched 2026-09-27).
- **Status:** documented as benign, like U11.
- **clingo 6 (wip-20):** Fixed (static). The shared counters use `mt::fetch_add_atomic`.

### U28. `PropagateControl::add_watch` races the master solver's own assignment word

- **What:** `ClingoPropagator::Control::addWatch` (`clasp/src/clingo.cpp:144`)
  checks `POTASSCO_REQUIRE(!s.sharedContext()->validVar(...) ||
  !s.sharedContext()->eliminated(p.var()), ...)` before adding the watch.
  `SharedContext::eliminated` (`clasp/src/shared_context.cpp:955`) reads the
  *master* solver's own `assign_[v]` word with no lock, while the master
  thread (whichever solver clasp designates thread 0) writes that same word
  through its own, ordinary `Solver::assume` -> `Assignment::assign`
  (`clasp/clasp/solver_types.h:564`) during its own unrelated decision-making.
  ThreadSanitizer reports this as a read/write data race on the same 4-byte
  word (`solver_types.h:522`, `Assignment::valid`).
- **Evidence:** `clingox/tests/propagator_reentrancy.rs`'s `data_race_probe_at_
  two_threads`/`data_race_probe_at_eight_threads`, which call
  `PropagateControl::add_watch`/`has_watch`/`remove_watch` from `propagate` on
  every solver thread of a non-sequential propagator. `data_race_probe_at_
  one_thread` never reproduces it (no second thread to race against), and
  reproduced independently by running the built ThreadSanitizer binary with
  `--test-threads=1` against each of the three tests in isolation (only the
  two- and eight-thread variants trigger it, every run).
- **Impact:** the bits actually read (a variable's elimination mark) are
  fixed by clasp's own preprocessing before the search starts and never
  change again, so the *value* observed is correct in every run tried; this
  is a data race in C++'s own formal sense (an unsynchronised concurrent
  read and write of the same word), not an observed wrong answer.
  **Conditions:** only `PropagateControl::add_watch` (and, by the same
  unlocked `ScopedUnlock` shape, `has_watch`/`remove_watch`, `clasp/src/
  clingo.cpp:138-161`), called from `propagate`/`undo`/`check` of a
  propagator registered with plain `Control::register_propagator`
  (non-sequential) while more than one solver thread is searching.
  `PropagateInit::add_watch` is not affected: `init` never runs
  concurrently with itself or with any solver thread (DESIGN S11), so there is no second thread to
  race against there. `Control::register_propagator_sequential` avoids
  this race entirely: it takes clasp's own `ClingoPropagatorLock` around
  every call into the propagator, serialising every `add_watch`/`has_watch`/
  `remove_watch` call against every other one on the same propagator.
- **clingox:** no code change; `add_watch`/`has_watch`/`remove_watch` are
  thin wrappers with no state or lock of their own, so there is nothing on
  clingox's side to fix. `xtask/tsan-suppressions.txt` suppresses
  `Clasp::SharedContext::eliminated` by function name, the narrowest frame
  `tsan-suppressions.txt`'s own `race:` matching supports (it cannot match a
  single call site or call chain, only a function). This is wider than this
  race alone: `eliminated` has two other call sites in the vendored source
  that are not this hazard --
  `CBConsequences::addLit` (`clasp/src/cb_enumerator.cpp:285`, reached
  during brave/cautious consequence enumeration; only a genuine risk with
  several solver threads, not checked directly here) and
  `ClassicHeuristic::updateVar` (`clasp/src/heuristics.cpp:594`, reached
  only during single-threaded `init`, so not a candidate for a cross-thread
  race at all) -- and a suppression matching by function name would hide a
  real race reached through either path exactly as readily as this one. No
  such race has been observed from either call site in this crate's own
  test suite; if one ever is, this suppression is the first place to widen
  the investigation from, not assume already covers it. Documented on
  `PropagateControl::add_watch`'s own rustdoc and in the propagator guide's
  reentrancy section, alongside the existing "never hold your own lock"
  rule, since a user weighing `register_propagator` against
  `register_propagator_sequential` needs this to make that choice with full
  information, not only a performance one.
- **Upstream tracker:** not reported; no issue in potassco/clasp mentions
  `SharedContext::eliminated` or a race in `ClingoPropagator::Control::
  addWatch` (searched 2026-09-28).
- **Status:** documented as benign, like U11/U21; suppressed for `cargo
  xtask sanitize`.
- **clingo 6 (wip-20):** Not reproduced (run under ThreadSanitizer, one trial, 0 warnings); the source is unchanged. The draft stays source-only.

## 6. Platform and toolchain

### U13. Android's C library lacks `canonicalize_file_name`

- **What:** gringo calls this glibc function whenever `__USE_GNU` is defined, which
  bionic also defines.
- **Evidence:** `libgringo/src/input/nongroundparser.cc:71`.
- **clingox:** a force-included header defines it as `realpath(path, NULL)`, glibc's
  documented equivalent. The vendored source is untouched.
- **Upstream tracker:** not reported. clingo#518 (2024, closed without change) asked to avoid `canonicalize_file_name` for performance, not for Android.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Fixed (static). wip-20 uses `std::filesystem::canonical`.

### U14. No CPU time on WebAssembly

- **What:** `getrusage` is a stub on Emscripten, so `summary.times.cpu` is 0.
- **Upstream tracker:** known: clasp#115 (closed 2026-01-29). The agreed change prints "N/A" and drops the key from JSON output; it is not in 5.8.2.
- **Status:** documented.
- **clingo 6 (wip-20):** Still present (static). `getrusage` is still a stub on Emscripten.

### U15. WebAssembly exceptions and LTO

- **What:** clingo's C++ exceptions work in one module with Rust only when clingo is
  built with `-fwasm-exceptions`. Link-time optimisation breaks exception catching.
- **clingox:** `build.rs` sets the flag and never enables LTO for clingo (RULES §8).
- **Upstream tracker:** not applicable (toolchain configuration).
- **Status:** mitigated.
- **clingo 6 (wip-20):** Unchanged (static). The root CMake file still adds `-fwasm-exceptions` for Emscripten.

### U16. Playwright's WebKit does not run on Fedora

- **What:** Playwright builds WebKit against Ubuntu's library versions.
- **Impact:** Safari's engine is untested until a macOS or Ubuntu run.
- **Upstream tracker:** not applicable (Playwright packaging).
- **Status:** documented (a test-infrastructure gap, not a clingo issue).
- **clingo 6 (wip-20):** Not applicable (Playwright packaging).

## 7. Versioning

### U17. The C API changes in minor releases, without a SOVERSION change

- **What:** clingo 5.7.0 changed a function signature and swapped two enum values.
  The shared library's `SOVERSION` stayed 4 across these releases.
- **Impact:** a binding built for one minor version can corrupt memory with another,
  and the dynamic loader cannot tell.
- **clingox:** the `508.x` version scheme, and build-time and run-time version checks
  (DESIGN §10, S18).
- **Upstream tracker:** not reported as such; clingo's CHANGES.md labels the API breaks.
- **Status:** mitigated.
- **clingo 6 (wip-20):** Still present (static). No SOVERSION anywhere in the CMake files while the whole C API changed. Issue draft ready.

## 8. Fixed upstream in versions clingox requires

These affected older releases and are fixed in 5.8.1, which is one reason clingox
accepts only `>= 5.8.1` (DESIGN §5.1, S18):

- clingo#641: symbol creation was not thread-safe (fixed in 5.8.1).
- clingo#622: `SolveHandle::cancel` right after an async multi-threaded solve could
  segfault (fixed in clasp, released in 5.8.1).

## 9. clingo 6

clingo 6 is in development on the `wip-20` branch and not released (latest release:
5.8.2, 2026-08-14). The maintainers' comments show it changes things clingox depends
on:
- symbols are reference-counted and owned by library objects instead of living in a
  global table (clingo#203);
- the ARM pointer-tagging problems are fixed (U18);
- the API changes, and a migration guide was requested (clingo#593).

Moving to clingo 6 will be a new major line for clingox (`600.x`, DESIGN §10) with
its own release plan, not a routine upgrade.

**When to check:** once the binding was complete, so that every upstream finding is
checked against clingo 6 in one pass, ideally against a released version. Each open
entry is then re-run on 5.8.2 and on clingo 6. Only what still fails on both is
drafted as an upstream issue.

The check ran on 2026-09-29 against `wip-20` for U1 to U49; the method, the results and
the C API differences are in [CLINGO6.md](CLINGO6.md), and every entry above carries its
result in a "clingo 6 (wip-20)" line. The issue drafts are available on request; nothing has been reported upstream yet.

## 10. The testing oracle (pyclingo), not clingox's own concern

pyclingo is used throughout to check clingox's behaviour against a reference, never
shipped by clingox, so a bug specific to it is recorded here only as a note for
whoever reproduces a finding against it, not as an upstream issue clingox tracks or
works around.

- **A closed yield handle's `.get()` segfaults the process with a `replace=True`
  observer registered.** Calling `.get()` on a `SolveHandle` that its
  own `with` block already closed segfaults the Python process when a ground program
  observer was registered with `replace=True`. This is pyclingo's own handle
  bookkeeping, not a clingo C API defect (the C API's `clingo_solve_handle_close` and
  `clingo_solve_handle_get` are unaffected; clingox's own `SolveHandle` has no
  equivalent double-use path, S7). clingox's tests avoid it by not calling
  `.get()` after the `with` block exits. Not reported upstream yet.
- **`StrSequence.__setitem__` calls a function that does not exist (U32).** pyclingo 5.8.2's `clingo/ast.py:858` calls
  `_lib.clingo_str_attribute_set_string_at`, but the C function is
  `clingo_ast_attribute_set_string_at`, so `node.operators[i] = x` (and the same for
  any other string array attribute) raises `AttributeError`. clingox's own
  `Ast::set_string_at` calls the real function. clingox's tests call the
  C function through `_lib` directly. Not reported upstream yet.

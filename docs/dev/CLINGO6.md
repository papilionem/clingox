# clingo 6 check

Whether the upstream issues in [UPSTREAM-ISSUES.md](UPSTREAM-ISSUES.md) (U1 to U49)
still exist in clingo 6, and what a port of clingox to clingo 6 would have to change.
The check ran on 2026-09-29, as planned in UPSTREAM-ISSUES section 9. U50 was found
later and is not covered.

Moving to clingo 6 will be a new major line (`600.x`, DESIGN §10) with its own
release plan. Nothing in this file changes what clingox does today.

**Pinned commit:** potassco/clingo branch `wip-20`,
`58853d07e42e75485eea1ce3ca5ed22360666a75` (2026-09-28, "fix cloudsmith cleanup
script"), submodules initialised. `CLINGO_VERSION` is 6.0.0. clingo 6 is not released.

## Method

wip-20 needs re2c 3.0 or newer to generate its lexer, and it was not installed.
re2c was cloned at tag 3.1, built and installed into a private prefix (nothing
system-wide). clingo was then built with that
prefix on `PATH`: Release, Ninja, `-j8`, ccache, `CLINGO_BUILD_PYTHON=OFF`,
`CLINGO_BUILD_LUA=OFF`, tests and examples off. A second build with ThreadSanitizer ran
U12, U19 and U28.

The repros are C probes linked against the built libclingo, and `.lp` files run through
the `clingo` binary, each run sequentially under a memory cap and a timeout. The probe
names in the table (`rp/probe2.c` and so on) refer to small C programs that are not
part of this repository; they are available on request.

Not run: U11, U20 and U21 (static only), Android ARM64 (U18), a build without threads
(U46), the toolchain items, and U2, U10, U24, U27, U36, U37 and U45. The `unsat` solve
event and U1's `clingo_parse_term`-style path were not exercised. The Python module was
not built.

Status tags: **(run)** reproduced or checked by executing wip-20; **(static)** decided from source only. The Python module was not built (as instructed). A second build with ThreadSanitizer (`build-tsan`) ran U12, U19 and U28. Not run: U11, U20, U21 (static only), Android ARM64 (U18), a no-threads build (U46), the toolchain items, U2, U10, U24, U27, U36, U37, U45. U1's `clingo_parse_term`-style path and the `unsat` solve event were not exercised.

Status tags: **(run)** means reproduced or checked by executing wip-20; **(static)**
means decided from source only.

## Architecture: a rewrite, not an evolution

The tree has no libgringo/libclingo. It is `lib/{core,util,input,ground,output,control,c-api,cxx-api,python-api}` plus `third_party/clasp` (clasp 4.0.0). Numbers are arbitrary precision (`lib/core/src/number.cc`, imath), symbols are reference counted and owned by a per-`clingo_lib_t` store, the AST is immutable and built by a variadic constructor, and errors are thread-local in `lib/c-api/src/lib.cc:11`. Most gringo-specific bugs (U1, U5, U29, U33, U35, U47, U49) disappear with the rewrite. The clasp bugs largely remain, since `third_party/clasp` is the same code base.

## Per-issue results

| U | Status | Evidence (wip-20) |
|---|---|---|
| U1 | FIXED (run) | `1\0` as program text: "operation undefined" info, no crash. `p(X/Y) :- X=-2147483647-1, Y=-1.` gives `p(2147483648)`; both U1 match-site programs run clean. Code: bigint `Number`, `Util::check_div`. The `clingo_parse_term`-style entry point was not exercised. |
| U2 | N/A | No global symbol table with a static destructor; symbols are refcounted (`core/src/symbol.cc:221`, `clingo_symbol_acquire/release`). |
| U3 | N/A | Error state is `thread_local` unconditionally (`c-api/src/lib.cc:11`); no `EMSCRIPTEN` guard. |
| U4 | STILL PRESENT (run) | rp/probe6.c, 5000 async solves of `{a;b}.` with a racing interrupt: 1 run reported unsatisfiable+exhausted, 121 interrupted, 4878 satisfiable. Draft U4. |
| U5 | FIXED (run) | `p(2147483648).` is kept as 2147483648. |
| U6 | STILL PRESENT (run) | Unknown option `--nonesuch` in `clingo_control_new` gives code 2 (logic), message `unknown option: 'nonesuch'`. |
| U7 | FIXED (run) | After a syntax error in `clingo_control_parse_string`, a later add, ground and solve work (rp/probe2.c u7). |
| U8 | STILL PRESENT (run) | `clingo_control_interrupt` before a search ends the next search as interrupted (result 8) (rp/probe2.c u8). Clasp's intended queuing. |
| U9 | NOT REPRODUCED (run) | `solve.models` set to `notanumber` fails with "cannot set value" and the value stays -1 (one option only). |
| U10 | BY DESIGN | Unchanged. |
| U11 | FIXED (static) | `trueAtom_g` no longer exists in `third_party/clasp`. |
| U12 | STILL PRESENT (run, TSan), different site | rp/probe6.c under TSan (300 async solves, interrupt racing the start): 3 warnings. `ClaspFacade::solve` writes the active solve pointer (`clasp_facade.cpp:1218`) while `SolveData::interrupt` reads it (`solving()`), and `SolveStrategy::running()` reads an atomic against the constructor's initialisation. `volatile int term_` is unchanged in source. |
| U13 | FIXED (static) | Uses `std::filesystem::canonical` (`control/include/clingo/control/parse.hh:133`). |
| U14 | STILL PRESENT | `getrusage` in `third_party/clasp/libpotassco/src/platform.cpp:150-155`, stub on Emscripten. |
| U15 | UNCHANGED | Root `CMakeLists.txt` adds `-fwasm-exceptions` for Emscripten. |
| U16 | N/A | Playwright. |
| U17 | STILL PRESENT | No SOVERSION/VERSION properties anywhere in the CMake files, while the whole C API changed. Draft U17. |
| U18 | UNVERIFIED | Needs ARM64 Android. Ints use the upper 32 bits and bigints tag the low 3 bits of a pointer (`number.cc:26-46`); clasp is still 4.0.0 with its own flag bits. Maintainer statement says fixed; not confirmed. |
| U19 | FIXED (run, TSan) | TSan build (`build-tsan`, clang `-fsanitize=thread`): 8 threads each creating a control, solving an optimisation and reading statistics (rp/tp2.c): 0 warnings. The registry no longer exists. |
| U20 | STILL PRESENT | `UncoreMinimize::initLevel` reads `shared_->upper(level_)` unchecked at `src/minimize_constraint.cpp:933,972,982`. Draft U20. |
| U21 | FIXED (static) | Shared learnt counters use `mt::fetch_add_atomic` (`src/shared_context.cpp`, ShortImplicationsGraph::add). |
| U22 | STILL PRESENT for the backend path (run) | `clingo_backend_rule` with head 5e8 allocates 495 MB and 1e9 allocates 983 MB before failing "Atom out of bounds" (rp/probe2.c u22). `assign_external`/`release_external` are gone. Draft U22. |
| U23 | FIXED (run) | After a failed `clingo_control_parse_files` on a missing file, parse_string, ground and solve work (rp/probe2.c u23). |
| U24 | STILL PRESENT (changed shape) | `clingo_theory_base_element_condition` returns a span into `static thread_local LitVec cond`, cleared per call (`c-api/src/base.cc:53-58`). Draft U24. |
| U25 | FIXED for stats and model (run); unsat untested | No process abort: a false return from `model` (sync and 4-thread async) and from `stats` gives an ordinary error at `solve`/`solve_handle_get` (rp/probe.c). The `unsat` callback was never invoked by the optimisation program used, so it is untested; the adapter still does not catch exceptions there (`solver.cc:~632`). |
| U26 | NOT REPRODUCED (run) | 4-thread async solve, model callback fails, then a further ground: no valgrind error, control stays usable (rp/probe.c `model_false 4 1`). release_external does not exist any more, so the exact 5.8.2 sequence cannot be run. |
| U27 | N/A | `aspif_theory_` does not exist; parser rewritten. |
| U28 | NOT REPRODUCED (run, TSan); source unchanged | 1 control, 4 solver threads, a propagator whose `propagate` calls `has_watch`/`add_watch` on 12 frozen literals (rp/tp.c): 0 warnings, result 5. Only one trial and the watches are added once, so the racing read in `SharedContext::eliminated` may simply not be reached; the code at `clingo.cpp:383-388` is unchanged. Draft U28 stays source-only. |
| U29 | N/A | AST is immutable; there are no setters, so no cycles. |
| U30 | FIXED (run) | A position with line 2**40 prints as `f:1099511627776:1-1`. |
| U31 | STILL PRESENT (run) | Literal sign 7 and -1 accepted, print as `a` (rp/probe4.c). Draft U31. |
| U32 | N/A | pyclingo bug, out of scope. |
| U33 | N/A | The array editors are gone. |
| U34 | STILL PRESENT (run) | `p(f(f(...)))`: depth 5000 solves, depth 20000, 50000 and 200000 die with SIGSEGV on an 8 MiB stack, in the normal solve path. Draft U34. |
| U35 | FIXED (run) | The three crashing programs (three `#external` type terms: X\2, X**2, abs of X) ground and solve with no crash. |
| U36 | N/A | `clingo_ast_unpool` replaced by `clingo_ast_rewrite` with a rewrite context; semantics of the new flags UNVERIFIED. |
| U37 | CHANGED | Builder replaced by `clingo_program_new/add/free`; misuse behaviour UNVERIFIED. |
| U38 | CHANGED (run) | SIGTERM handler is still replaced by `clingo_main` and not restored. A later SIGTERM no longer segfaults: it is swallowed silently ("survived SIGTERM"), so the process cannot be stopped by signal. Draft U38. |
| U39 | FIXED (run) | A `validate_options` callback returning false gives `clingo_main` code 65 and the callback's message (rp/probe.c main1). |
| U40 | CHANGED (run) | No segfault. Without `main`, `clingo_model_symbols` in `print_model` fails with code 1 "not in solving mode"; with a `main` it works (rp/probe3.c). Draft U40. |
| U41 | STILL PRESENT for NULL members (run) | A script with NULL `callable`/`main` crashes (core dump) in ground (rp/probe2.c u41). The registry is per lib now. Draft U41. |
| U42 | STILL PRESENT (run) | `-t 3`: offsets 3 and 4 succeed, 100 fails, 2**40 returns the key of element 0 (rp/probe.c cfg). Draft U42. |
| U43 | STILL PRESENT, changed (run) | `100% sure and 50%` prints `100 sure and 50` in `--help=3`. Draft U43. |
| U44 | FIXED (run) | A failing `#script` block discards the remainder: after the failed add, `add("z.")` gives the model `{z}` only (rp/probe5.c). |
| U45 | FIXED (static) | Scripts are per lib; `Scripts::main` asks `callable("main")` of that lib's scripts (`control/src/solver.cc:1019-1024`), no process-wide "executed" flag. |
| U46 | FIXED (static) | No `CLASP_HAS_THREADS` guard; stats/finish are delivered by `SolveEventHandlerAdapter::onEvent` (`control/src/solver.cc:600-616`). |
| U47 | N/A | `clingo_ast_t` is a handle over `shared_ptr` (`c-api/src/ast.cc:37`); no 32-bit counter. |
| U48 | FIXED (run) | `#project p/100000000.` and `p/2147483647.` in text run in 10 MB; text `p/4294967295` is a parse error; AST arity -1 is accepted by the constructor and printed as `p/-1`. |
| U49 | FIXED (run) | `p(2147483646..2147483647).` gives both atoms and finishes. |

## C API differences for a port of clingox

The C API is replaced, not adjusted (255 old functions, 267 new, 146 old names gone, 158 new). `clingo.h` now includes `app, ast, base, control, core, model, symbol`.

- **New root object `clingo_lib_t`** (`clingo_lib_new/acquire/release/free/report`). Symbols, strings, scripts, log live in it. Most constructors take `lib`. Locations and positions are opaque, heap allocated (`clingo_location_new/copy/free`, `clingo_position_*`).
- **Symbols** are refcounted: `clingo_symbol_acquire/release` appear; `clingo_symbol_is_equal_to/less_than/is_negative` become `equal/compare`; the signature API is gone; `create_tuple`, `create_number_str` (bigint) added. Symbol payload is no longer a global table.
- **Errors:** `clingo_error_code/message/string` replaced by `clingo_get_error`, `clingo_clear_error`, `clingo_result_string`, `clingo_message_string`. Result codes gain `bad_alloc`, `invalid`, `range`.
- **Control:** `clingo_control_add/load/cleanup/assign_external/release_external/get_const/has_const/configuration/statistics/symbolic_atoms/theory_atoms/register_backend/...` are gone. Now `parse_string`, `parse_files`, `start_ground` + `ground_handle_*` (async grounding), `set_parts/get_parts`, `const_map`, `mode`, `main`, `join`, `discard`, `buffer`, `write_aspif`, `observe`, `profile`, `config`, `stats`, `base`. Externals go through `clingo_backend_external`; there is no assign/release.
- **Symbolic and theory atoms:** replaced by `clingo_base_t` (`clingo_base_atoms_*`, `atom_base_*`, `term_base_*`, `clingo_base_theory`, `clingo_theory_base_*`). No iterator API.
- **AST:** no setters, no acquire/release refcount, no `attribute_set_*`/array editors; `clingo_ast_construct` (variadic), `copy`, `free`, `array_free`, `compare`, typed array getters, `clingo_ast_rewrite` with a rewrite context (replaces `unpool` and the builder), `clingo_program_new/add/free` replaces the program builder.
- **Solve:** `clingo_solve_event_handler_t` now has `model, unsat, stats, finish, free`; `stats` takes `clingo_stats_t`; `finish` returns void. Solve mode gains `lock`.
- **Config/statistics:** `clingo_configuration_*` renamed `clingo_config_*` (adds `add`, `to_string`), `clingo_statistics_*` renamed `clingo_stats_*` (adds `to_string`). `clingo_options_set_default_value` is new.
- **Propagators:** `clingo_propagate_init_*` renamed and reorganised (`solver_literal`, `freeze_literal`, `base`, `library`, `control`, `add_minimize`); `init_add_clause/add_literal/add_watch/propagate/assignment` removed; `clingo_assignment_thread_id` and `propagate_control_add_weight_constraint` added.
- **Scripts:** `clingo_script_register(lib, script, data)`; the script struct gains `name`, `version`, and passes `lib` to `call` and `main`.
- **Versioning:** `CLINGO_VERSION_MAJOR 6`; no SOVERSION (U17 stays).
- **Build:** C++20, cmake >= 3.22.1, re2c >= 3.0 required.

## Issue drafts

Fourteen issue drafts exist. Nine were confirmed by running wip-20: U4, U22, U31, U34,
U38, U40, U41, U42 and U43. U12 is backed by a ThreadSanitizer run. U17, U20, U24 and
U28 come from reading the source only.

The drafts are not part of this repository and are available on request. Nothing has
been reported upstream yet.

## Not done

The items listed under Method as not run. U50 was found after this check. The U6 row
records that unknown options are still logic errors on wip-20 (code 2).

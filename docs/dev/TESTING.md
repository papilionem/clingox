# Testing

This document describes the test suite: what each part proves, where it lives, how
to run it, and the rules every test follows. The goal is that "clingox works with
clingo 5.8.2" is a claim backed by tests anyone can run, not a belief.

## 1. What the suite has to prove

| Claim | Proven by |
|---|---|
| clingox gives the same answers as clingo | conformance tests ported from upstream |
| Our Rust declarations match `clingo.h` | the ABI check (`systest`) |
| Misuse cannot cause undefined behaviour | compile-fail tests, panic tests, sanitizers |
| Every wrapper works, including its failure path | unit and integration tests |
| Results do not change between runs | the determinism test |
| It runs on each target | the same suite, executed on Linux, Android and WASM |
| The linked library is the version we claim | the version-gate test |

## 2. Layout

```
clingox-sys/tests/
  version.rs                  the linked library is the version we bind
  raw_api.rs                  smoke tests of the raw C API on every target
clingox/src/**                unit tests (#[cfg(test)]), pure Rust, Miri-clean
clingox/tests/
  api_*.rs                    integration tests, one file per API area
  conformance/
    examples_c/*.rs           ports of upstream examples/c
    libclingo/*.rs            ports of libclingo/tests SECTIONs
    pyclingo/*.rs             ports of libpyclingo tests
    lp_fixtures.rs            data-driven runner over fixtures/lp
  fixtures/lp/*.lp, *.sol     copied from app/clingo/tests/lp (not published)
  ui/*.rs, *.stderr           compile-fail cases (trybuild)
  panic_safety.rs             one test per callback kind
  threads.rs                  thread-placement, interrupt and propagator tests
  determinism.rs
systest/                      ctest ABI check against clingo.h
```

Fixtures and conformance data stay out of the published crates (the package
`include` lists exclude `tests/fixtures`).

## 3. Rules for every test

1. **A test states what it proves.** The test name says the behaviour
   (`ground_fails_on_undefined_function`), not the function called. Test files are
   named for what they cover (`raw_api.rs`, `api_solve.rs`), never for the change
   or task that produced them: a task is history, the test stays.
2. **Every wrapper has a success test, and a failure test if the C call can fail.**
   Failure tests assert the `ErrorKind`, not the message text, unless the message
   is part of the contract.
3. **Acceptance tests are written before the implementation** and seen failing for
   the right reason (RULES §1).
4. **Negative controls.** After a test passes, break the code it covers on purpose
   (remove the check, swap the argument, skip the copy) and confirm the test fails.
   Record what was broken and that the test caught it in the PR description. A test
   that survives its negative control is strengthened before merge.
5. **Tests do not depend on model order** unless they set a deterministic
   configuration explicitly. Compare sets, or use `solve_all`, which sorts.
6. **No sleeps for synchronisation.** Thread tests use channels, barriers or
   `InterruptHandle` results. A test that needs timing uses generous bounds and
   says why.
7. **A flaky test is a bug.** It is fixed or quarantined with an issue the same day,
   never retried until green.
8. **Whoever implements a change does not edit the acceptance tests that judge it.**
   If a test is wrong, say so in the PR and let the author of the contract change it.

## 4. Conformance ports

Porting upstream tests is how we show clingox behaves like clingo. Each ported file
starts with its origin so a clingo upgrade can diff upstream's test changes against
our ports:

```rust
//! Ported from potassco/clingo v5.8.2
//! Source: libclingo/tests/clingo.cc, TEST_CASE "solving", SECTION "solve"
//! Differences: none.
```

- One upstream `SECTION`, example or Python test maps to one Rust test with a
  matching name.
- The port keeps upstream's program text and expected results exactly. Where Rust
  needs a different shape (for example, an iterator instead of a callback), the
  header's "Differences" line says so.
- An upstream test that cannot be ported (Python-specific behaviour, embedded
  scripting) is listed in `conformance/NOT_PORTED.md` with the reason.
- The `.lp`/`.sol` fixtures run through one data-driven test. It enumerates all
  models and compares the sorted model sets, normalised the way upstream's
  `app/clingo/tests/run.py` does.

Upstream counts at v5.8.2, taken with one fixed unit per source
(one `.lp` file, one `.c` file, one `TEST_CASE`/`SECTION` at any nesting
depth, one `def test_...` method) and checked automatically by `cargo xtask
conformance-count` (wired into `cargo xtask check`): 13 `examples/c`
programs, 105 `libclingo/tests` sections (`clingo.cc`, `symbol.cc`,
`astv2.cc`, `propagator.cc`, `variant.cc` together), 72 pyclingo tests and
114 `.lp` fixtures (8 `lp/` + 51 `python/` + 55 `lua/`). See
`clingox/tests/conformance/NOT_PORTED.md`'s section on the counting unit for
how each source is counted.

The conformance suite (`tests/conformance_*.rs`) ports upstream tests as Rust
tests. Each file covers one source: `conformance_fixtures.rs` for `.lp`
fixtures, `conformance_examples.rs` for C examples, `conformance_libclingo.rs`
for C++ sections, `conformance_pyclingo.rs` for pyclingo methods, and
`conformance_scripts.rs` for embedded-script fixtures. The shared helper
module `tests/conformance/mod.rs` provides the normaliser that matches
`app/clingo/tests/run.py::normalize()`.

Upstream items that are not ported are listed in
`tests/conformance/NOT_PORTED.md` with the reason.

## 5. Safety tests

- **Compile-fail (trybuild).** Each case in `tests/ui` is a program that must not
  compile, with its expected error in a `.stderr` file. Cases:
  - using a model after the search moved on;
  - a solve handle outliving its control;
  - iterating symbolic atoms across `ground`;
  - a propagator that is not `Sync`;
  - moving a `Control` to another thread.

  The `.stderr` files are regenerated only on the pinned toolchain
  (`TRYBUILD=overwrite`), and the diff is reviewed. The pin is the release in
  `xtask/compile-fail-toolchain`, installed with the `rust-src` component: rustc
  words its errors differently from release to release, and quotes the standard
  library only when it has the source. `cargo xtask test compile-fail` checks both
  and runs the tests. Setting `CLINGOX_SKIP_COMPILE_FAIL` (to any value) makes the
  two trybuild tests return at once, saying so; CI sets it in every job except the
  one that runs `test compile-fail`, and `test compile-fail` clears it.
- **Panic safety.** For each callback kind (ground function, model loop, logger,
  solve events, observer, propagator, AST parse), a test panics inside the callback
  and checks three things: the panic or error reaches the caller, the process did
  not abort, and the `Control` still drops cleanly.
- **Threads.** A model loop with 4 solver threads captures a `!Send` value and must
  never leave the caller's thread. Interrupt tests cover a running solve, an idle
  control, a grounding call and a racing drop. Propagator tests force backjumps with
  1, 2 and 8 threads.
- **Sanitizers.** `cargo xtask sanitize` runs every test of `clingox` and
  `clingox-sys`, except `compile_fail` and `user_docs`, with AddressSanitizer and
  LeakSanitizer, then `api_threads`, `patch_u19_statistics_registry`,
  `interrupt_races`, `api_interrupt`, `api_async` and `api_solve_options` with
  ThreadSanitizer. Rust is built by nightly with
  `-Zsanitizer` for `x86_64-unknown-linux-gnu` (ThreadSanitizer adds `-Zbuild-std`),
  and clingo by clang with the same `-fsanitize` flag, through a CMake toolchain file
  that makes CMake's compiler checks build static libraries: a sanitized test program
  links only with the runtime rustc brings. Each sanitizer has its own target
  directory under `target/`. ThreadSanitizer reads `xtask/tsan-suppressions.txt`,
  which names only clasp's known races (UPSTREAM-ISSUES U11 and U12, DESIGN S12),
  never clingox or Rust code. It needs `llvm-symbolizer` on `PATH` (the `llvm`
  package): without it ThreadSanitizer cannot name the global a race touches, and
  the suppression of the race on `trueAtom_g` does not apply. Any report fails the command. It runs on a Linux
  x86_64 host and installs nightly with `rust-src` if it is missing
  (`cargo xtask setup sanitize`).

## 6. Determinism

`determinism.rs` runs the conformance fixtures twice with the default options and
compares the results exactly. The defaults (one solver thread, a fixed seed) are what
make this pass. A change that breaks it breaks a documented promise.

## 7. Running the suite

All targets are driven through `xtask`, so the commands used locally are the ones CI
runs.

| Command | What it runs |
|---|---|
| `cargo xtask check` | fmt, clippy, doc, `cargo deny`, the stale-bindings check, the conformance recount (below), the unsafe budget, the semver check once its baseline tag exists, the guide build, and the doctests of rustdoc, the guide and the README |
| `cargo xtask conformance-count` | recounts the conformance inventory from the submodule and checks it against `tests/conformance/NOT_PORTED.md`; `--old-unit` prints the totals under the earlier counting unit for comparison, without failing |
| `cargo xtask test linux` | the full suite on the host, including systest and trybuild (unless `CLINGOX_SKIP_COMPILE_FAIL` is set) |
| `cargo xtask test compile-fail` | the trybuild tests, after checking that rustc is the release in `xtask/compile-fail-toolchain` and has `rust-src` |
| `cargo xtask test android` | builds for `x86_64-linux-android` with the NDK's own clang wrappers (cargo-ndk hides the test executables' paths), pushes the test binaries to the running emulator with `adb`, and runs them |
| `cargo xtask test wasm` | builds for `wasm32-unknown-emscripten` and runs the suite under Node.js |
| `cargo xtask test wasm --browser <engine>` | the same tests in a headless browser through Playwright: `chromium`, `firefox`, `webkit`, or `all` |
| `cargo xtask test all` | `test linux`, `test android` and `test wasm` at the same time, each exactly the command it is on its own; see section 10 |
| `cargo xtask setup browser` | installs the Playwright tooling and browsers |
| `cargo xtask sanitize` | ASan+LSan, then TSan on the thread tests (nightly, see section 5) |
| `cargo xtask miri` | unit tests and trampolines under Miri |
| `cargo xtask semver [--baseline-rev <rev>]` | `cargo semver-checks` for the three crates against `<rev>` or the baseline in `xtask/semver-baseline` (`latest`: the newest release tag; `cargo xtask check` skips the step while the repository has none), with the release type fixed to minor so that every breaking change is reported. Findings that are breaking for the tool but source-compatible by design are listed one by one in `xtask/semver-allow` (lint name, item path, reason) under a `baseline <tag>` line; against that release the step fails on an unlisted finding and on a listed entry that no longer occurs, and against any other release the entries are ignored |

**Android** needs an emulator running (`emulator -avd <name>`), with KVM for speed.
The x86_64 emulator image runs the tests; `aarch64-linux-android` is built but not
run locally.

**WASM** needs the pinned Emscripten SDK (the version is in `xtask/emsdk-version`);
`cargo xtask setup wasm` installs it. Under Node.js, `cargo xtask test wasm` also
fails when any line of the output reports `unsupported syscall`: Emscripten's debug
libc prints that for a syscall it only stubs with made-up values. clingo's one such
call, clasp's `getrusage`, is patched out (U14), so a report is a new call to look
at, not noise.

**Browsers.** Each test executable runs in a fresh page. The runner reports a test
as failed on a non-zero exit, an abort, an uncaught error, a failed load, a crash,
or no result within the time limit (`--timeout`, default 120 s). Chromium and
Firefox run on the development machine. **Playwright's WebKit does not launch on
Fedora** (it is built against Ubuntu's library versions), so the WebKit run fails
there with a clear message; CI runs it on Ubuntu (`test (WASM in webkit)` in
`ci.yml`). That is Playwright's WebKit engine, not Safari, which is untested.

## 8. Documentation tests

Rustdoc examples run with `cargo test --doc`. The guide's and the README's Rust code
blocks run through a test module that includes each Markdown file with
`#[doc = include_str!(...)]`, so a stale example fails the suite. `mdbook build
guide` is part of `cargo xtask check`.

## 9. Coverage

`docs/dev/COVERAGE.md` has one row per clingo C function: its wrapper, its test, whether
its `bool` return is a success flag or a value, and notes. A function without a test
is not wrapped (RULES §3). `cargo xtask coverage` checks that every function in
`clingo.h` has a row, and that every listed test exists.

## 10. Build speed

The suite above is unchanged by anything in this section: same tests, same flags,
same failure on a real failure (verified below). What changed is how fast a clean
checkout gets there, on a 24-core, 32 GB machine with ccache 4.12 and cargo-nextest
0.9.132 installed.

### ccache reaches the C++ build

`clingox-sys/build.rs` builds clingo's C++ through the `cmake` crate, which asks the
`cc` crate for the compiler. On this machine `CC`/`CXX` are set to the absolute paths
of the real compilers (`/usr/bin/clang`, `/usr/bin/clang++`), which is exactly what
lets a build reproduce byte-for-byte, but it also means the `cmake`/`cc` crates never
see the `ccache`-wrapped `gcc`/`g++`/`clang` symlinks that `/usr/lib64/ccache` puts
earlier on `PATH`: `CC=/usr/bin/clang` bypasses that symlink entirely. Measured before
any change: a clean `cargo build -p clingox-sys` (vendored, host target) took 27.51 s,
and `ccache -s` afterwards showed only 2 calls total, both uncacheable: none of the
82 object files clingo's CMake build compiles went through ccache at all.

The fix (`clingox-sys/build.rs`, `forward_compiler_launcher`) sets
`CMAKE_C_COMPILER_LAUNCHER`/`CMAKE_CXX_COMPILER_LAUNCHER` explicitly, which wraps
whatever `CMAKE_<LANG>_COMPILER` resolves to rather than depending on a `PATH` symlink
in front of it. sccache is still preferred automatically when it already wraps
`rustc` (`RUSTC_WRAPPER`, unchanged).

ccache is **opt-in only**, unlike an earlier version of this fix: `clingox-sys` is a
published crate, so its build script must not start reaching outside the build just
because `ccache` happens to be on a user's `PATH`. `CLINGOX_SYS_CCACHE=1` or `=auto`
uses whatever `ccache` is found on `PATH`; any other non-empty value is read as an
explicit path to the `ccache` binary to use; either fails loudly if the requested
`ccache` cannot be used, since it was asked for by name. `0`, empty or unset (the
default) never reaches for ccache, so a plain `cargo build` on a machine that happens
to have `ccache` installed behaves exactly as it did before this section.

`cargo xtask` sets `CLINGOX_SYS_CCACHE=auto` for its own builds when `ccache` is
found, so clingox's own workflow still gets the speedup below without a downstream
build being affected. This lives in `xtask/src/util.rs`
(`use_ccache_if_found`), called from `cargo()` (reaching `check`, `test
linux`/`android`/`wasm`/`all`, `coverage`, `semver`, `bindgen`) and, for the two
places that shell out to `cargo` through `rustup` instead of `cargo()`, called
directly from `sanitize::sanitized_cargo` and `test::miri`. It only sets the
variable when the environment does not already say something about it, so an
explicit `CLINGOX_SYS_CCACHE=0` (or any other explicit value) always wins.
`coverage` parses the vendored `clingo.h` directly and never builds anything, so
there is nothing for it to inject there.

Measured under an earlier version of this fix, before ccache was made opt-in (an
unset `CLINGOX_SYS_CCACHE` auto-detected `ccache` on `PATH`; see below for the
opt-in version's numbers with `CLINGOX_SYS_CCACHE` genuinely unset), all with
`CARGO_BUILD_JOBS=4`, `ccache -z` before each build and `ccache -s` after:

| Build (`cargo build -p clingox-sys`, vendored) | First build (misses) | Second build, fresh `CARGO_TARGET_DIR` | ccache hit rate on the second build |
|---|---|---|---|
| Host (clang, `x86_64-unknown-linux-gnu`) | 30.47 s | 3.06 s | 82/82 (100%), 78 direct + 4 preprocessed |
| Android NDK clang (`x86_64-linux-android`, API 24) | 40.70 s | 3.61 s | 82/82 (100%) |
| Emscripten (`wasm32-unknown-emscripten`, emcc/em++) | 43.81 s | 3.64 s | 81/81 (100%) |

**Re-verified after making ccache opt-in**, with `CLINGOX_SYS_CCACHE` unset (so
everything below is each command's actual default behaviour, no thumb on the
scale):

- A bare `cargo build -p clingox-sys` in a fresh `CARGO_TARGET_DIR`, with `ccache`
  present on `PATH` but `CLINGOX_SYS_CCACHE` unset: 28.58 s, `ccache -s` shows 0
  cacheable calls, matching the pre-fix, no-caching baseline. This is the control
  that proves the opt-in actually opted out: a downstream user who has never heard
  of `CLINGOX_SYS_CCACHE` gets exactly what they would have gotten before any of
  this work.
- `cargo xtask check` (fmt, clippy x2, doc, doctests, `cargo deny`, conformance,
  the unsafe budget, `semver-checks`, the guide build, `cargo package`) in a fresh
  `CARGO_TARGET_DIR`, again with `CLINGOX_SYS_CCACHE` unset: 40 s, all checks
  passed, and `ccache -s` shows 164/164 cacheable calls hit (100%). This is
  `xtask`'s `use_ccache_if_found` doing its job: nothing in the environment asked
  for ccache, `xtask` asked on its own because it found `ccache` on `PATH`, and the
  whole `check` pipeline (which touches `clingox-sys` several times, through
  `clippy`, `doc`, `test --doc` and `package`) reused the same cached objects.

The second build in each row of the first table uses a brand new `CARGO_TARGET_DIR` (so a fresh `OUT_DIR`
and a freshly re-patched copy of the submodule under it), which is what proves the
cache is shared **across build directories**, not just reused within one: the
scenario that matters when several worktrees or a CI runner's ephemeral workspace
build the same clingo commit. This works because the global ccache
config sets `base_dir` to the parent of every worktree and `hash_dir=false`,
so the absolute `OUT_DIR` path baked into each compile command is normalised away
before hashing; nothing in `clingox-sys` had to change for that part.

Emscripten was the one open question (`compiler_type` in `ccache`'s manual lists
`auto`, `clang`, `gcc`, `msvc`, and others, but no `emscripten` entry): a direct
smoke test (`ccache emcc -c t.c -o t.o` twice) confirmed ccache's `auto` detection
still classifies `emcc` correctly and gets a direct hit on the second call, with no
`CCACHE_COMPILERTYPE` override needed.

`forward_compiler_launcher` runs unconditionally at the end of `build_vendored`, after
the Android and Emscripten toolchain setup, so this reaches every vendored build,
including the ones `xtask` does not build through `cmake::Config` directly:
`cargo xtask sanitize`'s own `CMakeCache.txt` was checked and shows
`CMAKE_C_COMPILER_LAUNCHER`/`CMAKE_CXX_COMPILER_LAUNCHER` set to ccache alongside its
`-fsanitize=address` flags. ccache keys its cache on the full compiler invocation, so
a sanitizer build's objects never collide with a plain build's; that run showed 92
cacheable calls (mostly misses, expected on a cache that had never seen
`-fsanitize=address` before).

### `cargo xtask test all`

`test all` runs `test linux`, `test android` and `test wasm` at the same time, each as
its own OS process running exactly `cargo xtask test <suite>` (so CI parity holds: the
commands `test all` runs are byte-identical to the ones you would run by hand). Output
is captured per suite and printed under a header naming it once that suite finishes,
so the three runs' output cannot interleave. `sanitize` and `miri` are deliberately
left out of `test all`: both instrument the whole dependency tree already and would
fight a concurrent suite for memory rather than overlap productively with it, so they
stay their own, exclusive commands.

Android and WASM each get their own `CARGO_TARGET_DIR` under
`target/parallel-test-all/`; `test linux` keeps the default one. Cargo takes a single
lock over an entire target directory for the duration of a build, so without separate
directories the three suites would serialise on that lock instead of overlapping. This
is also why the C++ caching above had to work across target directories before
running suites concurrently was worth doing at all.

Measured on a clean checkout (`rm -rf target`, no `test all` run before, and for the
sequential column, no earlier run of any of the three either):

| | Sequential (one after another) | `cargo xtask test all` |
|---|---|---|
| linux | 160 s | (concurrent) |
| android | 188 s | (concurrent) |
| wasm | 290 s | (concurrent) |
| **total wall time** | **638 s** | **427–490 s** (two runs) |

That is roughly a 1.3–1.5x speedup, not the ~2.2x a naive `max` of the three would
suggest; the gap is real CPU contention between the suites' own test execution (the
Linux suite alone runs about 1000 tests, several of them multi-threaded), not just
compilation, since all three ran with the same `CARGO_BUILD_JOBS=4` cap. WASM is the
long pole (it runs the whole suite twice, once per profile, under Node.js), so
`test all`'s wall time tracks close to WASM's solo time plus the contention overhead.

**Nothing weakened, shown two ways:**

- `cargo nextest run --workspace` was evaluated for `test linux` per the brief's
  condition ("if it keeps every test, and is faster"). It does not keep every test
  without extra work: `systest`'s `ctest`-generated binary uses `harness = false` and
  does not support nextest's `--list` protocol, so a bare `cargo nextest run
  --workspace` fails outright ("did not end with the string \": test\" or \":
  benchmark\""). Excluding it (`--exclude systest`) and adding back `cargo test -p
  systest` and `cargo test --doc --workspace` to keep parity brought the total to
  roughly the same wall time as plain `cargo test --workspace` (measured on an
  already-built target directory: nextest 104 s + systest ~7 s cold/near-0 s warm +
  doctests ~2.3 s, against `cargo test --workspace`'s 111 s for the same three), because
  one single test (`clingox::async_core_waits::core_waits_for_the_search_to_finish_before_reading_it`,
  ~89.5 s) dominates either way. **Decision: `test linux` keeps `cargo test
  --workspace`**, since nextest is a wash here once parity is restored, for three
  commands instead of one.
- A deliberately failing test (`clingox/tests/zzz_deliberate_failure.rs`, `assert_eq!(1,
  2, ...)`) was added in a throwaway `git worktree` (removed afterwards) and `cargo xtask test all` was run against it. It exited 1 and
  printed `test all: failed: linux, android, wasm`, with each suite's own `FAILED`
  test output intact under its own `===== <suite> =====` header: the same failure
  `test linux`/`test android`/`test wasm` would report on their own, just captured and
  attributed rather than interleaved.

### Final checks (after every change above)

| Command | Result | Time |
|---|---|---|
| `cargo xtask check` | all checks passed | 33 s |
| `cargo xtask test all` | linux, android and wasm all passed | 427–490 s |
| `cargo xtask sanitize` | "the address, leak and thread sanitizers found nothing" | 995 s (~16.6 min) |

`cargo xtask sanitize` was run last and on its own (never alongside `test all` or
another heavy build), per its existing "never in parallel" design.

### What did not get faster, and why

- **`cargo xtask sanitize`'s 995 s is mostly Rust build time under `-Zbuild-std`
  plus the tests' own runtime, not something ccache reaches.** ccache only speeds up
  clingo's C++, which is a small fraction of a sanitizer run; the rest is nightly
  `rustc` recompiling `std` and every crate from scratch for each of the two
  sanitizer passes (address+leak, then thread), which `RUSTC_WRAPPER`/sccache could
  help with but was out of scope here (sccache is not installed on this machine).
  Nothing was attempted here beyond confirming (via `CMakeCache.txt`) that the C++
  side does get ccache.
- **The single ~89.5 s test (`async_core_waits::core_waits_for_the_search_to_finish_before_reading_it`)
  is the long pole of the whole Linux suite**, under both `cargo test` and nextest.
  It is presumably deliberately slow (interrupt-race timing), and neither a faster
  test runner nor better caching touches it; only running it concurrently with other
  suites (which `test all` already does) hides its cost behind theirs.
- **Android and WASM device/runtime execution itself was not sped up**, only their
  build. The emulator boot and `adb push`/`shell` round-trips, and running the whole
  suite twice under Node.js for WASM (once per profile), are unchanged.

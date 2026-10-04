# Feature flags

`clingox` has four Cargo features, all on by default. Two of them, `vendored` and
`threads`, are forwarded to `clingox-sys`, which builds or finds clingo. This page
says what each feature does, what the build needs for it, and what changes when you
turn it off.

| Feature | Default | What it does | Effect on the build |
|---|---|---|---|
| `vendored` | on | builds clingo 5.8.2 from the source shipped in `clingox-sys`, with its patches | needs CMake and a C++ compiler; compiles clingo once per target |
| `threads` | on | builds clasp with threads: more than one solver thread, timeouts and asynchronous solving | links the platform's thread library; no effect on WebAssembly without the `atomics` target feature |
| `derive` | on | `#[derive(ToSymbol)]`, `#[derive(FromSymbol)]` and `sym!` | compiles the proc-macro crate `clingox-derive` and its dependencies |
| `log` | on | forwards clingo's messages to the `log` crate | adds the `log` crate |

To choose your own set, turn the defaults off and list what you want:

```toml
[dependencies]
clingox = { version = "508.2.0-beta.5", default-features = false, features = ["vendored", "derive"] }
```

Cargo features are additive across the whole dependency graph. If another crate in
your build depends on `clingox` with its defaults, `vendored` and `threads` are on for
you too, whatever your own manifest says.

## `vendored`

With `vendored`, the build script of `clingox-sys` copies the clingo source that ships
in the crate, applies clingox's patches to the copy, and builds clingo's five static
libraries with CMake. The patches fix clingo defects listed in
[Known issues](known-issues.md); `clingox_sys::PATCHES` names them.

- **What it needs.** CMake 3.10 or newer and a C and C++ compiler with C++14 support,
  as listed in [Installation](../getting-started/installation.md). It needs no
  network, and no Bison, re2c, Python or Lua.
- **Release mode always.** clingo is compiled optimised even in a debug build of your
  project. In the repository's measurements the first build of `clingox-sys` took
  30 to 44 seconds with four build jobs, depending on the target; later builds reuse
  the result. `CLINGOX_SYS_CCACHE=1` lets ccache cache the C++ compilation.
- **The C++ runtime.** The build links the platform's C++ standard library:
  `libstdc++` on Linux and NetBSD, `libc++` on macOS, iOS, FreeBSD, OpenBSD,
  DragonFly and WebAssembly, `libc++_shared` on Android, and nothing extra with
  MSVC. The variable `CXXSTDLIB` overrides the choice, and an empty value links
  none.

Without `vendored`, `clingox-sys` links a clingo installed on the system instead. It
must be 5.8.1 or newer within 5.8, and the variables `CLINGO_INCLUDE_DIR`,
`CLINGO_LIB_DIR`, `CLINGO_STATIC` and `CLINGO_THREADS` describe it (see
[Installation](../getting-started/installation.md#a-clingo-installed-on-the-system)).
A system clingo gets none of the patches.

Because features are additive, a dependency can turn `vendored` back on. The
environment variable `CLINGOX_SYS_NO_VENDOR=1` overrides the feature and always
links the system clingo.

## `threads`

With `threads`, clasp is built with thread support (`CLASP_BUILD_WITH_THREADS`). On
Unix targets other than Android and WebAssembly the build also links `pthread`, and
it links `libatomic` where clasp's CMake build asks for it.

These need a build with threads:

- more than one solver thread: `ControlBuilder::threads(n)` with `n` above 1, and
  clingo's `--parallel-mode` and the options that go with it;
- a timeout in `SolveOptions` (see [Set a time budget](../how-to/time-budget.md));
- `Control::solve_async` and `solve_async_with_events`.

Without threads, `threads(n)` above 1, a timeout and `solve_async` return
[`ErrorKind::Unsupported`](error-kinds.md#unsupported) before anything is solved, and
a control stays usable. clingo itself has no `--parallel-mode` option in such a
build, so passing it to `Control::with_args` fails when the control is created.
Everything else works.

On `wasm32-unknown-emscripten` the feature takes effect only when the target is
compiled with the `atomics` target feature, which currently needs nightly Rust. The
default WebAssembly build is therefore single-threaded with the feature on. See
[Run in the browser](../how-to/browser.md).

One defect depends on this feature: clasp's registry of statistic kinds is not
thread-safe. The vendored build fixes it only with `threads` on, and a system clingo
never gets the fix, whether it has threads or not. In those builds, controls must not
solve on several threads at once (see
[Known issues](known-issues.md#fixed-in-the-vendored-build-open-with-a-system-clingo)).

For a system clingo, the feature does nothing: `CLINGO_THREADS` says how that library
was built (`1` or unset for threads, `0` for none).

A program can ask at run time whether its clingo has threads.
`clingox_sys::HAS_THREADS` is the answer the build script recorded; it needs
`clingox-sys` as a direct dependency, at the same version as `clingox`. A program that
does not want that dependency can try the call and handle `ErrorKind::Unsupported`
instead, as the [time budget](../how-to/time-budget.md) page shows.

## `derive`

With `derive`, `clingox` re-exports three macros from `clingox-derive`:
`#[derive(ToSymbol)]`, `#[derive(FromSymbol)]` (which also implements `Predicate`
for a struct) and `sym!`. The prelude includes them. They are covered in
[Facts and rules](../tutorial/facts-and-rules.md) and
[Reading results as Rust types](../tutorial/typed-results.md).

Without `derive`, the build skips the proc-macro crate and its dependencies (`syn`,
`quote`, `proc-macro2` and `heck`). The traits `ToSymbol`, `FromSymbol` and
`Predicate` remain, and so do `add_facts`, `Model::atoms`, `Model::shown` and
`clingox::testing`; you implement the traits by hand, reporting a value that does not
convert with `Error::conversion`.

## `log`

With `log`, every message clingo logs goes to the `log` crate under the target
`clingox`, unless the control has its own logger (`ControlBuilder::logger`). Errors
are logged at the level `Error` and everything else at `Warn`, as clingo's own
application ranks them. Nothing appears unless your program installs a `log`
implementation.

Without `log`, a message that no logger receives is dropped. Either way, the messages
of a failed call are attached to its error ([`Error::messages`](error-kinds.md)).

## docs.rs

The API documentation on [docs.rs](https://docs.rs/clingox) is built with `derive`
and `log` for `x86_64-unknown-linux-gnu`. The build script of `clingox-sys` skips the
C++ build there, because rustdoc needs only the committed declarations.

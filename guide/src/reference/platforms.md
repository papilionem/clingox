# Platform support

clingox builds clingo from its C++ source for each target, so the question for every
platform is whether that build and the test suite have been run there. The table
records what the continuous integration workflow (`.github/workflows/ci.yml`) runs
on each target. The jobs that were experimental at the first release became
ordinary jobs after passing in every run that finished from 2026-09-30 to
2026-10-02, for example
[run 36991276242](https://github.com/papilionem/clingox/actions/runs/36991276242).

## Status words

- **Tested**: the test suite runs in CI and a failure fails the run.
- **Built only**: CI compiles the library and its test executables for the target,
  but no test runs there.
- **Not tested**: no test runs there in CI.
- **nightly**, after a status: the job runs every night, for a release, and when
  started by hand, not on every push.

"The test suite" means `cargo test` on the target: the unit tests, the integration
tests and the doctests of `clingox-sys` and `clingox`. Where a row says so, the
doctests or some tests are left out. The compile-fail tests, which compare exact
compiler output, run only on Linux x86_64 with a pinned Rust release.

## Desktop and server

| Platform | Target | Status | How it runs |
|---|---|---|---|
| Linux x86_64 | `x86_64-unknown-linux-gnu` | Tested | Ubuntu 24.04, whole workspace. Also runs the sanitizers and Miri (below). |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | Tested | Ubuntu 24.04 on an ARM64 runner, whole workspace. |
| Linux x86 (32-bit) | `i686-unknown-linux-gnu` | Tested | Ubuntu 24.04 with multilib, `clingox-sys` and `clingox`. |
| Linux ARMv7 | `armv7-unknown-linux-gnueabihf` | Tested, nightly | Cross-compiled and run under qemu-user. |
| macOS ARM64 | `aarch64-apple-darwin` | Tested | macOS 15, whole workspace. |
| macOS x86_64 | `x86_64-apple-darwin` | Tested | macOS 15 on an Intel runner, whole workspace. |
| Windows x64 | `x86_64-pc-windows-msvc` | Tested | Windows Server 2025, whole workspace. |
| Windows ARM64 | `aarch64-pc-windows-msvc` | Tested | Windows 11 on an ARM64 runner, whole workspace. |
| Windows x86 (32-bit) | `i686-pc-windows-msvc` | Tested | Windows Server 2025, `clingox-sys` and `clingox`. |
| Windows with MinGW | `*-pc-windows-gnu` | Not tested | |
| FreeBSD 14 | `x86_64-unknown-freebsd` | Tested | A virtual machine on a Linux runner, whole workspace. |
| NetBSD 10 | `x86_64-unknown-netbsd` | Tested | A virtual machine, whole workspace, with the system GCC and libstdc++. |
| OpenBSD 7 | `x86_64-unknown-openbsd` | Tested | A virtual machine, whole workspace, with the Rust that OpenBSD packages. |

All Windows targets are tested with the MSVC toolchain only. The `-gnu` targets
may work, but no one has run the suite on them. On Windows, the vendored build
compiles clingo with `/EHsc`; a clingo installed on the system needs the same (see
[Installation](../getting-started/installation.md)).

## Mobile

| Platform | Target | Status | How it runs |
|---|---|---|---|
| Android x86_64 | `x86_64-linux-android` | Tested | An emulator, API level 34. |
| Android x86 (32-bit) | `i686-linux-android` | Tested, nightly | An emulator, API level 30. |
| Android ARM64 | `aarch64-linux-android` | Built only | See below. |
| Android ARMv7 | `armv7-linux-androideabi` | Tested, nightly | The 32-bit x86 emulator's ARM translation, API level 30. See below. |
| iOS simulator | `aarch64-apple-ios-sim` | Tested | The iPhone simulator on an Apple silicon runner. |
| iOS devices | `aarch64-apple-ios` | Not tested | |

On Android and iOS the test executables run as plain programs in the emulator or
simulator, so the doctests of `clingox` are left out. The iOS job also leaves out the
compile-fail test file, which starts `cargo` and so cannot run inside the simulator.

Android ARM64 is not tested yet. Its test executables are built in CI, but the job
that should run them on an ARM64 emulator needs hardware virtualisation (KVM), which
the ARM64 runner does not offer, so the job fails after checking for it. It is the
one job marked `continue-on-error`, so its failure is reported without failing the
run.

The ARMv7 test executables run on the 32-bit x86 emulator through its ARM
translation layer, with the C++ runtime linked statically into each executable.
This exercises the ARMv7 build of clingo and clingox, but under translation, not on
ARM hardware.

clingo 5.8 is known to give wrong results on ARM64 devices with Android 11 or
later, because it stores flags in pointer bits that Android uses for pointer
tagging. The x86 emulators cannot show this problem. See
[Known issues](known-issues.md#platforms) before you ship on ARM64 Android.

## WebAssembly

| Runtime | Target | Status | How it runs |
|---|---|---|---|
| Node.js | `wasm32-unknown-emscripten` | Tested | Under Node.js, on Linux. |
| Chromium | `wasm32-unknown-emscripten` | Tested | Headless, through Playwright, on Linux. |
| Firefox | `wasm32-unknown-emscripten` | Tested | Headless, through Playwright, on Linux. |
| WebKit | `wasm32-unknown-emscripten` | Tested | Playwright's WebKit build, headless, on Linux. |
| Safari | `wasm32-unknown-emscripten` | Not tested | |

The WebKit row is Playwright's build of the WebKit engine on Linux, not Safari.
Safari on macOS and iOS uses the same engine, but a different build and
integration, and has not been run. The default WebAssembly build is
single-threaded, so the tests that need threads (parallel solving, timeouts, async
solving, moving a control between threads) are skipped there.

## When the jobs run

- On every pull request: Linux x86_64 and ARM64, macOS ARM64, Windows x64, Node.js,
  the compile-fail tests, and `cargo check` with the minimum supported Rust version.
- On every push to the main branch, in addition: every other row above except those
  marked "nightly", and the sanitizers and Miri.
- Every night, and when started by hand: everything, including Linux ARMv7 and
  32-bit Android, which take too long to run on every push.

The sanitizer job runs the suite on Linux x86_64 under AddressSanitizer with
LeakSanitizer, then the thread tests under ThreadSanitizer. Miri runs the unit
tests and the callback trampolines.

## Building for WebAssembly

clingox targets `wasm32-unknown-emscripten`, which is the target clingo itself
documents for web builds. In a release build, the module of a small program that
adds, grounds and solves a program and prints its answer sets is about 2.4 MB, or
0.84 MB compressed with gzip (Rust 1.99, Emscripten 6.0.10). It is single-threaded
by default. A multi-threaded build is
possible on pages served with cross-origin isolation headers, and currently needs
nightly Rust.

`wasm32-unknown-unknown`, the target used with `wasm-bindgen`, cannot host clingo's
C++ code.

### Memory

Rust's emscripten target keeps emscripten's default heap of 16 MB, and does not
let it grow. That is enough for small programs, but grounding a larger one fails
with [`ErrorKind::BadAlloc`](../concepts/errors-and-panics.md). Let the heap grow
when you link the application, for example in `.cargo/config.toml`:

```toml
[target.wasm32-unknown-emscripten]
rustflags = ["-C", "link-arg=-sALLOW_MEMORY_GROWTH=1"]
```

The flag has to be set by the application: a library such as clingox cannot
pass link arguments to the crates that depend on it. `-sMAXIMUM_MEMORY` caps the
growth if you need a limit (the default cap is 2 GB).

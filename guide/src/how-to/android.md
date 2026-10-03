# Run on Android

clingox builds for Android with the Android NDK. The repository's CI runs the test
suite on an x86_64 emulator on every push to the main branch, and on 32-bit x86 and
ARMv7 emulation every night (see
[Platform support](../reference/platforms.md#mobile)). This page says what your own
project needs to build for Android, what to ship with it, and what clingox does not
cover.

**Read this before you ship on a phone.** clingo 5.8 stores flags in the high bits of
pointers, which Android 11 and later tag on ARM64. On such devices, which include most
current phones, grounding and solving give wrong, empty results (clingo issues #475
and #540). The emulators the tests run on cannot show this, and the ARM64 build is
compiled in CI but never run. clingox has no workaround; the clingo maintainers say
clingo 6 fixes it. See [Known issues](../reference/known-issues.md#platforms).

## What you need

- **The Android NDK.** The repository does not pin an NDK version: CI uses the one
  installed on GitHub's Ubuntu runners, and `cargo xtask test android` uses the one
  named by `ANDROID_NDK_HOME` or `ANDROID_NDK_ROOT`, or else the newest one in your
  Android SDK. The steps below were checked with NDK 29.
- **The Rust target** for each ABI you ship, for example
  `rustup target add aarch64-linux-android`.
- **CMake**, as for every vendored build ([Installation](../getting-started/installation.md)).

| ABI | Rust target | Prefix of the NDK's clang |
|---|---|---|
| `arm64-v8a` | `aarch64-linux-android` | `aarch64-linux-android` |
| `armeabi-v7a` | `armv7-linux-androideabi` | `armv7a-linux-androideabi` |
| `x86` | `i686-linux-android` | `i686-linux-android` |
| `x86_64` | `x86_64-linux-android` | `x86_64-linux-android` |

## Point the build at the NDK

Cargo needs the NDK's clang as the linker, and the build script of `clingox-sys` needs
to find the NDK for clingo's CMake build. These variables are what the repository's
own Android builds set (here for ARM64 at API level 24, the level CI builds for, on a
Linux host):

```sh
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/<version>"
export ANDROID_PLATFORM=24
bin="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$bin/aarch64-linux-android24-clang"
export CC_aarch64_linux_android="$bin/aarch64-linux-android24-clang"
export CXX_aarch64_linux_android="$bin/aarch64-linux-android24-clang++"
export AR_aarch64_linux_android="$bin/llvm-ar"
cargo build --release --target aarch64-linux-android
```

From these, the build script uses the NDK's CMake toolchain file
(`build/cmake/android.toolchain.cmake`), the ABI of the Rust target, and the API level
in `ANDROID_PLATFORM`. It also reads `ANDROID_NDK_ROOT` or `ANDROID_NDK` in place of
`ANDROID_NDK_HOME`, and `CARGO_NDK_PLATFORM`, which the `cargo-ndk` tool sets, in
place of `ANDROID_PLATFORM`; CI does not use `cargo-ndk`. A `CMAKE_TOOLCHAIN_FILE`
you set yourself takes precedence. The build also supplies one function that clingo
expects from glibc and Android's C library lacks, so the vendored source needs no
change.

## Ship the C++ runtime

The build links clingo against the NDK's shared C++ runtime, so your binary needs
`libc++_shared.so` at run time:

```console
$ llvm-readelf -d target/aarch64-linux-android/release/libmy_app.so | grep NEEDED
  0x0000000000000001 (NEEDED)       Shared library: [libc++_shared.so]
  0x0000000000000001 (NEEDED)       Shared library: [libdl.so]
  0x0000000000000001 (NEEDED)       Shared library: [libm.so]
  0x0000000000000001 (NEEDED)       Shared library: [libc.so]
```

Copy it from the NDK, at
`toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/<triple>/libc++_shared.so`
(the triple is `arm-linux-androideabi` for ARMv7 and the Rust target's otherwise), and
ship it with your library for the same ABI. The repository's tests push it next to
each test executable and set `LD_LIBRARY_PATH`.

To link the runtime statically instead, set `CXXSTDLIB=c++_static` and add the C++ ABI
library to the link, as CI does for its ARMv7 tests:

```sh
export CXXSTDLIB=c++_static
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_RUSTFLAGS="-C link-arg=-lc++abi"
```

The NDK's
[C++ library support](https://developer.android.com/ndk/guides/cpp-support) page says
when a static runtime is safe in an app with several native libraries.

## Call clingox from an app

An Android app usually loads Rust code as a shared library (`crate-type = ["cdylib"]`)
from Java or Kotlin through JNI. clingox builds that way like any other crate, but it
has no JNI support of its own, and its CI runs only command-line test executables on
the emulator, not an app. The bindings between your app and your Rust code are yours
to write and test.

Android builds have threads. The test executables of `clingox` and `clingox-sys` run
on the emulator, including the tests of parallel solving, timeouts and asynchronous
solving. The test files that need the repository's files, another process or Cargo
skip themselves there, and the doctests do not run.

## Files and temporary directories

clingox opens files only when you ask it to: `Control::load`, `Control::load_aspif`,
`ast::parse_files`, a backend writer, clingo's application, an `#include` directive
in program text, and clingo's option `--configuration=<file>`. On Android,
`std::env::temp_dir()` returns `/data/local/tmp` unless `TMPDIR` is set. The
repository's tests run from an `adb shell`, which may write there; an app normally
may not.
Pass your Rust code a directory the app owns, such as the cache directory Android
gives it, and keep paths out of your rules:

```rust
use std::error::Error;
use std::path::Path;

use clingox::{Control, Part};

/// Saves `program` in `dir`, a directory the app may write to, and solves it.
fn solve_saved(dir: &Path, program: &str) -> Result<bool, Box<dyn Error>> {
    let file = dir.join("program.lp");
    std::fs::write(&file, program)?;
    let mut ctl = Control::new()?;
    ctl.load(&file)?;
    ctl.ground(&[Part::base()])?;
    Ok(ctl.solve(&[])?.is_sat())
}

// Here the temporary directory stands in for the app's cache directory.
let dir = std::env::temp_dir().join(format!("clingox-guide-android-{}", std::process::id()));
std::fs::create_dir_all(&dir)?;
assert!(solve_saved(&dir, "a. b :- a.")?);
std::fs::remove_dir_all(&dir)?;
# Ok::<(), Box<dyn Error>>(())
```

## Run your tests on an emulator

The repository runs its test executables as plain programs in an x86_64 emulator, and
you can do the same with your own crate's tests:

```sh
cargo test --no-run --target x86_64-linux-android
adb push target/x86_64-linux-android/debug/deps/my_test-<hash> /data/local/tmp/
adb push "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/x86_64-linux-android/libc++_shared.so" /data/local/tmp/
adb shell "cd /data/local/tmp && LD_LIBRARY_PATH=. ./my_test-<hash>"
```

`cargo test --no-run` prints the path of each test executable. Doctests do not run
this way, and the repository leaves them out on Android. A debug test executable that
links clingo is large (about 70 MB for the repository's own), so remove each one from
the emulator after it has run if its storage is small.

## Related pages

- [Platform support](../reference/platforms.md) lists every Android target and how CI
  runs it.
- [Known issues](../reference/known-issues.md#platforms) has the ARM64 problem in
  full.
- [Feature flags](../reference/feature-flags.md#vendored) lists the C++ runtime each
  platform links.

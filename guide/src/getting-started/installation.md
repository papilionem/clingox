# Installation

This chapter sets up a Rust project that depends on clingox and checks that clingo
builds and runs.

## What the build needs

clingox compiles clingo from its C++ source as part of your Rust build. You need:

- **Rust 1.98 or newer.** clingox follows the latest stable release.
- **CMake 3.10 or newer.** The build script drives clingo's own CMake build.
- **A C and C++ compiler** that CMake finds, with C++14 support: GCC or Clang on
  Linux and the BSDs, Apple's Clang on macOS, and Visual Studio's MSVC on Windows.

Nothing else is required. clingo's parsers are generated ahead of time, so Bison and
re2c are not needed, and Python and Lua support are switched off.

The first build compiles clingo in release mode, even for a debug build of your
project, because an unoptimised clingo is too slow to be useful. Later builds reuse
the result. If you have [ccache](https://ccache.dev) installed and want clingo's
C++ build cached by it too, set `CLINGOX_SYS_CCACHE=1` (or `=auto`); it is off by
default, so a plain `cargo build` never reaches outside the build for you.

For Android you also need the Android NDK, and for WebAssembly the Emscripten SDK.
The [platform page](../reference/platforms.md) lists which targets are tested.

## Add the dependency

Add clingox from crates.io:

```sh
cargo add clingox@508.2.0-beta.1
```

or write the dependency in `Cargo.toml` yourself:

```toml
[dependencies]
clingox = "508.2.0-beta.1"
```

The published crate contains the clingo source, so the build needs no network. To
work on clingox itself, clone the repository with `git clone --recurse-submodules`
(clingo is a git submodule there) and use a path dependency:

```toml
[dependencies]
clingox = { path = "../clingox/clingox" }
```

## Features

`clingox` has four features, all on by default:

| Feature | What it does |
|---|---|
| `derive` | `#[derive(ToSymbol)]`, `#[derive(FromSymbol)]` and `sym!` |
| `log` | clingo's messages go to the `log` crate, under the target `clingox` |
| `vendored` | builds clingo from the source that ships with clingox, with clingox's fixes for known clingo bugs applied |
| `threads` | builds clingo with thread support, which parallel solving, timeouts and async solving need; on WebAssembly it takes effect only with the `atomics` target feature |

Without `derive`, the traits `ToSymbol` and `FromSymbol` remain, and you implement
them by hand. Without `threads`, or with a system clingo built without threads,
`ControlBuilder::threads(n)` for `n` above 1, `Control::solve_async` and every
timeout return `ErrorKind::Unsupported`; everything else works. To turn every
feature off and choose your own:

```toml
[dependencies]
clingox = { git = "https://github.com/papilionem/clingox", default-features = false, features = ["vendored"] }
```

Without `vendored`, the build looks for a clingo installed on your system.

## A clingo installed on the system

To link an installed clingo instead of the vendored one, turn `vendored` off as
above, or set `CLINGOX_SYS_NO_VENDOR=1` (which also works when another crate turns
`vendored` on, since Cargo features are additive). These variables, read at build
time, describe the installation:

| Variable | Meaning |
|---|---|
| `CLINGO_INCLUDE_DIR` | the directory that holds `clingo.h`, if it is not in a standard place |
| `CLINGO_LIB_DIR` | the directory that holds the library, if it is not in a standard place |
| `CLINGO_STATIC` | `1` to link clingo's five static libraries instead of the shared `libclingo` |
| `CLINGO_THREADS` | `1` (or unset) if that clingo was built with threads, the default of its CMake build; `0` if it was built with `CLASP_BUILD_WITH_THREADS=OFF` |

clingox accepts clingo 5.8.1 or newer within 5.8: the build checks the header, and
the first call into clingo checks the library your program actually loads. Anything
else is refused with a message that says why. 5.8.1 is the first release that creates
symbols safely from several threads at once, and it has the same C API as 5.8.2, the
version clingox ships.

On Windows the installed clingo must have been built with `/EHsc`, which a normal
CMake build does by default. A build without it compiles clingo's exception handling
without unwinding, and errors that clingo reports through exceptions then hang or
crash the process.

A system clingo does not get clingox's fixes for known clingo bugs, which are
applied only to the vendored source. The [known issues](../reference/known-issues.md)
list what remains there.

## Check the setup

Create a project, and add the dependency above to its `Cargo.toml`:

```console
$ cargo new check-clingox
$ cd check-clingox
```

Replace `src/main.rs` with a program that solves a one-line logic program:

```rust
use clingox::{Control, Part};

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base("hello.")?;
    ctl.ground(&[Part::base()])?;
    let result = ctl.solve(&[])?;
    println!("{result}");
#   assert!(result.is_sat());
    Ok(())
}
```

Run it:

```console
$ cargo run
SATISFIABLE
```

The program has one answer set, which contains `hello`, so clingo reports it as
satisfiable. If you see this line, clingo was built, linked and called correctly.
The [next chapter](first-program.md) explains each step.

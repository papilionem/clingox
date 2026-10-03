# Run in the browser

clingox runs in a browser as WebAssembly. clingo's C++ and your Rust code link into
one module for the target `wasm32-unknown-emscripten`, which Emscripten builds. The
repository's CI runs the test suite this way under Node.js and in headless Chromium,
Firefox and WebKit (see [Platform support](../reference/platforms.md#webassembly)).
This page says what your own project needs, how to build and load it, and what
behaves differently from a native build.

clingox has no JavaScript API. What is tested is a Rust program, with a `main`, built
for the Emscripten target and loaded into a page. The other WebAssembly targets do
not work: `wasm32-unknown-unknown`, the target of `wasm-bindgen`, cannot host clingo's
C++ code, and WASI is not supported.

## What you need

- **The Rust target**: `rustup target add wasm32-unknown-emscripten`.
- **The Emscripten SDK.** CI uses Emscripten 6.0.10, the version in the repository's
  `xtask/emsdk-version`; other versions are not tested. Install and activate it, and
  load its environment in the shell you build from:

  ```sh
  git clone --branch 6.0.10 https://github.com/emscripten-core/emsdk.git
  cd emsdk
  ./emsdk install 6.0.10
  ./emsdk activate 6.0.10
  source ./emsdk_env.sh
  ```

  `emsdk_env.sh` puts `emcc` on the `PATH`, which Rust links with, and sets `EMSDK`,
  through which the build script of `clingox-sys` finds Emscripten's CMake toolchain
  for clingo.
- **CMake**, as for every vendored build ([Installation](../getting-started/installation.md)).

No other change to your code or your dependencies is needed. The default features
build a single-threaded clingo for this target.

## Let the heap grow

Rust's Emscripten target keeps Emscripten's default heap of 16 MB and does not let it
grow. Grounding a program of a few hundred thousand atoms then fails with
`ErrorKind::BadAlloc`. Allow growth when you link the application, for example in
`.cargo/config.toml` next to your `Cargo.toml`:

```toml
[target.wasm32-unknown-emscripten]
rustflags = ["-C", "link-arg=-sALLOW_MEMORY_GROWTH=1"]
```

clingox cannot set this for you: a library cannot pass link arguments to the crates
that depend on it. `-sMAXIMUM_MEMORY` caps the growth (the default cap is 2 GB).

## Build and run under Node.js

```sh
cargo build --release --target wasm32-unknown-emscripten
node target/wasm32-unknown-emscripten/release/my-app.js
```

For a crate named `my-app`, the build writes two files: `my-app.js`, Emscripten's
loader, and `my_app.wasm`, the module it loads from the same directory. The module's
name has an underscore where the crate's name has a hyphen, as Rust names compiled
crates. Node.js runs `main` and prints to the
terminal. To run `cargo run` and `cargo test` the same way, set Node.js as the
target's runner, as the repository's own WebAssembly tests do:

```toml
[target.wasm32-unknown-emscripten]
runner = "node"
```

With Emscripten 6.0.10 the link prints a warning that `WASM_BIGINT is deprecated`.
Rust's target passes that setting, not clingox, and the warning is harmless.

## Load it in a page

Emscripten's loader reads its settings from a global `Module` object, so define it
before the loader's `<script>` tag. `print` and `printErr` receive what `main` writes
to standard output and standard error:

```html
<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>clingox in the browser</title></head>
<body>
<pre id="output"></pre>
<script>
  const output = document.getElementById('output');
  var Module = {
    print: (text) => { output.textContent += text + '\n'; },
    printErr: (text) => console.error(text),
  };
</script>
<script src="my-app.js"></script>
</body>
</html>
```

Serve the page, `my-app.js` and `my_app.wasm` from one directory over HTTP, with the
type `application/wasm` for the module. `main` runs as soon as the module has loaded.
The repository's browser tests load each test program this way (their page is
`xtask/browser/index.html`); `Module.arguments` passes command-line arguments if your
program reads any.

Firefox writes a warning to its console that WebAssembly's `try` instruction is
deprecated. The module uses that form of exception handling, and runs.

The search runs on the thread that calls it, which in this setup is the page's main
thread, so a long search keeps the page from responding until it ends. Loading the
module in a Web Worker would avoid that; this repository does not test it.

## Write code that works without threads

The default WebAssembly build has one thread. These differ from a native build:

| Feature | In the default WebAssembly build |
|---|---|
| `SolveOptions::timeout`, `solve_async` | `ErrorKind::Unsupported`, and the control stays usable |
| `ControlBuilder::threads(n)` with `n` above 1 | `build()` fails with `ErrorKind::Unsupported` and makes no control |
| `--parallel-mode` and related options | unknown to clingo: `Control::with_args` fails with `ErrorKind::Logic` |
| `InterruptHandle` on `Control::solve` | `interrupt()` returns `false` |
| CPU time in the statistics | always 0 ([Known issues](../reference/known-issues.md#behaviour-to-know-about)) |

Code shared between native and browser builds can try a timeout and fall back to a
conflict limit, which works everywhere:

```rust
use std::time::Duration;

use clingox::prelude::*;
use clingox::{ErrorKind, SolveOptions};

/// Solves within five seconds where clingo has threads, and within
/// 100 000 conflicts where it has not.
fn solve_bounded(ctl: &mut Control) -> clingox::Result<SolveResult> {
    let options = SolveOptions::new().timeout(Duration::from_secs(5));
    match ctl.solve_with(options) {
        Err(err) if err.kind() == ErrorKind::Unsupported => {
            ctl.configuration().set("solve.solve_limit", "100000")?;
            ctl.solve(&[])
        }
        result => result,
    }
}

let mut ctl = Control::new()?;
ctl.add_base("{ p(1..3) }. :- not p(2).")?;
ctl.ground(&[Part::base()])?;
assert!(solve_bounded(&mut ctl)?.is_sat());
# Ok::<(), clingox::Error>(())
```

The limit stays set on the control for later calls.
[Set a time budget](time-budget.md#without-threads) compares the options.

## Threads in WebAssembly

A multi-threaded build is possible in principle: the `threads` feature takes effect
when the target is compiled with the `atomics` target feature, and the build script
then compiles clingo with Emscripten's `-pthread`. That needs nightly Rust (to
rebuild the standard library with atomics), and a page served with the headers
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp`. clingox's CI does not build or test this
configuration, and this guide does not document it further.

## Size

A release build of a program that adds, grounds and solves a one-line logic program
and prints its answer sets, made with Rust 1.99 and Emscripten 6.0.10, gave a module
of 2.4 MB: 0.84 MB compressed with `gzip -9` and 0.60 MB with Brotli. Its loader added
63 KB, 18 KB with gzip. Most of the module is clingo's grounder and solver, which stay
whole whatever your program uses, because clingo reaches most of its code through
virtual calls. Ship a `--release` build.

## Related pages

- [Feature flags](../reference/feature-flags.md#threads) explains when the `threads`
  feature takes effect.
- [Test your rules](test-your-rules.md) applies to WebAssembly too, with Node.js as
  the runner.
- [Known issues](../reference/known-issues.md) lists what differs in clingo itself.

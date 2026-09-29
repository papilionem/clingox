# Platform support

clingox builds clingo from its C++ source for each target, so the question for every
platform is whether that build and the test suite have been run there.

| Platform | Target | Status |
|---|---|---|
| Linux x86_64 | `x86_64-unknown-linux-gnu` | Tested |
| Android x86_64 | `x86_64-linux-android` | Tested on an emulator (API 34) |
| Android ARM64 | `aarch64-linux-android` | Planned. clingo 5.8 is known to misbehave on Android 11+ ARM64 devices ([known issues](known-issues.md)) |
| WebAssembly in Node.js | `wasm32-unknown-emscripten` | Tested |
| WebAssembly in Chromium | `wasm32-unknown-emscripten` | Tested |
| WebAssembly in Firefox | `wasm32-unknown-emscripten` | Tested |
| WebAssembly in Safari | `wasm32-unknown-emscripten` | Not yet tested |
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | Planned |
| Windows | `x86_64-pc-windows-msvc`, `-gnu` | Planned |
| iOS | `aarch64-apple-ios` | Planned |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | Planned |

"Tested" means the full test suite passes on that platform, including clingo's error
handling, which is the part most sensitive to how C++ is compiled for a target.

## WebAssembly

clingox targets `wasm32-unknown-emscripten`, which is the target clingo itself
documents for web builds. The module for a minimal program is about 655 KB
compressed with gzip. It is single-threaded by default. A multi-threaded build is
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

# clingox-sys

Raw FFI declarations of the [clingo](https://potassco.org/clingo/) 5.8 C API, and the
build of clingo itself. This is the low-level crate behind
[clingox](https://crates.io/crates/clingox); use clingox for a safe API.

Every item mirrors a declaration in `clingo.h` under its C name, and none of it is
safe to call without reading the header's contract.

## Building clingo

With the default `vendored` feature, the build script compiles clingo 5.8.2 from the
source that ships in this crate, with the patches in `patches/` applied to a copy of
it. It needs CMake 3.10 or newer and a C++14 compiler, and no network. The first
build takes a few minutes; later builds reuse the result.

To link a clingo installed on the system instead, turn `vendored` off or set
`CLINGOX_SYS_NO_VENDOR=1`. The build then reads `CLINGO_INCLUDE_DIR`,
`CLINGO_LIB_DIR`, `CLINGO_STATIC` and `CLINGO_THREADS`, and accepts clingo 5.8.1 or
newer within 5.8. A system clingo gets none of the patches. The
[installation chapter](https://papilionem.github.io/clingox/getting-started/installation.html)
of the guide describes each variable.

This crate declares `links = "clingo"`, so it cannot share a dependency graph with
Potassco's `clingo-sys`.

## License

The Rust code is licensed under either of
[Apache License, Version 2.0](https://github.com/papilionem/clingox/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/papilionem/clingox/blob/main/LICENSE-MIT) at your
option. clingo is MIT-licensed; its bundled libraries are under the licenses listed in
[THIRD-PARTY-LICENSES](https://github.com/papilionem/clingox/blob/main/clingox-sys/THIRD-PARTY-LICENSES),
and every license file ships in the package.

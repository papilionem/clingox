# Versions

A clingox version number says which clingo release it contains:

```text
{clingo major}{clingo minor, two digits}.{clingo patch}.{clingox release}
```

| clingox | contains |
|---|---|
| `508.2.0` | clingo 5.8.2, first clingox release for it |
| `508.2.1` | clingo 5.8.2, a clingox fix or addition |
| `508.3.0` | clingo 5.8.3 |
| `509.0.0` | clingo 5.9.0 |

## Why the number looks like this

clingo changes its C API in minor releases: 5.7.0, for example, changed a function
signature and swapped two enum values. Cargo treats `5.8` to `5.9` as a compatible
upgrade, so a version number copied from clingo would let `cargo update` pull in an
incompatible clingo. Putting clingo's major and minor version into clingox's major
version (`508` for 5.8, `509` for 5.9) means Cargo never moves you across a clingo
minor release without you asking.

The same scheme is used by `llvm-sys` for LLVM and `openssl-src` for OpenSSL.

## Choosing a clingo version

Depend on the clingox release whose number names the clingo version you want:

```toml
[dependencies]
clingox = "508.2"
```

`clingox`, `clingox-sys` and `clingox-derive` always share the same version number.

## Stability

Cargo treats releases that differ only in the patch number as compatible, so within
`508.2.x` the Rust API has no breaking change: the last number only fixes bugs and
adds. A breaking change waits for a new clingo version. Pre-releases such as
`508.2.0-beta.4` may still change the API between them, and Cargo does not select
one unless you name it.

## Deprecation

An item that is going away is first marked `#[deprecated]`, with its replacement in
the note, and listed under "Deprecated" in the changelog. It is removed only in a
release that may break the API, which means a new clingo version.

## Minimum supported Rust version

The minimum supported Rust version (MSRV) is 1.98. It is the `rust-version` of all
three crates, and CI compiles the workspace with that toolchain.

clingox follows the latest stable Rust, so the MSRV rises from time to time. A rise
is not treated as a breaking change: it can come in any release, and the changelog
names it. Cargo's resolver takes `rust-version` into account in edition 2024
projects, so a project on an older toolchain keeps the newest clingox release that
supports it.

## Checks

clingox checks the clingo version twice: when it builds, and when your program
first calls into clingo. clingo's shared library keeps the same version number
across releases whose API differs, so a mismatched library could otherwise go
unnoticed and fail in unpredictable ways. If you link a clingo installed on your
system, it must be 5.8.1 or newer within 5.8.

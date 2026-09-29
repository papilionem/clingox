# clingox

[![crates.io](https://img.shields.io/crates/v/clingox.svg)](https://crates.io/crates/clingox)
[![docs.rs](https://img.shields.io/docsrs/clingox)](https://docs.rs/clingox)
[![CI](https://github.com/papilionem/clingox/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/papilionem/clingox/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/clingox.svg)](https://github.com/papilionem/clingox#license)
[![MSRV](https://img.shields.io/crates/msrv/clingox.svg)](https://papilionem.github.io/clingox/concepts/versions.html#minimum-supported-rust-version)

Safe Rust bindings to [clingo](https://potassco.org/clingo/) 5.8.2, the answer set
programming system from the Potassco project.

clingox builds clingo from source as part of your Rust build, with patches for known
clingo defects, and exposes its C API through a safe interface: your code needs no
`unsafe`. The mirror layer follows clingo's own API, so clingo's documentation
applies. The typed layer converts Rust types to facts and reads models back as Rust
values.

```rust
use clingox::prelude::*;

#[derive(ToSymbol)]
struct Edge(i32, i32);

#[derive(FromSymbol, Debug, PartialEq)]
struct Reach(i32);

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base("reach(1). reach(Y) :- reach(X), edge(X, Y).")?;
    ctl.add_facts([Edge(1, 2), Edge(2, 3), Edge(4, 5)])?;
    ctl.ground(&[Part::base()])?;

    if let Outcome::Sat(model, _) = ctl.solve_first()? {
        let mut reached: Vec<Reach> = model.atoms()?;
        reached.sort_by_key(|r| r.0);
        assert_eq!(reached, [Reach(1), Reach(2), Reach(3)]);
    }
    Ok(())
}
```

The build needs Rust 1.98 or newer, CMake 3.10 or newer, and a C and C++ compiler.
The first build compiles clingo, which takes a few minutes; later builds reuse it.

Versions ending in `-beta.N` are pre-releases, and the API may still change between
them.

## Features

| Feature | Default | What it does |
|---|---|---|
| `vendored` | yes | Builds clingo 5.8.2 from the source in `clingox-sys`, with its patches. Without it, a system clingo 5.8.1 or newer within 5.8 is linked. |
| `threads` | yes | Builds clingo with threads: parallel solving, timeouts and async solving. |
| `derive` | yes | `#[derive(ToSymbol)]`, `#[derive(FromSymbol)]` and `sym!`. |
| `log` | yes | Forwards clingo's messages to the `log` crate, target `clingox`. |

## Documentation

- [The guide](https://papilionem.github.io/clingox/) teaches clingox from the first
  program on, with pages for readers coming from pyclingo or the `clingo` crate.
- [The API reference](https://docs.rs/clingox) on docs.rs.
- [Platform support](https://papilionem.github.io/clingox/reference/platforms.html)
  lists the targets the test suite runs on.
- [Versions](https://papilionem.github.io/clingox/concepts/versions.html) explains
  the version numbers (`508.2.x` contains clingo 5.8.2) and the MSRV policy.

clingox is independent of the Potassco project and is not the
[`clingo`](https://crates.io/crates/clingo) crate. Its `clingox-sys` declares
`links = "clingo"`, so it cannot share a dependency graph with `clingo-sys`.

## License

Licensed under either of [Apache License, Version 2.0](https://github.com/papilionem/clingox/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/papilionem/clingox/blob/main/LICENSE-MIT) at your
option. clingo and the code it bundles are under their own permissive licenses,
listed in `clingox-sys`'s
[THIRD-PARTY-LICENSES](https://github.com/papilionem/clingox/blob/main/clingox-sys/THIRD-PARTY-LICENSES).

# Conformance tests

The files `conformance_*.rs` in the parent directory port tests that ship with clingo,
to check clingox against the behaviour of the reference implementation.

## Upstream sources

- **Project:** clingo v5.8.2, by Roland Kaminski and contributors, part of the
  [Potassco](https://potassco.org/) project. Source: <https://github.com/potassco/clingo>.
- **License:** MIT. The upstream license text is `clingox-sys/clingo/LICENSE.md` and is
  reproduced in `clingox-sys/THIRD-PARTY-LICENSES`.
- **What is ported:** the C examples in `examples/c`, the sections of
  `libclingo/tests`, the pyclingo tests in `libpyclingo/clingo/tests`, and the
  `.lp`/`.sol` fixtures in `app/clingo/tests` (which the fixture tests read straight
  from the vendored submodule).

## Origin headers

Every ported file starts with a header that names the upstream path and tag it came
from and how the port differs (for example `Ported from potassco/clingo v5.8.2`,
`Source: ...`, `Differences: ...`). Keep the header when you edit a ported file, and add
one to any new port. Upstream items that are not ported are listed, with the reason, in
`NOT_PORTED.md`; `cargo xtask conformance-count` checks that list against the
submodule.

## Licensing of the ported files

The ported tests are derived from MIT-licensed upstream tests and keep that origin.
The rest of clingox is available under MIT or Apache-2.0 (see the repository root).
The clingox package published to crates.io excludes the `tests/` directory (see
`exclude` in `clingox/Cargo.toml`), so derived test code is not redistributed there;
the files stay in the repository, where the origin headers and this notice apply.

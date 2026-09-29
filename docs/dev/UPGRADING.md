# Upgrading

This is the procedure for moving clingox to a new clingo release, a new Emscripten
SDK, a new Rust toolchain, or new dependency versions. Follow it step by step; it is
written so that nobody has to remember how the last upgrade went.

## 1. A new clingo release

### 1.1 Decide the new version number

Read the release's `CHANGES.md` entry, then apply the rule from DESIGN §10:

| clingo change | clingox version | Example |
|---|---|---|
| new minor or major (5.9.0) | new major | `509.0.0` |
| new patch (5.8.3) | next minor | `508.3.0` |

A clingo patch release that changes the C API has never happened (every API break
so far came with a minor release), but the rule for it is fixed in advance (DESIGN
§10): `clingox-sys` mirrors clingo, so it takes the change under `508.3.0` with a
changelog warning, and `clingox` absorbs the change internally so its safe API stays
compatible. Step 1.4 shows whether this case applies.

### 1.2 Create the branch and move the source

```sh
git switch -c upgrade/clingo-5.9.0
cd clingox-sys/clingo
git fetch --depth 1 origin tag v5.9.0
git checkout v5.9.0
cd ../..
```

Check the tag is complete without nested submodules:

```sh
git -C clingox-sys/clingo ls-tree -r HEAD | awk '$1 == "160000"'   # must print nothing
```

If it prints anything, clingo started using submodules again. Stop and update
DESIGN §5.1 and the build script before continuing.

### 1.3 Record the new upstream commit

Update the pinned commit hash used by the commit-hash check
(`clingox-sys/clingo.commit`), and the `CLINGO_VERSION` constant source.

### 1.4 Regenerate the bindings and read the diff

```sh
cargo xtask bindgen
git diff --stat clingox-sys/src/bindings.rs
git diff clingox-sys/src/bindings.rs
```

This diff is the C API change. Sort every hunk into one of these:

- **Added** functions, types, enum values: new rows in COVERAGE.md, to be wrapped.
- **Changed** signatures or enum values: the existing wrappers must change. Each
  one is a breaking change for `clingox-sys` and usually for `clingox`.
- **Removed** items: the wrappers go, with a changelog entry.
- **Comment-only** changes: read them anyway. clingo documents behaviour in the
  header, and a changed comment can mean changed semantics.

Also compare the header between the two tags directly, because bindgen does not show
macros that it drops:

```sh
git -C clingox-sys/clingo diff v5.8.2 v5.9.0 -- libclingo/clingo.h
```

### 1.5 Re-check the build assumptions

Each item below is an assumption DESIGN §5.2 relies on. Confirm it still holds:

- CMake option names (`git diff v5.8.2 v5.9.0 -- CMakeLists.txt libclingo/CMakeLists.txt`);
- the library list and link order (compare with the exported `ClingoTargets`);
- committed bison/re2c output still present under `libgringo/gen/`;
- the minimum CMake version and C++ standard;
- anything new fetched at build time (search for `FetchContent`, `ExternalProject`,
  `file(DOWNLOAD`); the build must stay offline;
- every patch in `clingox-sys/patches/`: check whether the new release contains the
  fix (then delete the patch and mark the entry fixed upstream), and that the rest
  still apply cleanly;
- every open or mitigated entry in `docs/dev/UPSTREAM-ISSUES.md`: re-run its
  reproduction against the new release, and update its status, the clingox
  workaround and the guide's known-issues page.
- the theory term printer: `Display for TheoryTerm` follows gringo's private
  `TheoryData::printTerm` (`libgringo/src/output/theory.cc`) rather than
  calling clingo, because a resolved term keeps no handle. Diff that file
  between the two releases. The unit test
  `theory::tests::displayed_compounds_match_clingos_own_term_to_string`
  compares the two printers and must pass on the new release.

### 1.6 Update the ported tests

Diff upstream's own tests between the two tags:

```sh
git -C clingox-sys/clingo diff --stat v5.8.2 v5.9.0 -- \
    examples/c libclingo/tests libpyclingo/clingo/tests app/clingo/tests/lp
```

For each changed file, update the matching port under `clingox/tests/conformance`
(the header of each port names its source). Update the `Ported from … v5.9.0` line
in every port, including unchanged ones, so the headers always name the version they
were checked against. Copy new `.lp`/`.sol` fixtures.

### 1.7 Wrap, test, document

For every added or changed function, follow RULES §3 (definition of done). Update
COVERAGE.md. `cargo xtask coverage` must pass with no missing rows.

### 1.8 Run everything

```sh
cargo xtask check
cargo xtask test linux
cargo xtask test android
cargo xtask test wasm
cargo xtask sanitize
```

All must pass. Then run the negative controls for any safety rule the upgrade
touched.

### 1.9 Update the version and the documents

- The release pull request (`docs/dev/RELEASING.md`) sets the version of the three
  crates and the `=` requirements between them, by the rule of 1.1. Within `508.2.x`
  no breaking change of the Rust API is allowed; a change that breaks it waits for
  the next clingo version.
- System library range in `clingox-sys/build.rs` and DESIGN §5.1.
- README version table, DESIGN §1 target line, COVERAGE.md header.
- Changelog: the clingo version, the API changes found in 1.4, new wrappers, and
  any breaking change with a migration note.

### 1.10 Merge

The PR description lists: the clingo version, the version decision and its reason,
the API diff summary, the ported tests that changed, and the negative controls run.

## 2. A new Emscripten SDK

Rust's `wasm32-unknown-emscripten` target works only with compatible emsdk versions,
so the SDK is pinned in `xtask/emsdk-version` and upgraded deliberately.

1. Check the Rust release notes and the emscripten target documentation for the
   supported emsdk range.
2. Change `xtask/emsdk-version`, run `cargo xtask setup wasm`.
3. Run `cargo xtask test wasm` and `cargo xtask test wasm --browser`.
4. Confirm specifically that the error-path tests pass (parse error, callback error,
   callback panic). A wrong exception mode passes every happy-path test and aborts on
   the first error, which is the failure the spike found.
5. Record the new size of the minimal WASM module in the PR. A jump of more than 10%
   needs an explanation.

## 3. A new Rust toolchain

The MSRV is the latest stable Rust (DESIGN §8.3).

1. `rustup update`.
2. Set `rust-version` in `[workspace.package]` to the new stable.
3. Run `cargo xtask check` and the full test suite.
4. Regenerate the trybuild `.stderr` files if compiler messages changed, and review
   the diff: a changed message is fine, a case that now compiles is a regression.
   Do it on the new release with `rust-src` installed, then set the release in
   `xtask/compile-fail-toolchain` to it, in the same commit.
5. Changelog: "MSRV raised to 1.x".

## 4. Dependencies

1. `cargo update` for compatible updates, `cargo upgrade` (cargo-edit) for new major
   versions. Never edit version numbers by hand.
2. Read the changelog of every dependency that changed major version.
3. `cargo deny check`, then the full test suite.
4. One commit per dependency that needed code changes, so each can be reverted alone.

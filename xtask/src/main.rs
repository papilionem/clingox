//! Repository tasks: binding generation, checks, per-target test runs,
//! coverage.
//!
//! Every check that CI will run later is a subcommand here, so local runs and
//! CI runs are the same commands (DESIGN 8.3, TESTING 7).

// xtask is a command-line tool: what it prints is its interface.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod ast_codegen;
mod bindings;
mod budget;
mod check;
mod conformance;
mod coverage;
mod crate_metadata;
mod package_lists;
mod sanitize;
mod semver;
mod setup;
mod test;
mod util;

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  bindgen         regenerate clingox-sys/src/bindings.rs
  ast-codegen     regenerate clingox/src/ast/generated.rs and
                  clingox/src/raw/ast_generated.rs from clingo's AST
                  constructor table
  check           fmt, clippy, doc, doctests (with the guide and README),
                  bindings, generated AST layer, submodule, deny, unsafe budget, guide and package
                  checks, and the package lists
  test linux      cargo test --workspace on the host, including systest
  test compile-fail
                  the trybuild tests of clingox, on the Rust release their
                  snapshots are made with (xtask/compile-fail-toolchain); the
                  other test commands skip them when CLINGOX_SKIP_COMPILE_FAIL is set
  test android    run the tests on the running x86_64 Android emulator
  test wasm       run the tests for wasm32-unknown-emscripten under Node.js
  test wasm --browser <chromium|firefox|webkit|all> [--timeout <seconds>]
                  run the same tests in headless browsers (default timeout
                  120 s per test executable)
  test all        run `test linux`, `test android` and `test wasm` at the
                  same time, each exactly as its own command would; output is
                  captured per suite and printed as each finishes
  miri            the unit tests of clingox, including the unsafe primitives
                  and trampolines, under Miri (nightly)
  sanitize        the tests of clingox and clingox-sys under AddressSanitizer
                  and LeakSanitizer, then the thread tests under
                  ThreadSanitizer (nightly, Linux x86_64)
  sanitize [--address | --thread] [--test <name>]...
                  one sanitizer only, and only the named test files (each
                  under every sanitizer that runs), with the full run's flags
  crate-metadata  the manifest fields crates.io checks on publish and cargo
                  does not: keywords, categories, description, licence, URLs,
                  readme (part of `check`)
  package-lists [--bless]
                  compare the files each published crate ships with
                  xtask/package-lists/<crate>.txt (part of `check`), or
                  rewrite those lists with --bless
  semver [--baseline-rev <rev>]
                  cargo semver-checks for the three crates against <rev>, or
                  the tag in xtask/semver-baseline (skipped while that tag
                  does not exist)
  setup miri      install nightly Rust with Miri
  setup sanitize  install nightly Rust with the standard library source
  setup wasm      install the emsdk version pinned in xtask/emsdk-version
  setup browser   install the browser test tooling and Playwright's browsers
  coverage        regenerate docs/dev/COVERAGE.md from clingo.h
  conformance-count
                  recount the upstream conformance inventory from the
                  submodule and check it against
                  clingox/tests/conformance/NOT_PORTED.md
  conformance-count --old-unit
                  print the totals of the earlier hand-written inventory for comparison
                  (does not fail the build)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["bindgen"] => bindings::write(),
        ["ast-codegen"] => ast_codegen::write(),
        ["check"] => check::run(),
        ["test", "linux"] => test::linux(),
        ["test", "compile-fail"] => test::compile_fail(),
        ["test", "android"] => test::android(),
        ["test", "wasm", options @ ..] => test::wasm(options),
        ["test", "all"] => test::all(),
        ["miri"] => test::miri(),
        ["sanitize", options @ ..] => sanitize::run_selected(options),
        ["crate-metadata"] => crate_metadata::check(),
        ["package-lists"] => package_lists::run(false),
        ["package-lists", "--bless"] => package_lists::run(true),
        ["semver"] => semver::run(None),
        ["semver", "--baseline-rev", rev] => semver::run(Some(rev)),
        ["setup", "miri"] => setup::miri(),
        ["setup", "sanitize"] => sanitize::setup(),
        ["setup", "wasm"] => setup::wasm(),
        ["setup", "browser"] => setup::browser(),
        ["coverage"] => coverage::run(),
        ["conformance-count"] => conformance::run(),
        ["conformance-count", "--old-unit"] => conformance::old_unit(),
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: {err}");
            ExitCode::FAILURE
        }
    }
}

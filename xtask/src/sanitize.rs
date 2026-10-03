//! `cargo xtask sanitize`: the test suite under the address and leak
//! sanitizers, then the thread tests under the thread sanitizer (RULES 2,
//! TESTING 5).
//!
//! Rust and clingo's C++ are instrumented together. Rust uses nightly's
//! `-Zsanitizer` for the host target, named explicitly so that build scripts
//! and the derive macro stay uninstrumented, and its runtime is the one linked.
//! clingo is built by clang with the same `-fsanitize` flag through a `CMake`
//! toolchain file, whose only job beyond naming the compilers is to make
//! `CMake`'s compiler checks build static libraries: a sanitized test program
//! does not link without the runtime that rustc brings.
//!
//! Each sanitizer builds in its own target directory, so the instrumented
//! clingo never mixes with the normal build. The runs go one after the
//! other, never in parallel, to keep the memory use of the builds bounded.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{Result, root, run};

const TARGET: &str = "x86_64-unknown-linux-gnu";

/// The test files of `clingox` that do not run under a sanitizer:
/// `compile_fail` runs the compiler, not clingo, and `user_docs` runs the
/// guide's examples, which the doctests already cover.
/// `application_printer_raw` pins raw clingo's SIGSEGV when a printer reads a
/// model without a `main` (U40); `ASan` turns that fault into an abort, so the
/// test cannot see its signal. `regression_ast_refcount` (A1) forgets 2^32
/// clones in a child that must die of SIGABRT: instrumented, that would run for
/// hours, and the abort is the behaviour under test, not a finding.
const SKIPPED: [&str; 4] = [
    "compile_fail",
    "user_docs",
    "application_printer_raw",
    "regression_ast_refcount",
];

/// The thread tests that run under the thread sanitizer.
const THREAD_TESTS: [&str; 20] = [
    "api_threads",
    "patch_u19_statistics_registry",
    "interrupt_races",
    "api_interrupt",
    "api_async",
    "api_solve_options",
    "threads_optimisation",
    "solve_events_interrupt_races",
    // The timeout thread retrying an interrupt until a late search starts.
    "timeout_before_the_search_starts",
    "api_statistics_writing",
    // Races only TSan catches reliably (`AsyncSolveHandle::core` racing the
    // end of the search, and `Control::is_conflicting`/the enable getters
    // racing a forgotten async search).
    "async_core_waits",
    "is_conflicting_forgotten_async_search",
    // DESIGN S11's own gate: tests that force backjumps with 1, 2 and 8 threads
    // must pass under TSan (forced-backjump reentrancy test and the
    // PerThread-shaped data-race probe).
    "propagator_reentrancy",
    // `Model::thread_id` and `decide` at several solver threads.
    "api_model_thread_id",
    "api_propagator_decide_heuristic",
    // Runs from several threads at once, one proceeding and the others
    // refused by the process-wide run flag, next to an unrelated `Control`.
    "api_application",
    // A scoped thread drives the application's control inside `main`, and
    // another thread interrupts a search that `main` started.
    "application_main_threads",
    // A `Flag` read from a thread while and after a run, and the option
    // callbacks checked to run on the calling thread with `-t 4`.
    "application_options_threads",
    // The model printer on solver threads (never two calls at once, off
    // the calling thread for an async solve), and a failing printer at `-t4`
    // followed by updates of the control (the U26 pattern).
    "application_printer_threads",
    // A blocking solve runs on the calling thread when no handle exists
    // and on clasp's thread otherwise; the test switches between the two
    // while another thread interrupts, 500 times on one control.
    "blocking_solve_mode",
];

/// The suppressions for the races inside clasp that UPSTREAM-ISSUES records.
const TSAN_SUPPRESSIONS: &str = "xtask/tsan-suppressions.txt";

/// Which sanitizer a run uses.
#[derive(Clone, Copy)]
enum Sanitizer {
    Address,
    Thread,
}

impl Sanitizer {
    fn name(self) -> &'static str {
        match self {
            Sanitizer::Address => "address",
            Sanitizer::Thread => "thread",
        }
    }
}

pub(crate) fn run_all() -> Result<()> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err("`cargo xtask sanitize` runs on a Linux x86_64 host only".into());
    }
    ensure_nightly()?;
    ensure_symbolizer()?;
    address()?;
    thread()?;
    eprintln!("sanitize: the address, leak and thread sanitizers found nothing");
    Ok(())
}

/// Every test of `clingox` and `clingox-sys`, except [`SKIPPED`], with the
/// address sanitizer and leak detection.
fn address() -> Result<()> {
    let mut cmd = sanitized_cargo(Sanitizer::Address)?;
    cmd.env(
        "ASAN_OPTIONS",
        "detect_leaks=1:halt_on_error=1:abort_on_error=0",
    )
    .env("LSAN_OPTIONS", "report_objects=1");
    cmd.args(["-p", "clingox-sys", "-p", "clingox", "--lib"]);
    for test in test_files()? {
        cmd.args(["--test", &test]);
    }
    run(&mut cmd)
}

/// Fails unless `llvm-symbolizer` is on `PATH`. `ThreadSanitizer` names the
/// global that a race touches only through it, and the suppression of the
/// race on clasp's shared `trueAtom_g` matches that name: without the tool the
/// same race is reported as an unnamed one and fails the run, which reads as a
/// new race and is not one.
fn ensure_symbolizer() -> Result<()> {
    let found = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join("llvm-symbolizer").is_file())
    });
    if found {
        Ok(())
    } else {
        Err(
            "`llvm-symbolizer` is not on PATH; ThreadSanitizer needs it to match the \
             suppressions that name a global (install LLVM's tools, e.g. the `llvm` \
             package)"
                .into(),
        )
    }
}

/// The thread tests, with the suppressions for clasp's known races.
fn thread() -> Result<()> {
    let suppressions = root().join(TSAN_SUPPRESSIONS);
    let mut cmd = sanitized_cargo(Sanitizer::Thread)?;
    cmd.env(
        "TSAN_OPTIONS",
        format!(
            "suppressions={} halt_on_error=1 second_deadlock_stack=1",
            suppressions.display()
        ),
    )
    // std must be instrumented too, or its synchronisation looks like races.
    .args(["-Zbuild-std", "-p", "clingox"]);
    for test in THREAD_TESTS {
        cmd.args(["--test", test]);
    }
    run(&mut cmd)
}

/// `cargo +nightly test` for the host target with `sanitizer` on both sides,
/// in a target directory of its own.
fn sanitized_cargo(sanitizer: Sanitizer) -> Result<Command> {
    let name = sanitizer.name();
    let target_dir = root().join("target").join(format!("sanitize-{name}"));
    std::fs::create_dir_all(&target_dir)?;
    let toolchain = write_toolchain_file(&target_dir)?;
    let target_env = TARGET.replace('-', "_");
    let cflags = format!("-fsanitize={name} -fno-omit-frame-pointer -g");

    let mut cmd = Command::new("rustup");
    cmd.current_dir(root())
        .args(["run", "nightly", "cargo", "test", "--target", TARGET])
        // Every test file runs, so one failure does not hide a later report.
        .arg("--no-fail-fast")
        .arg("--target-dir")
        .arg(&target_dir)
        // With `--target`, RUSTFLAGS reaches only the target's crates, not
        // build scripts or the derive macro.
        .env(
            "RUSTFLAGS",
            format!("-Zsanitizer={name} -Cforce-frame-pointers=yes"),
        )
        .env("CMAKE_TOOLCHAIN_FILE", &toolchain)
        .env(format!("CC_{target_env}"), "clang")
        .env(format!("CXX_{target_env}"), "clang++")
        .env(format!("CFLAGS_{target_env}"), &cflags)
        .env(format!("CXXFLAGS_{target_env}"), &cflags);
    crate::util::use_ccache_if_found(&mut cmd);
    Ok(cmd)
}

/// The `CMake` toolchain file for an instrumented clingo. The `cmake` crate
/// drops `-g` from the flags it passes, so debug information is asked for
/// here, for line numbers in reports.
fn write_toolchain_file(dir: &Path) -> Result<PathBuf> {
    let path = dir.join("sanitize-toolchain.cmake");
    std::fs::write(
        &path,
        "# Written by `cargo xtask sanitize`.\n\
         set(CMAKE_C_COMPILER clang)\n\
         set(CMAKE_CXX_COMPILER clang++)\n\
         # A sanitized test program needs the runtime rustc links later.\n\
         set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)\n\
         set(CMAKE_C_FLAGS_RELEASE_INIT \"-g\")\n\
         set(CMAKE_CXX_FLAGS_RELEASE_INIT \"-g\")\n",
    )?;
    Ok(path)
}

/// The names of the test files of `clingox` and `clingox-sys`, less
/// [`SKIPPED`].
fn test_files() -> Result<Vec<String>> {
    let mut names = Vec::new();
    for dir in ["clingox/tests", "clingox-sys/tests"] {
        for entry in std::fs::read_dir(root().join(dir))? {
            let path = entry?.path();
            let is_rust = path.extension().is_some_and(|ext| ext == "rs");
            if let (true, Some(stem)) = (is_rust, path.file_stem().and_then(|s| s.to_str()))
                && !SKIPPED.contains(&stem)
            {
                names.push(stem.to_owned());
            }
        }
    }
    names.sort();
    if names.is_empty() {
        return Err("found no test files to run under the sanitizers".into());
    }
    Ok(names)
}

/// Installs nightly Rust with its standard library source unless it is there.
fn ensure_nightly() -> Result<()> {
    let sysroot = Command::new("rustup")
        .args(["run", "nightly", "rustc", "--print", "sysroot"])
        .output();
    let has_source = sysroot.is_ok_and(|out| {
        out.status.success()
            && Path::new(String::from_utf8_lossy(&out.stdout).trim())
                .join("lib/rustlib/src/rust/library")
                .is_dir()
    });
    if has_source { Ok(()) } else { setup() }
}

/// Nightly Rust with the standard library source that `-Zbuild-std` needs.
pub(crate) fn setup() -> Result<()> {
    run(Command::new("rustup").args([
        "toolchain",
        "install",
        "nightly",
        "--profile",
        "minimal",
        "--component",
        "rust-src",
    ]))?;
    eprintln!("nightly Rust with rust-src is installed for `cargo xtask sanitize`");
    Ok(())
}

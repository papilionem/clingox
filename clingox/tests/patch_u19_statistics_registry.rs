//! Patch U19 (`clingox-sys/patches/U19-statistics-type-registry.patch`):
//! clasp registers each kind of statistics object in a global table the
//! first time it is used, and the table must not grow under a reader on
//! another thread (UPSTREAM-ISSUES U19).
//!
//! Unpatched, the table is a growable vector: the first use of two kinds of
//! statistics on two threads at once appends to it without a lock, and a
//! thread that reads statistics meanwhile indexes a vector that may be
//! reallocating. That is a data race and a possible use-after-free, which
//! the thread sanitizer reports in about 3 of 10 runs of `api_async`.
//!
//! Registration happens once per process, so each attempt needs a fresh
//! process: the test runs this binary again as a child, many times. In each
//! child, threads released together create controls with different solver
//! settings, solve and read all their statistics, so that many kinds are
//! registered and read at once. Under `cargo xtask sanitize` a race makes the
//! child exit with the thread sanitizer's error status, and the test fails. Without
//! a sanitizer, a child that reads freed memory may crash or see the wrong
//! kind of object.
//!
//! Spawning a child needs a host with processes, so the file is host-only. A
//! system library does not get the patch (RULES 8); on one, the test returns
//! early.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail loudly on unexpected errors"
)]
#![cfg(not(any(target_os = "android", target_os = "ios", target_family = "wasm")))]

use std::io::Write;
use std::process::Command;
use std::sync::Barrier;
use std::thread;

use clingox::{Control, Part, StatsTree};

/// The environment variable that puts this binary in child mode.
const CHILD: &str = "CLINGOX_TEST_U19_CHILD";
/// What a child prints when its threads have finished.
const DONE: &str = "u19-child-done";
const RUNS: usize = 20;
const THREADS: usize = 8;

/// Solver settings and programs that bring different statistics with them:
/// more solver threads add per-thread and parallel entries, `--stats=2` the
/// detailed ones, and optimisation the cost entries.
///
/// Optimisation runs with one solver thread only: with several, clasp races
/// on the shared optimum itself (a separate upstream issue), which the thread
/// sanitizer would report here and hide what this test is about.
const SETTINGS: [(&[&str], &str); 4] = [
    (&["--stats=2"], OPTIMISE),
    (&["--stats=2", "--parallel-mode=2"], CHOOSE),
    (&["--stats=1", "--opt-mode=optN"], OPTIMISE),
    (&["--stats=2", "--parallel-mode=3", "--models=0"], CHOOSE),
];

const CHOOSE: &str = "{a;b;c}. :- a, b.";
const OPTIMISE: &str = "{a;b;c}. :- a, b. :~ a. [1] :~ not c. [2]";

/// The number of entries of a statistics tree, so that every entry is read.
fn count(tree: &StatsTree) -> usize {
    match tree {
        StatsTree::Map(entries) => 1 + entries.iter().map(|(_, t)| count(t)).sum::<usize>(),
        StatsTree::Array(items) => 1 + items.iter().map(count).sum::<usize>(),
        _ => 1,
    }
}

/// The child: threads create controls, solve, and read their statistics, all
/// at the same moment.
#[test]
fn child_registers_statistics_on_many_threads_at_once() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let start = Barrier::new(THREADS);
    thread::scope(|scope| {
        for worker in 0..THREADS {
            let start = &start;
            scope.spawn(move || {
                let (args, program) = SETTINGS[worker % SETTINGS.len()];
                let mut ctl = Control::with_args(args).unwrap();
                ctl.add_base(program).unwrap();
                ctl.ground(&[Part::base()]).unwrap();
                start.wait();
                assert!(ctl.solve(&[]).unwrap().is_sat());
                let tree = ctl.statistics().unwrap().snapshot().unwrap();
                assert!(count(&tree) > 10, "{tree:?}");
            });
        }
    });
    // The parent checks this marker on the child's standard output.
    let mut out = std::io::stdout();
    writeln!(out, "{DONE}").expect("stdout is a pipe to the parent test");
}

fn patched() -> bool {
    if clingox_sys::VENDORED {
        assert!(
            clingox_sys::PATCHES.contains(&"U19"),
            "the vendored build applies U19: {:?}",
            clingox_sys::PATCHES
        );
    }
    clingox_sys::VENDORED
}

#[test]
fn registering_statistics_on_many_threads_at_once_never_races() {
    if std::env::var_os(CHILD).is_some() || !patched() {
        return;
    }
    let exe = std::env::current_exe().expect("the test binary knows its path");
    let mut failures = Vec::new();
    for run in 0..RUNS {
        let output = Command::new(&exe)
            .args([
                "--exact",
                "child_registers_statistics_on_many_threads_at_once",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .output()
            .expect("the test binary runs again");
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || !stdout.contains(DONE) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let report: String = stderr.lines().take(40).collect::<Vec<_>>().join("\n");
            failures.push(format!("run {run}: {:?}\n{report}", output.status));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {RUNS} children failed:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

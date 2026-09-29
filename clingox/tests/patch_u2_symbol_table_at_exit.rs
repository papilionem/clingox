//! Patch U2 (`clingox-sys/patches/U2-leak-symbol-table.patch`): clingo's
//! symbol table is never destroyed, not even when the process exits, so
//! symbols and their borrowed text stay valid for as long as any thread runs
//! (UPSTREAM-ISSUES U2, DESIGN S12).
//!
//! The test runs this binary again as a child process, in a mode where a
//! thread keeps using symbols while the process exits. `std::process::exit`
//! runs the C++ static destructors exactly as returning from `main` does.
//! Without the patch, a stress run saw 144 of 200 such runs segfault when the
//! thread creates symbols, and 39 of 300 read corrupted text when it only
//! reads them; each mode here runs 40 children, so an unpatched build fails
//! with near certainty. A child that sees corrupted text aborts, so that it
//! fails even when the exit code would otherwise hide it.
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
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clingox::Symbol;

/// The environment variable that puts this binary in child mode, with the
/// mode as its value.
const CHILD: &str = "CLINGOX_TEST_U2_CHILD";
/// What a child prints once its thread is busy, just before it exits.
const READY: &str = "u2-child-ready";
const RUNS: usize = 40;

fn name(i: usize) -> String {
    format!("u2_symbol_{i}")
}

/// Uses symbols until the process ends: `create` makes new ones and reads
/// them back, `read` reads a fixed set of them. Wrong text aborts.
fn busy(mode: &str, progress: &AtomicUsize) {
    let fixed: Vec<(Symbol, String)> = (0..2000)
        .map(|i| {
            let symbol = Symbol::function(&name(i), &[]).expect("a valid constant name");
            (symbol, name(i))
        })
        .collect();
    let mut i = 0usize;
    loop {
        if mode == "create" {
            let expected = name(2000 + i);
            let symbol = Symbol::function(&expected, &[]).unwrap();
            if symbol.name() != Some(expected.as_str()) {
                std::process::abort();
            }
        } else {
            let (symbol, expected) = &fixed[i % fixed.len()];
            if symbol.name() != Some(expected.as_str()) || symbol.to_string() != *expected {
                std::process::abort();
            }
        }
        i = i.wrapping_add(1);
        progress.fetch_add(1, Ordering::Relaxed);
    }
}

/// The child: a thread uses symbols while the process exits. In a normal run
/// of this binary the variable is unset and the test does nothing.
#[test]
fn child_uses_symbols_while_the_process_exits() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    let progress = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&progress);
    thread::spawn(move || busy(&mode, &counter));
    let started = Instant::now();
    while progress.load(Ordering::Relaxed) < 1000 && started.elapsed() < Duration::from_secs(30) {
        thread::yield_now();
    }
    // The parent waits for this marker on the child's standard output.
    let mut out = std::io::stdout();
    writeln!(out, "{READY}").expect("stdout is a pipe to the parent test");
    out.flush().expect("stdout is a pipe to the parent test");
    std::process::exit(0);
}

fn run_children(mode: &str) {
    let exe = std::env::current_exe().expect("the test binary knows its path");
    let mut failures = Vec::new();
    for run in 0..RUNS {
        let output = Command::new(&exe)
            .args([
                "--exact",
                "child_uses_symbols_while_the_process_exits",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, mode)
            .output()
            .expect("the test binary runs again");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(READY),
            "child {run} ({mode}) never got its thread going: {stdout}"
        );
        if !output.status.success() {
            failures.push(format!("run {run}: {:?}", output.status));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {RUNS} children ({mode}) crashed while exiting: {failures:?}",
        failures.len()
    );
}

fn patched() -> bool {
    if clingox_sys::VENDORED {
        assert!(
            clingox_sys::PATCHES.contains(&"U2"),
            "the vendored build applies U2: {:?}",
            clingox_sys::PATCHES
        );
    }
    clingox_sys::VENDORED
}

#[test]
fn creating_symbols_while_the_process_exits_never_crashes() {
    if std::env::var_os(CHILD).is_some() || !patched() {
        return;
    }
    run_children("create");
}

#[test]
fn reading_symbols_while_the_process_exits_never_crashes() {
    if std::env::var_os(CHILD).is_some() || !patched() {
        return;
    }
    run_children("read");
}

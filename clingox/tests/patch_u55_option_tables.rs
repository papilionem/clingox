//! Patch U55 (`clingox-sys/patches/U55-option-value-tables.patch`): clingo
//! keeps the names of some enumerated option values (`--output-debug`,
//! `--output`, `--mode`) in process-wide tables and appended to them every time
//! it registered its options, without a lock (UPSTREAM-ISSUES U55).
//! `clingo_control_new` serializes the registration with a mutex of its own,
//! but `clingo_main` does not, so `Application::run` on one thread and
//! `Control::new` on another wrote to the same vector at once. The CI run of
//! the 508.2.0-beta.4 release found it as heap corruption in
//! `conformance_pyclingo`, whose tests do exactly that in parallel.
//!
//! Without the patch the stress test below corrupts the heap: the process
//! aborted with glibc's `double free or corruption` or died of SIGSEGV in 20
//! of 20 runs (checked; 300 rounds instead of 3000 failed 11 of 20).
//! `AddressSanitizer` names the double free in `ValueMappingBase::add`, and the
//! thread sanitizer runs this file (`cargo xtask sanitize`).
//!
//! A system clingo has no patch (RULES 8): the tests return early there. On a
//! build without threads nothing runs at the same time.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test helpers fail loudly on unexpected errors, and say why they skip"
)]

#[path = "common/child.rs"]
mod child;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use clingox::Control;
use clingox::application::Application;

#[test]
fn the_vendored_build_applies_u55() {
    if !clingox_sys::VENDORED {
        eprintln!("SKIPPED: a system clingo has no option table patch (RULES 8)");
        return;
    }
    assert!(
        clingox_sys::PATCHES.contains(&"U55"),
        "the vendored build applies U55: {:?}",
        clingox_sys::PATCHES
    );
}

#[test]
fn controls_created_while_an_application_runs_leave_the_heap_intact() {
    if !clingox_sys::VENDORED {
        eprintln!("SKIPPED: a system clingo keeps U55 (RULES 8)");
        return;
    }
    if !clingox_sys::HAS_THREADS {
        eprintln!("SKIPPED: this build has no threads");
        return;
    }
    let file = child::fixture("u55.lp", "a. {b}.");
    let stop = Arc::new(AtomicBool::new(false));
    let creators: Vec<_> = (0..3)
        .map(|_| {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                let mut created = 0_u32;
                while !stop.load(Ordering::Relaxed) {
                    drop(Control::new().unwrap());
                    created += 1;
                }
                created
            })
        })
        .collect();
    for _ in 0..3000 {
        let code = Application::new()
            .run([child::arg(&file), "--outf=3".to_owned()])
            .unwrap();
        assert_eq!(code, 10);
    }
    stop.store(true, Ordering::Relaxed);
    for creator in creators {
        assert!(creator.join().unwrap() > 0, "each thread created controls");
    }
}

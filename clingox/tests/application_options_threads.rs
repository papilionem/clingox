//! The options callbacks and threads.
//!
//! - `Flag` is `Send + Sync`, so another thread may poll it while clingo parses
//!   the command line. clasp writes its target with a plain store, so the run
//!   must give clasp a private cell and publish into the `Flag` with an atomic
//!   store; a plain write into the flag's own memory is a data race that `TSan`
//!   reports here (`cargo xtask sanitize`, the file is in `THREAD_TESTS`).
//! - `register_options`, every `parse` and `validate_options` run on the thread
//!   that called `run`, also when clingo solves with four threads.
//!
//! Both tests need threads and are skipped, visibly, on a build without them
//! (WebAssembly without atomics). Expected values: pyclingo 5.8.2.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test helpers fail loudly, and a skipped case says so"
)]

#[path = "common/child.rs"]
mod child;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, ThreadId};

use clingox::application::{Application, Flag, OptionSpec};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether this build can start a thread; says so on standard error if not.
fn threads_available(test: &str) -> bool {
    if clingox_sys::HAS_THREADS {
        return true;
    }
    eprintln!("SKIPPED: {test}: this build of clingo has no threads");
    false
}

#[test]
fn a_flag_polled_by_another_thread_during_the_run_is_race_free() {
    if !threads_available("a_flag_polled_by_another_thread_during_the_run_is_race_free") {
        return;
    }
    let _lock = serial();
    for _ in 0..10 {
        let flag = Flag::new(false);
        let stop = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(AtomicBool::new(false));
        let reader = {
            let (flag, stop, ready) = (flag.clone(), Arc::clone(&stop), Arc::clone(&ready));
            thread::spawn(move || {
                ready.store(true, Ordering::Release);
                let mut polls = 0_u64;
                while !stop.load(Ordering::Acquire) {
                    // The value itself is not the point: the loads must not
                    // race with clasp's store.
                    let _ = flag.get();
                    polls += 1;
                    if polls.is_multiple_of(64) {
                        thread::yield_now();
                    }
                }
                flag.get()
            })
        };
        while !ready.load(Ordering::Acquire) {
            thread::yield_now();
        }
        let code = Application::new()
            .register_options(|o| o.add_flag("Test", "zzfl", "a flag", &flag))
            .main(|_ctl, _files| Ok(()))
            .run(["--outf=3", "--zzfl"])
            .unwrap();
        stop.store(true, Ordering::Release);
        let last_seen = reader.join().unwrap();
        assert_eq!(code, 0);
        assert!(flag.get());
        // The run has returned and the reader joined after it: it saw the
        // result.
        assert!(last_seen);
    }
}

#[test]
fn the_options_callbacks_run_on_the_calling_thread_with_four_solver_threads() {
    if !threads_available(
        "the_options_callbacks_run_on_the_calling_thread_with_four_solver_threads",
    ) {
        return;
    }
    let _lock = serial();
    let file = child::fixture("opts_threads.lp", "a. {b}.");
    let path = child::arg(&file);
    let caller = thread::current().id();
    let seen: RefCell<Vec<(&str, ThreadId)>> = RefCell::new(Vec::new());
    let code = Application::new()
        .register_options(|o| {
            seen.borrow_mut().push(("register", thread::current().id()));
            o.add(OptionSpec::new("Test", "zzo", "an option"), |_| {
                seen.borrow_mut().push(("parse", thread::current().id()));
                Ok(())
            })
        })
        .validate_options(|| {
            seen.borrow_mut().push(("validate", thread::current().id()));
            Ok(())
        })
        // clingo's own main solves with the four threads.
        .run([path.as_str(), "0", "--outf=3", "-t", "4", "--zzo=1"])
        .unwrap();
    assert_eq!(code, 30);
    let seen = seen.into_inner();
    let names: Vec<&str> = seen.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["register", "parse", "validate"]);
    assert!(seen.iter().all(|(_, id)| *id == caller), "{seen:?}");
}

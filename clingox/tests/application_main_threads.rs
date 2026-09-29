//! Threads and the application's control, run under `ThreadSanitizer` by `cargo
//! xtask sanitize` (this file is in `THREAD_TESTS`).
//!
//! The control lent to `main` is `Send` like any other, so a scoped thread may
//! drive it while the callback waits; the scope ends before the callback
//! returns, which is the only way a thread can hold it. A thread that outlives
//! the callback is a compile error
//! (`tests/ui/application_main_spawn_control.rs`).

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "tests assert on invariants, and the watchdog explains its abort"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use clingox::Part;
use clingox::application::Application;

const PIGEONS: &str =
    "p(1..11). h(1..10). 1 {in(P,H): h(H)} 1 :- p(P). :- in(P1,H), in(P2,H), P1 < P2.";

const LIMIT: Duration = Duration::from_secs(30);

static SERIAL: Mutex<()> = Mutex::new(());

/// Aborts the test process if a test runs longer than `LIMIT`: a search left
/// open and not closed by the borrowed control's `Drop` hangs `run`, and a
/// hung test would stall CI with no message. The watchdog thread ends when the
/// guard is dropped. Without threads (WebAssembly) there is no watchdog.
struct Watchdog(Option<std::sync::mpsc::Sender<()>>);

impl Watchdog {
    fn start() -> Watchdog {
        if !clingox_sys::HAS_THREADS {
            return Watchdog(None);
        }
        let (done, wait) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if wait.recv_timeout(LIMIT) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                eprintln!("the test hung for {LIMIT:?}: run did not return; aborting the process");
                std::process::abort();
            }
        });
        Watchdog(Some(done))
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Some(done) = self.0.take() {
            let _ = done.send(());
        }
    }
}

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
fn a_scoped_thread_may_drive_the_control_inside_main() {
    let _lock = serial();
    let _dog = Watchdog::start();
    // A scoped thread needs threads (WebAssembly builds have none).
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let mut models = None;
    Application::new()
        .main(|ctl, _files| {
            std::thread::scope(|scope| {
                let control = &mut *ctl;
                scope
                    .spawn(move || {
                        control.add_base("a. {b}.")?;
                        control.ground(&[Part::base()])?;
                        let _ = control.solve(&[])?;
                        Ok::<(), clingox::Error>(())
                    })
                    .join()
                    .expect("the scoped thread does not panic")
            })?;
            // Back on the callback's thread, the control is as it was left.
            let (result, all) = ctl.solve_all()?;
            models = Some((result.is_sat(), all.len()));
            Ok(())
        })
        .run(["--outf=3", "0"])
        .unwrap();
    assert_eq!(models, Some((true, 2)));
}

#[test]
fn an_interrupt_from_another_thread_stops_a_search_started_in_main() {
    let _lock = serial();
    let _dog = Watchdog::start();
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let accepted = AtomicBool::new(false);
    let mut interrupted = None;
    Application::new()
        .main(|ctl, _files| {
            ctl.add_base(PIGEONS)?;
            ctl.ground(&[Part::base()])?;
            let stop = ctl.interrupt_handle();
            let handle = ctl.solve_async(&[])?;
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    // Retry until the search is running: the handle answers
                    // `false` while none is.
                    for _ in 0..200 {
                        if stop.interrupt() {
                            accepted.store(true, Ordering::SeqCst);
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                });
            });
            interrupted = Some(handle.close()?.is_interrupted());
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
    assert!(accepted.load(Ordering::SeqCst));
    assert_eq!(interrupted, Some(true));
}

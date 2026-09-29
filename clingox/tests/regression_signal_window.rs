//! `run` restores the signal
//! dispositions it found as soon as `clingo_main` returns, before any user
//! `Drop` code runs.
//!
//! While a run is active the dispositions are clasp's, and clasp's handler
//! reads a singleton that is null once clingo is done. The observers and
//! propagators a borrowed control kept and the closures of
//! `register_options` are dropped by `run` after clingo returned; a SIGTERM
//! during one of those `Drop`s used to segfault. It now gets the disposition
//! the process had before `run` (here the default, so SIGTERM ends the child).
//! The signal is sent by a `kill` process, so the child is safe Rust. Every
//! case runs in a child; `cfg(unix)` because of the signal.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::print_stdout, reason = "test")]

#[path = "common/child.rs"]
mod child;

use std::time::Duration;

use clingox::Part;
use clingox::application::Application;
use clingox::observer::GroundProgramObserver;

const ENTRY: &str = "child_entry";

/// Sends SIGTERM to its own process when dropped, and waits for it.
struct Tripwire;

impl GroundProgramObserver for Tripwire {}

impl Drop for Tripwire {
    fn drop(&mut self) {
        println!("DROP-START");
        let pid = std::process::id().to_string();
        std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        println!("DROP-END");
    }
}

#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    let code = match case.as_str() {
        // The observer the borrowed control kept is dropped by `run`.
        "arena" => Application::new()
            .main(|ctl, _files| {
                ctl.register_observer(Tripwire, false)?;
                ctl.add_base("a.")?;
                ctl.ground(&[Part::base()])?;
                Ok(())
            })
            .run(["--outf=3"]),
        // The closure of `register_options` is dropped by `run` as well.
        "closure" => {
            let tripwire = Tripwire;
            Application::new()
                .register_options(move |_options| {
                    let _keep = &tripwire;
                    Ok(())
                })
                .main(|_ctl, _files| Ok(()))
                .run(["--outf=3"])
        }
        // The same Drop, after `run` returned: the dispositions are back.
        "after" => {
            let code = Application::new()
                .main(|_ctl, _files| Ok(()))
                .run(["--outf=3"]);
            drop(Tripwire);
            code
        }
        _ => unreachable!(),
    };
    println!("RETURNED {code:?}");
}

fn check(case: &str) -> Option<child::Outcome> {
    let outcome = child::run_child(ENTRY, case)?;
    println!("{case}: {outcome:?}");
    Some(outcome)
}

#[test]
fn a_signal_after_clingo_returned_gets_the_hosts_disposition() {
    let Some(after) = check("after") else {
        return;
    };
    assert_eq!(after.signal, Some(libc::SIGTERM), "control case: {after:?}");
}

#[test]
fn a_signal_while_run_drops_the_kept_observers_is_not_a_crash() {
    let Some(outcome) = check("arena") else {
        return;
    };
    assert!(outcome.stdout.contains("DROP-START"), "{outcome:?}");
    assert_eq!(
        outcome.signal,
        Some(libc::SIGTERM),
        "SIGTERM while run drops the observer must act as before run, not crash: {outcome:?}"
    );
    assert!(!outcome.stdout.contains("DROP-END"), "{outcome:?}");
}

#[test]
fn a_signal_while_run_drops_the_option_closures_is_not_a_crash() {
    let Some(outcome) = check("closure") else {
        return;
    };
    // The capture may also be dropped with the `Application`, after the
    // dispositions are back, which is just as good.
    assert_eq!(
        outcome.signal,
        Some(libc::SIGTERM),
        "SIGTERM during the drop of the closures must act as before run, not crash: {outcome:?}"
    );
    assert!(!outcome.stdout.contains("DROP-END"), "{outcome:?}");
}

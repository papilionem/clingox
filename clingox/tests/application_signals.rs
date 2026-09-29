//! The process-wide effects of `Application::run` that need signals.
//!
//! This is the one test file with `unsafe` (an exception to RULES 2): reading
//! and raising signals and writing to C stdio take `libc` calls, and the check
//! has to be independent of the code it checks. Every case runs in a child
//! process (`common/child.rs`), one run at a time, because the dispositions are
//! clasp's while any `run` is active and because clasp ends the process on
//! these paths. Expected values are from pyclingo 5.8.2.

#![cfg(unix)]
#![allow(
    unsafe_code,
    reason = "signals and C stdio are reachable only through libc"
)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clingox::application::Application;

const ENTRY: &str = "child_entry";
const SAT: &str = "a. {b}.";
const WARN: &str = "a :- b. c :- d. e :- f. g :- h.";
/// The signals clasp installs a handler for, plus SIGALRM for `--time-limit`.
const SIGNALS: [libc::c_int; 9] = [
    libc::SIGINT,
    libc::SIGTERM,
    libc::SIGUSR1,
    libc::SIGUSR2,
    libc::SIGQUIT,
    libc::SIGHUP,
    libc::SIGXCPU,
    libc::SIGXFSZ,
    libc::SIGALRM,
];

#[cfg(any(target_os = "linux", target_os = "android"))]
const SA_RESTORER: libc::c_int = 0x0400_0000;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const SA_RESTORER: libc::c_int = 0;

static HOST_HANDLER_RAN: AtomicBool = AtomicBool::new(false);

extern "C" fn host_handler(_: libc::c_int) {
    HOST_HANDLER_RAN.store(true, Ordering::SeqCst);
}

/// What the kernel holds for one signal.
#[derive(Debug, PartialEq, Eq)]
struct Disposition {
    handler: usize,
    flags: libc::c_int,
    blocked: Vec<libc::c_int>,
}

fn read(signal: libc::c_int) -> Disposition {
    // SAFETY: `sigaction` with a null new action only reads into `old`, a
    // zeroed struct of the right type; `sigismember` reads the set in it.
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        assert_eq!(libc::sigaction(signal, std::ptr::null(), &raw mut old), 0);
        let blocked = (1..=31)
            .filter(|s| libc::sigismember(&raw const old.sa_mask, *s) == 1)
            .collect();
        Disposition {
            handler: old.sa_sigaction,
            // glibc adds SA_RESTORER when it writes an action back, which is
            // meaningless for SIG_DFL and SIG_IGN and not part of the host's
            // choice, so it is left out of the comparison.
            flags: old.sa_flags & !SA_RESTORER,
            blocked,
        }
    }
}

fn install(signal: libc::c_int, handler: libc::sighandler_t) {
    // SAFETY: a valid action (a handler of the right type, or SIG_IGN, an
    // empty mask) for a catchable signal; the old action is not wanted.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&raw mut action.sa_mask);
        assert_eq!(
            libc::sigaction(signal, &raw const action, std::ptr::null_mut()),
            0
        );
    }
}

fn snapshot() -> Vec<Disposition> {
    SIGNALS.iter().map(|s| read(*s)).collect()
}

fn raise(signal: libc::c_int) {
    // SAFETY: raising a signal in the calling thread; every caller has a
    // handler installed that the process survives, or means to end the process.
    assert_eq!(unsafe { libc::raise(signal) }, 0);
}

/// Raises `signal` from the first message clingo logs, then waits for it to
/// take effect.
fn interrupt_from_logger(signal: libc::c_int, extra: &[&str], file: &std::path::Path) {
    let mut args = vec![child::arg(file)];
    args.extend(extra.iter().map(|a| (*a).to_owned()));
    let result = Application::new()
        .logger(|_, _| {
            raise(signal);
            std::thread::sleep(Duration::from_millis(500));
        })
        .run(args);
    // Never reached when the process ends as clasp intends.
    println!("CHILD-RETURNED {result:?}");
}

/// The child side: runs the case named by the environment, or does nothing.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    // A parent started as a background job (nohup, cron, many CI runners)
    // hands down SIGINT and SIGQUIT as ignored, and clasp keeps an ignored
    // signal ignored, so start every case from the default dispositions.
    for signal in SIGNALS {
        install(signal, libc::SIG_DFL);
    }
    let sat = child::fixture("sig_sat.lp", SAT);
    let warn = child::fixture("sig_warn.lp", WARN);
    match case.as_str() {
        "dispositions_restored" => {
            // A host with its own handlers, one ignored signal, the rest
            // default.
            install(libc::SIGTERM, host_handler as *const () as usize);
            install(libc::SIGUSR1, host_handler as *const () as usize);
            install(libc::SIGXFSZ, libc::SIG_IGN);
            let before = snapshot();
            assert_ne!(
                before[1].handler,
                libc::SIG_DFL,
                "the host handler is installed"
            );
            let mut variants = vec![vec![], vec!["--time-limit=100"]];
            if clingox_sys::HAS_THREADS {
                variants.push(vec!["-t2"]);
            }
            for extra in variants {
                let mut args = vec![child::arg(&sat), "--outf=3".to_owned()];
                args.extend(extra.iter().map(|a| (*a).to_owned()));
                let code = Application::new().run(args).unwrap();
                assert_eq!(code, 10);
                assert_eq!(snapshot(), before, "after {extra:?}");
            }
            assert_eq!(read(libc::SIGXFSZ).handler, libc::SIG_IGN);
            // The F4 regression: a signal after a normal run reaches the host.
            raise(libc::SIGTERM);
            println!(
                "CHILD-HANDLER-RAN {}",
                HOST_HANDLER_RAN.load(Ordering::SeqCst)
            );
            println!("CHILD-ALIVE");
        }
        "default_dispositions_restored" => {
            let before = snapshot();
            let args = [child::arg(&sat), "--outf=3".to_owned()];
            assert_eq!(Application::new().run(args).unwrap(), 10);
            assert_eq!(snapshot(), before);
            println!("CHILD-ALIVE");
        }
        "dispositions_restored_after_a_panic" => {
            install(libc::SIGTERM, host_handler as *const () as usize);
            let before = snapshot();
            let args = [child::arg(&warn), "--outf=3".to_owned()];
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Application::new().logger(|_, _| panic!("boom")).run(args)
            }));
            assert!(outcome.is_err());
            assert_eq!(snapshot(), before);
            raise(libc::SIGTERM);
            println!(
                "CHILD-HANDLER-RAN {}",
                HOST_HANDLER_RAN.load(Ordering::SeqCst)
            );
        }
        "sigint_from_logger" => interrupt_from_logger(libc::SIGINT, &[], &warn),
        "sigterm_from_logger" => interrupt_from_logger(libc::SIGTERM, &[], &warn),
        "time_limit" => {
            let pigeons = child::fixture(
                "sig_pigeon.lp",
                "p(1..11). h(1..10). 1 {in(P,H): h(H)} 1 :- p(P). :- in(P1,H), in(P2,H), P1 < P2.",
            );
            let started = std::time::Instant::now();
            let result = Application::new().run([
                child::arg(&pigeons),
                "--time-limit=1".to_owned(),
                "-q".to_owned(),
            ]);
            println!("CHILD-RETURNED {result:?} after {:?}", started.elapsed());
        }
        "pending_output_survives_exit" => {
            print!("rust-partial ");
            // SAFETY: `puts` gets a valid NUL-terminated string.
            assert!(unsafe { libc::puts(c"c-pending".as_ptr()) } >= 0);
            interrupt_from_logger(libc::SIGINT, &["--outf=3"], &warn);
        }
        other => panic!("unknown child case {other}"),
    }
}

fn run(case: &str) -> Option<child::Outcome> {
    child::run_child(ENTRY, case)
}

#[test]
fn the_nine_dispositions_are_restored_and_a_host_handler_still_runs() {
    let Some(out) = run("dispositions_restored") else {
        return;
    };
    assert_eq!(out.code, Some(0), "{out:?}");
    assert!(
        out.stdout.contains("CHILD-HANDLER-RAN true"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("CHILD-ALIVE"), "{}", out.stdout);
}

#[test]
fn default_dispositions_are_restored_too() {
    let Some(out) = run("default_dispositions_restored") else {
        return;
    };
    assert_eq!(out.code, Some(0), "{out:?}");
    assert!(out.stdout.contains("CHILD-ALIVE"), "{}", out.stdout);
}

#[test]
fn the_dispositions_are_restored_when_the_logger_panicked() {
    let Some(out) = run("dispositions_restored_after_a_panic") else {
        return;
    };
    assert_eq!(out.code, Some(0), "{out:?}");
    assert!(
        out.stdout.contains("CHILD-HANDLER-RAN true"),
        "{}",
        out.stdout
    );
}

fn assert_interrupted(out: &child::Outcome) {
    assert_eq!(out.signal, None, "{out:?}");
    assert_eq!(out.code, Some(1), "{out:?}");
    assert!(out.stdout.contains("INTERRUPTED"), "{}", out.stdout);
    assert!(
        out.stderr.contains("INTERRUPTED by signal!"),
        "{}",
        out.stderr
    );
    assert!(!out.stdout.contains("CHILD-RETURNED"), "{}", out.stdout);
}

#[test]
fn sigint_during_a_run_ends_the_process_with_clasps_summary() {
    let Some(out) = run("sigint_from_logger") else {
        return;
    };
    assert_interrupted(&out);
}

#[test]
fn sigterm_during_a_run_ends_the_process_too() {
    let Some(out) = run("sigterm_from_logger") else {
        return;
    };
    assert_interrupted(&out);
}

#[test]
fn a_time_limit_ends_the_process_and_run_never_returns() {
    let Some(out) = run("time_limit") else {
        return;
    };
    assert_eq!(out.code, Some(1), "{out:?}");
    assert!(out.stdout.contains("TIME LIMIT   : 1"), "{}", out.stdout);
    assert!(
        out.stderr.contains("INTERRUPTED by signal!"),
        "{}",
        out.stderr
    );
    assert!(!out.stdout.contains("CHILD-RETURNED"), "{}", out.stdout);
}

#[test]
fn output_written_before_the_run_survives_the_process_exit() {
    let Some(out) = run("pending_output_survives_exit") else {
        return;
    };
    assert_eq!(out.code, Some(1), "{out:?}");
    // Both were still buffered when the run began; the wrapper flushes them.
    assert!(out.stdout.contains("rust-partial "), "{}", out.stdout);
    assert!(out.stdout.contains("c-pending"), "{}", out.stdout);
    assert!(!out.stdout.contains("CHILD-RETURNED"), "{}", out.stdout);
}

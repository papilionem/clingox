//! `clingox::application::Application::run`, in process.
//!
//! Expected values come from pyclingo 5.8.2 (clingo 5.8.2, clasp 3.4.1)
//! calling `clingo_main` with no `main` callback. Every run here passes
//! `--outf=3`, which silences clingo's standard output.
//!
//! `run` refuses a second run while one is in progress, on any thread, so the
//! tests of this file take one lock and never overlap; the tests of the
//! refusal itself create the overlap on purpose. Output, signals and process
//! exits are checked in `application_child.rs` and `application_signals.rs`.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test helpers fail loudly on unexpected errors, and the hang guard explains itself"
)]

#[path = "common/child.rs"]
mod child;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use clingox::application::{Application, exit_code};
use clingox::{Control, ErrorKind, MessageCode, Part};

const SAT: &str = "a. {b}.";
const UNSAT: &str = "a. :- a.";
/// Four atoms that occur in no head: four `AtomUndefined` messages, at columns
/// 6, 14, 22 and 30 (pyclingo 5.8.2).
const WARN: &str = "a :- b. c :- d. e :- f. g :- h.";
const LIMIT: Duration = Duration::from_secs(30);

static SERIAL: Mutex<()> = Mutex::new(());

/// Holds the file-wide lock: no two tests of this file run clingo at once.
fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn quiet(file: &std::path::Path) -> Vec<String> {
    vec![child::arg(file), "--outf=3".to_owned()]
}

fn many_warnings() -> String {
    use std::fmt::Write as _;
    (0..25).fold(String::new(), |mut text, i| {
        write!(text, "a{i} :- b{i}. ").unwrap();
        text
    })
}

fn code(result: clingox::Result<i32>) -> i32 {
    result.unwrap()
}

// ---- exit codes ----

#[test]
fn the_default_application_returns_clingos_exit_codes() {
    let _lock = serial();
    let sat = child::fixture("codes_sat.lp", SAT);
    let unsat = child::fixture("codes_unsat.lp", UNSAT);
    let all = [child::arg(&sat), "0".into(), "--outf=3".into()];
    assert_eq!(code(Application::new().run(all)), 30);
    let number_first = ["0".into(), child::arg(&sat), "--outf=3".into()];
    assert_eq!(
        code(Application::new().run::<[String; 3]>(number_first)),
        30
    );
    assert_eq!(code(Application::new().run(quiet(&sat))), 10);
    assert_eq!(code(Application::new().run(quiet(&unsat))), 20);
    let unsat_all = [child::arg(&unsat), "0".into(), "--outf=3".into()];
    assert_eq!(code(Application::new().run(unsat_all)), 20);
}

#[test]
fn the_exit_code_constants_are_clasps() {
    assert_eq!(exit_code::UNKNOWN, 0);
    assert_eq!(exit_code::INTERRUPTED, 1);
    assert_eq!(exit_code::SATISFIABLE, 10);
    assert_eq!(exit_code::EXHAUSTED, 20);
    assert_eq!(exit_code::MEMORY, 33);
    assert_eq!(exit_code::ERROR, 65);
    assert_eq!(exit_code::NO_RUN, 128);
    // A run that found models and searched everything is both.
    assert_eq!(exit_code::SATISFIABLE | exit_code::EXHAUSTED, 30);
}

#[test]
fn clingos_own_command_line_errors_are_ok_values() {
    let _lock = serial();
    // Measured with the default main: an unknown option and an ambiguous
    // prefix give 1, a missing input file gives 65 (not 128).
    assert_eq!(code(Application::new().run(["--nosuch"])), 1);
    assert_eq!(code(Application::new().run(["--f"])), 1);
    assert_eq!(code(Application::new().run(["-f"])), 1);
    assert_eq!(
        code(Application::new().run(["/nonexistent/clingox_m5a.lp", "--outf=3"])),
        exit_code::ERROR
    );
}

#[test]
fn arguments_may_be_any_os_string_like_type() {
    let _lock = serial();
    let file = child::fixture("types.lp", SAT);
    let as_paths = vec![file.clone(), std::path::PathBuf::from("--outf=3")];
    assert_eq!(code(Application::new().run(as_paths)), 10);
    let as_os = vec![file.into_os_string(), "--outf=3".into()];
    assert_eq!(code(Application::new().run(as_os)), 10);
    let mut owned = vec![child::arg(&child::fixture("types2.lp", SAT))];
    owned.push(String::from("--outf=3"));
    assert_eq!(code(Application::new().run(owned.iter())), 10);
}

#[test]
fn a_time_limit_that_is_not_reached_is_allowed() {
    let _lock = serial();
    let file = child::fixture("time_ok.lp", SAT);
    let mut args = quiet(&file);
    args.push("--time-limit=100".into());
    assert_eq!(code(Application::new().run(args)), 10);
}

// ---- name, version, setters ----

#[test]
fn name_version_and_limit_setters_accept_edge_values() {
    let _lock = serial();
    let file = child::fixture("setters.lp", SAT);
    for app in [
        Application::new().program_name("").version(""),
        Application::new()
            .program_name("first")
            .program_name("last"),
        Application::new().message_limit(20),
        Application::new().message_limit(0).message_limit(5),
    ] {
        assert_eq!(code(app.run(quiet(&file))), 10);
    }
}

#[test]
fn a_nul_in_the_name_or_version_is_reported_by_run() {
    let _lock = serial();
    let file = child::fixture("nul_names.lp", SAT);
    // The setters do not fail; `run` does, before anything runs.
    let named = Application::new().program_name("a\0b");
    assert_eq!(named.run(quiet(&file)).unwrap_err().kind(), ErrorKind::Nul);
    let versioned = Application::new().version("1\0");
    assert_eq!(
        versioned.run(quiet(&file)).unwrap_err().kind(),
        ErrorKind::Nul
    );
    // Nothing was left locked.
    assert_eq!(code(Application::new().run(quiet(&file))), 10);
}

#[test]
fn debug_names_what_is_set() {
    let plain = format!("{:?}", Application::new());
    assert!(plain.contains("logger: false"), "{plain}");
    assert!(plain.contains("message_limit: None"), "{plain}");
    let set = format!(
        "{:?}",
        Application::new()
            .program_name("prog")
            .version("9.9-test")
            .message_limit(2)
            .logger(|_, _| {})
    );
    assert!(set.contains("logger: true"), "{set}");
    assert!(set.contains("message_limit: Some(2)"), "{set}");
    assert!(set.contains("\"prog\""), "{set}");
    assert!(set.contains("\"9.9-test\""), "{set}");
}

// ---- the logger and the message limit ----

fn messages(limit: Option<u32>, program: &str) -> (i32, Vec<(MessageCode, String)>) {
    let file = child::fixture(&format!("log_{}.lp", program.len()), program);
    let mut seen = Vec::new();
    let mut app = Application::new().logger(|code, text| seen.push((code, text.to_owned())));
    if let Some(limit) = limit {
        app = app.message_limit(limit);
    }
    let result = code(app.run(quiet(&file)));
    (result, seen)
}

#[test]
fn message_limit_two_delivers_exactly_two() {
    let _lock = serial();
    let file = child::fixture("limit2.lp", WARN);
    let path = child::arg(&file);
    let mut seen = Vec::new();
    let app = Application::new()
        .message_limit(2)
        .logger(|code, text| seen.push((code, text.to_owned())));
    assert_eq!(code(app.run(quiet(&file))), 30);
    let head = "info: atom does not occur in any rule head:";
    assert_eq!(
        seen,
        vec![
            (
                MessageCode::AtomUndefined,
                format!("{path}:1:6-7: {head}\n  b")
            ),
            (
                MessageCode::AtomUndefined,
                format!("{path}:1:14-15: {head}\n  d")
            ),
        ]
    );
}

#[test]
fn without_a_limit_up_to_twenty_messages_arrive() {
    let _lock = serial();
    let (result, four) = messages(None, WARN);
    assert_eq!(result, 30);
    assert_eq!(four.len(), 4);
    assert!(four.iter().all(|(c, _)| *c == MessageCode::AtomUndefined));
    let many = many_warnings();
    let (_, twenty) = messages(None, &many);
    assert_eq!(twenty.len(), 20);
    let (_, still_twenty) = messages(Some(20), &many);
    assert_eq!(still_twenty.len(), 20);
}

#[test]
fn a_limit_of_zero_delivers_nothing_and_a_large_one_everything() {
    let _lock = serial();
    let (result, none) = messages(Some(0), WARN);
    assert_eq!(result, 30);
    assert!(none.is_empty(), "{none:?}");
    let (_, all) = messages(Some(1000), &many_warnings());
    assert_eq!(all.len(), 25);
}

#[test]
fn the_logger_may_borrow_from_the_caller() {
    let _lock = serial();
    let file = child::fixture("borrow.lp", WARN);
    let mut lines: Vec<String> = Vec::new();
    let count = AtomicUsize::new(0);
    let app = Application::new().logger(|_, text| {
        lines.push(text.to_owned());
        count.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(code(app.run(quiet(&file))), 30);
    assert_eq!(lines.len(), 4);
    assert_eq!(count.load(Ordering::Relaxed), 4);
}

#[test]
fn a_panicking_logger_is_resumed_after_the_run_and_the_next_run_works() {
    let _lock = serial();
    let file = child::fixture("panic_logger.lp", WARN);
    let calls = AtomicUsize::new(0);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        Application::new()
            .logger(|_, _| {
                calls.fetch_add(1, Ordering::Relaxed);
                panic!("logger boom");
            })
            .run(quiet(&file))
    }));
    let payload = outcome.expect_err("the panic resumes in run");
    assert_eq!(payload.downcast_ref::<&str>().copied(), Some("logger boom"));
    // The panic stopped the logger being called again; clingo still finished.
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    // Fresh slots, the run flag is free, the dispositions were restored.
    assert_eq!(code(Application::new().run(quiet(&file))), 30);
    let (_, again) = messages(None, WARN);
    assert_eq!(again.len(), 4);
}

// ---- sequence, threads, the run flag ----

#[test]
fn runs_in_sequence_and_on_another_thread() {
    let _lock = serial();
    let file = child::fixture("sequence.lp", SAT);
    for _ in 0..3 {
        assert_eq!(code(Application::new().run(quiet(&file))), 10);
    }
    // Without threads (WebAssembly without atomics) there is no other thread.
    if clingox_sys::HAS_THREADS {
        let args = quiet(&file);
        let on_thread = std::thread::spawn(move || Application::new().run(args))
            .join()
            .unwrap();
        assert_eq!(code(on_thread), 10);
    }
}

#[test]
fn two_hundred_runs_in_a_row() {
    let _lock = serial();
    let file = child::fixture("many_runs.lp", "a.");
    for _ in 0..200 {
        assert_eq!(code(Application::new().run(quiet(&file))), 30);
    }
}

/// Runs `body` on its own thread and ends the whole test process if it takes
/// longer than 30 s: a deadlock in `run` must fail in bounded time, and a
/// panic would leave the stuck thread (and the run flag) behind for the rest
/// of the binary. Without threads (WebAssembly without atomics) `body` runs
/// inline.
fn within_limit<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
    if !clingox_sys::HAS_THREADS {
        return body();
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(body());
    });
    if let Ok(value) = rx.recv_timeout(LIMIT) {
        value
    } else {
        eprintln!("the test hung for {LIMIT:?}: run deadlocked; aborting the test process");
        std::process::abort();
    }
}

#[test]
fn a_nested_run_is_refused_and_the_outer_run_finishes() {
    let _lock = serial();
    let outer = child::fixture("nested_outer.lp", WARN);
    let inner = child::fixture("nested_inner.lp", SAT);
    let refusals = within_limit(move || {
        let mut kinds = Vec::new();
        let mut messages = Vec::new();
        let app = Application::new().logger(|_, _| match Application::new().run(quiet(&inner)) {
            Ok(code) => kinds.push(format!("ok {code}")),
            Err(error) => {
                kinds.push(format!("{:?}", error.kind()));
                messages.push(error.to_string());
            }
        });
        let result = app.run(quiet(&outer));
        (result.unwrap(), kinds, messages)
    });
    let (result, kinds, messages) = refusals;
    assert_eq!(result, 30, "the outer run is not disturbed");
    assert_eq!(kinds, vec!["InvalidInput"; 4]);
    assert!(
        messages.iter().all(|m| m.contains("nested")),
        "{messages:?}"
    );
}

#[test]
fn a_run_on_a_second_thread_while_one_is_active_is_refused_without_blocking() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let _lock = serial();
    let file = child::fixture("concurrent.lp", WARN);
    let other = child::fixture("concurrent_other.lp", SAT);
    let (inside_tx, inside_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let first = std::thread::spawn(move || {
        let mut once = true;
        let app = Application::new().logger(move |_, _| {
            if std::mem::take(&mut once) {
                inside_tx.send(()).unwrap();
                // Hold the run open until the test has tried its own.
                let _ = release_rx.recv_timeout(LIMIT);
            }
        });
        app.run(quiet(&file))
    });
    inside_rx
        .recv_timeout(LIMIT)
        .expect("the first run reached its logger");
    let second = within_limit(move || Application::new().run(quiet(&other)));
    assert_eq!(second.unwrap_err().kind(), ErrorKind::InvalidInput);
    // An independent control is not affected by a running application.
    let mut ctl = Control::new().unwrap();
    ctl.add_base("x. y :- x.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
    release_tx.send(()).unwrap();
    assert_eq!(code(first.join().unwrap()), 30);
    // The refusal left nothing behind.
    let again = child::fixture("concurrent_again.lp", SAT);
    assert_eq!(code(Application::new().run(quiet(&again))), 10);
}

#[test]
fn racing_runs_each_get_a_result_and_at_least_one_proceeds() {
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let _lock = serial();
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let file = child::fixture(&format!("race{i}.lp"), SAT);
            std::thread::spawn(move || Application::new().run(quiet(&file)))
        })
        .collect();
    let mut proceeded = 0;
    for handle in handles {
        match handle.join().unwrap() {
            Ok(code) => {
                assert_eq!(code, 10);
                proceeded += 1;
            }
            Err(error) => assert_eq!(error.kind(), ErrorKind::InvalidInput),
        }
    }
    assert!(proceeded >= 1);
}

// ---- the argument checks ----

#[test]
fn fast_exit_and_its_prefixes_are_refused_before_anything_runs() {
    let _lock = serial();
    let file = child::fixture("fast_exit.lp", WARN);
    for bad in [
        "--fast-exit",
        "--fast",
        "--fast-e",
        "--fa",
        "--fast-exit=no",
        "--fa=1",
    ] {
        let calls = AtomicUsize::new(0);
        let mut args = quiet(&file);
        args.push(bad.to_owned());
        let result = Application::new()
            .logger(|_, _| {
                calls.fetch_add(1, Ordering::Relaxed);
            })
            .run(args);
        let error = result.expect_err(bad);
        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{bad}");
        assert!(error.to_string().contains(bad), "{bad}: {error}");
        assert_eq!(calls.load(Ordering::Relaxed), 0, "{bad}: nothing may run");
    }
    // After a separator too, and in first position.
    let mut args = quiet(&file);
    args.extend(["--".to_owned(), "--fast-exit".to_owned()]);
    assert_eq!(
        Application::new().run(args).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        Application::new().run(["--fast-exit"]).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn near_misses_of_the_filter_reach_clingo() {
    let _lock = serial();
    // `--f` is one letter: not filtered, clingo calls it ambiguous (1).
    // `--no-fast-exit` is a different name, refused by clingo itself (1).
    for near in ["--f", "--no-fast-exit", "--no-fa", "--time-limit=100", "-f"] {
        let file = child::fixture("near.lp", SAT);
        let mut args = quiet(&file);
        args.push(near.to_owned());
        let result = Application::new().run(args);
        let expected = if near.starts_with("--time") { 10 } else { 1 };
        assert_eq!(result.unwrap(), expected, "{near}");
    }
}

#[test]
fn a_nul_in_an_argument_is_a_nul_error() {
    let _lock = serial();
    let error = Application::new().run(["a\0b"]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Nul);
    // The check is on the arguments, left to right, before the run flag.
    let file = child::fixture("nul_arg.lp", SAT);
    assert_eq!(code(Application::new().run(quiet(&file))), 10);
}

#[cfg(unix)]
#[test]
fn an_argument_that_is_not_utf8_is_invalid_input() {
    use std::os::unix::ffi::OsStringExt;
    let _lock = serial();
    let bad = std::ffi::OsString::from_vec(vec![b'x', 0xff]);
    let error = Application::new().run([bad]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(error.to_string().contains("UTF-8"), "{error}");
}

#[test]
fn a_refused_run_leaves_the_next_one_working() {
    let _lock = serial();
    let file = child::fixture("refused_then_ok.lp", SAT);
    assert!(Application::new().run(["--fa"]).is_err());
    assert!(Application::new().run(["a\0"]).is_err());
    assert_eq!(code(Application::new().run(quiet(&file))), 10);
}

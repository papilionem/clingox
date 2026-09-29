//! What a failing or absent `main` callback prints, checked in child
//! processes.
//!
//! When `main` returns false clingo prints `*** ERROR: (clingo): <text>` on
//! standard error and exits with 65, whatever the error's kind (pyclingo
//! 5.8.2). `run` returns
//! `Err` for it, so the exit code itself is not visible to the caller: the
//! line on standard error is the observable, and it is what tells "clingo was
//! told the callback failed" from "`run` returned `Err` on its own". The
//! application's logger does not receive that line (measured). `--help`,
//! `--version` and unknown options never reach `main`.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use std::panic::{AssertUnwindSafe, catch_unwind};

use clingox::application::Application;
use clingox::{Error, ErrorKind};

const ENTRY: &str = "child_entry";
const SAT: &str = "a. {b}.";
const ERROR_LINE: &str = "*** ERROR: (clingo): ";

fn failing(kind: ErrorKind) -> Application<'static> {
    Application::new().main(move |_ctl, _files| Err(Error::new(kind, "my main failed")))
}

/// The child side: runs the case named by the environment, or does nothing.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    let file = child::fixture("main_child.lp", SAT);
    let args = [child::arg(&file), "--outf=3".to_owned()];
    match case.as_str() {
        "error_runtime" | "error_logic" => {
            let kind = if case == "error_runtime" {
                ErrorKind::Runtime
            } else {
                ErrorKind::Logic
            };
            match failing(kind).run(args) {
                Ok(code) => println!("CHILD-RC {code}"),
                Err(error) => println!("CHILD-ERR {:?} {error}", error.kind()),
            }
        }
        "panic" => {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                Application::new()
                    .main(|_ctl, _files| panic!("boom in main"))
                    .run(args)
            }));
            match outcome {
                Ok(result) => println!("CHILD-NO-PANIC {result:?}"),
                Err(payload) => {
                    let text = payload
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    println!("CHILD-PANIC {text}");
                }
            }
        }
        "logger_and_error" => {
            let mut seen = 0_usize;
            let result = Application::new()
                .logger(|_, _| seen += 1)
                .main(|_ctl, _files| Err(Error::new(ErrorKind::Runtime, "my main failed")))
                .run(args);
            println!("CHILD-LOGGED {seen}");
            println!("CHILD-IS-ERR {}", result.is_err());
        }
        "help" | "version" => {
            let mut calls = 0_u32;
            let flag = if case == "help" {
                "--help"
            } else {
                "--version"
            };
            let result = Application::new()
                .program_name("prog")
                .version("9.9-test")
                .main(|_ctl, _files| {
                    calls += 1;
                    Ok(())
                })
                .run([flag]);
            println!("CHILD-MAIN-CALLS {calls}");
            match result {
                Ok(code) => println!("CHILD-RC {code}"),
                Err(error) => println!("CHILD-ERR {:?}", error.kind()),
            }
        }
        other => panic!("unknown child case {other}"),
    }
}

fn run(case: &str) -> Option<child::Outcome> {
    child::run_child(ENTRY, case)
}

fn error_lines(stderr: &str) -> Vec<&str> {
    stderr.lines().filter(|l| l.contains(ERROR_LINE)).collect()
}

#[test]
fn a_failing_main_makes_clingo_print_one_error_line() {
    for case in ["error_runtime", "error_logic"] {
        let Some(out) = run(case) else {
            return;
        };
        let lines = error_lines(&out.stderr);
        assert_eq!(lines.len(), 1, "{case}: {}", out.stderr);
        assert!(lines[0].contains("my main failed"), "{case}: {}", lines[0]);
        // `run` returns the callback's own error, kind and text.
        let expected_kind = if case == "error_runtime" {
            "Runtime"
        } else {
            "Logic"
        };
        assert!(
            out.stdout.contains(&format!("CHILD-ERR {expected_kind} ")),
            "{case}: {}",
            out.stdout
        );
        assert!(out.stdout.contains("my main failed"), "{}", out.stdout);
        // Nothing was solved or summarised after the failure.
        assert!(!out.stdout.contains("SATISFIABLE"), "{}", out.stdout);
        assert!(!out.stdout.contains("CHILD-RC"), "{}", out.stdout);
    }
}

#[test]
fn a_panicking_main_makes_clingo_print_an_error_line_and_run_resumes_the_panic() {
    let Some(out) = run("panic") else {
        return;
    };
    let lines = error_lines(&out.stderr);
    assert_eq!(lines.len(), 1, "{}", out.stderr);
    assert!(
        lines[0].contains("main callback panicked"),
        "the line names the panic: {}",
        lines[0]
    );
    assert!(
        out.stdout.contains("CHILD-PANIC boom in main"),
        "{}",
        out.stdout
    );
}

#[test]
fn the_logger_does_not_receive_the_error_line() {
    let Some(out) = run("logger_and_error") else {
        return;
    };
    assert_eq!(error_lines(&out.stderr).len(), 1, "{}", out.stderr);
    assert!(out.stdout.contains("CHILD-LOGGED 0"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-IS-ERR true"), "{}", out.stdout);
}

#[test]
fn help_and_version_do_not_call_main() {
    let Some(help) = run("help") else {
        return;
    };
    assert!(
        help.stdout.contains("prog version 9.9-test\nusage: prog "),
        "{}",
        help.stdout
    );
    assert!(
        help.stdout.contains("CHILD-MAIN-CALLS 0"),
        "{}",
        help.stdout
    );
    assert!(help.stdout.contains("CHILD-RC 0"), "{}", help.stdout);
    let Some(version) = run("version") else {
        return;
    };
    assert!(
        version.stdout.contains("prog version 9.9-test\n"),
        "{}",
        version.stdout
    );
    assert!(
        version.stdout.contains("CHILD-MAIN-CALLS 0"),
        "{}",
        version.stdout
    );
    assert!(version.stdout.contains("CHILD-RC 0"), "{}", version.stdout);
}

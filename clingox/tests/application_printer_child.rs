//! What the model printer writes and in which order, checked in child processes
//! with standard output a pipe.
//!
//! `clingo_main` writes with C stdio, which libtest cannot capture, and Rust's
//! `println!` writes straight to file descriptor 1, so the relative order of
//! the two is only observable from outside. Expected values come from pyclingo
//! 5.8.2 (clingo 5.8.2, clasp 3.4.1) calling `clingo_main` with a `printer`
//! callback. Program `{a;b}.` with `--verbose=0 0` delivers four models whose
//! text lines are an empty line (the empty model), `b`, `a`, `a b`, then clingo
//! prints `SATISFIABLE`.
//!
//! Without the flushes around the default printer, the closure's lines all come
//! before the model text on a pipe (C stdio is fully buffered there).

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the child reports through standard output, and the closure under test prints"
)]

#[path = "common/child.rs"]
mod child;

use std::sync::atomic::{AtomicUsize, Ordering};

use clingox::application::Application;
use clingox::{Error, ErrorKind, Part, Result, ScopedControl};

const ENTRY: &str = "child_entry";
const AB: &str = "{a;b}.";
const MODELS: [&str; 4] = ["", "b", "a", "a b"];

fn blocking(ctl: &mut ScopedControl<'_>) -> Result<()> {
    ctl.add_base(AB)?;
    ctl.ground(&[Part::base()])?;
    let _ = ctl.solve(&[])?;
    Ok(())
}

fn asynchronous(ctl: &mut ScopedControl<'_>) -> Result<()> {
    ctl.add_base(AB)?;
    ctl.ground(&[Part::base()])?;
    let mut handle = ctl.solve_async(&[])?;
    let _ = handle.get()?;
    let _ = handle.close()?;
    Ok(())
}

fn report(result: Result<i32>) {
    println!("CHILD-END");
    match result {
        Ok(code) => println!("CHILD-RC {code}"),
        Err(error) => println!("CHILD-ERR {:?} {error}", error.kind()),
    }
}

/// The child side: runs the case named by the environment, or does nothing.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    let args: Vec<&str> = match case.as_str() {
        "outf1" => vec!["--verbose=0", "0", "--outf=1"],
        "quiet1" => vec!["--verbose=0", "0", "--quiet=1"],
        _ => vec!["--verbose=0", "0"],
    };
    // Ends libtest's unfinished "test child_entry ... " line.
    println!();
    println!("CHILD-BEGIN");
    let calls = AtomicUsize::new(0);
    let application = Application::new().main(|ctl, _files| {
        if case == "async_order" {
            asynchronous(ctl)
        } else {
            blocking(ctl)
        }
    });
    let result = match case.as_str() {
        "order" | "async_order" => application
            .print_model(|_model, printer| {
                println!("before");
                printer.print()?;
                println!("after");
                Ok(())
            })
            .run(args),
        "zero" => application.print_model(|_model, _printer| Ok(())).run(args),
        "twice" => application
            .print_model(|_model, printer| {
                printer.print()?;
                printer.print()
            })
            .run(args),
        "partial" => application
            .print_model(|_model, printer| {
                printer.print()?;
                print!("tail ");
                Ok(())
            })
            .run(args),
        "outf1" | "quiet1" => application
            .print_model(|_model, printer| {
                println!("called");
                printer.print()
            })
            .run(args),
        "failing" => application
            .print_model(|_model, printer| {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                println!("call {n}");
                printer.print()?;
                if n == 2 {
                    return Err(Error::new(ErrorKind::Runtime, "child printer failed"));
                }
                Ok(())
            })
            .run(args),
        "stderr_between" => application
            .print_model(|_model, printer| {
                printer.print()?;
                eprintln!("between");
                printer.print()
            })
            .run(args),
        other => panic!("unknown child case {other}"),
    };
    report(result);
}

fn run(case: &str) -> Option<child::Outcome> {
    child::run_child(ENTRY, case)
}

/// The lines the child wrote between its markers.
fn body(outcome: &child::Outcome) -> Vec<String> {
    let out = &outcome.stdout;
    let start = out.find("CHILD-BEGIN\n").expect("begin marker") + "CHILD-BEGIN\n".len();
    let end = out.find("CHILD-END\n").expect("end marker");
    out[start..end]
        .strip_suffix('\n')
        .unwrap_or(&out[start..end])
        .split('\n')
        .map(str::to_owned)
        .collect()
}

fn rc(outcome: &child::Outcome) -> &str {
    outcome
        .stdout
        .lines()
        .find(|l| l.starts_with("CHILD-RC") || l.starts_with("CHILD-ERR"))
        .expect("result marker")
}

fn around(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for model in lines {
        out.push("before".to_owned());
        out.push((*model).to_owned());
        out.push("after".to_owned());
    }
    out.push("SATISFIABLE".to_owned());
    out
}

#[test]
fn c1_println_around_print_is_in_program_order() {
    let Some(outcome) = run("order") else { return };
    assert_eq!(body(&outcome), around(&MODELS));
    assert_eq!(rc(&outcome), "CHILD-RC 30");
}

#[test]
fn c2_zero_calls_print_nothing_and_two_calls_print_twice() {
    if let Some(outcome) = run("zero") {
        assert_eq!(body(&outcome), ["SATISFIABLE"]);
    }
    if let Some(outcome) = run("twice") {
        let mut want: Vec<String> = MODELS
            .iter()
            .flat_map(|m| [(*m).to_owned(), (*m).to_owned()])
            .collect();
        want.push("SATISFIABLE".to_owned());
        assert_eq!(body(&outcome), want);
    }
}

#[test]
fn c3_a_partial_line_is_flushed_before_clingo_writes_again() {
    let Some(outcome) = run("partial") else {
        return;
    };
    // Each `tail ` ends up in front of clingo's next output.
    assert_eq!(
        body(&outcome),
        ["", "tail b", "tail a", "tail a b", "tail SATISFIABLE"]
    );
}

#[test]
fn c4_the_printer_runs_once_for_outf1_and_quiet1() {
    for case in ["outf1", "quiet1"] {
        let Some(outcome) = run(case) else { continue };
        let lines = body(&outcome);
        assert_eq!(
            lines.iter().filter(|l| *l == "called").count(),
            1,
            "{case}: {lines:?}"
        );
        let at = lines.iter().position(|l| l == "called").unwrap();
        // `--outf=1` writes an `ANSWER` line and the model as facts (`a. b.`).
        assert!(
            lines[at + 1..].iter().any(|l| l == "a b" || l == "a. b."),
            "{case}: the last model: {lines:?}"
        );
        assert_eq!(rc(&outcome), "CHILD-RC 30", "{case}");
    }
}

#[test]
fn c5_a_failing_printer_is_not_reported_to_clingo_and_stops_the_output() {
    let Some(outcome) = run("failing") else {
        return;
    };
    let lines = body(&outcome);
    assert!(lines.contains(&"call 1".to_owned()), "{lines:?}");
    assert!(lines.contains(&"call 2".to_owned()), "{lines:?}");
    assert!(
        !lines.iter().any(|l| l.starts_with("call 3")),
        "no closure call after the failure: {lines:?}"
    );
    // The error is stored and the search interrupted; clingo is not told the
    // callback failed (U25/U26), so it prints no error line of its own.
    assert!(
        !outcome.stderr.contains("*** ERROR"),
        "stderr: {}",
        outcome.stderr
    );
    let result = rc(&outcome);
    assert!(
        result.starts_with("CHILD-ERR Runtime") && result.contains("child printer failed"),
        "{result}"
    );
}

#[test]
fn c6_the_order_holds_on_the_solver_thread_of_an_async_solve() {
    if !clingox_sys::HAS_THREADS {
        eprintln!("SKIPPED: c6: this build has no threads");
        return;
    }
    let Some(outcome) = run("async_order") else {
        return;
    };
    assert_eq!(body(&outcome), around(&MODELS));
    assert_eq!(rc(&outcome), "CHILD-RC 30");
}

#[test]
fn c7_standard_error_between_print_calls_neither_hangs_nor_garbles() {
    let Some(outcome) = run("stderr_between") else {
        return;
    };
    let mut want: Vec<String> = MODELS
        .iter()
        .flat_map(|m| [(*m).to_owned(), (*m).to_owned()])
        .collect();
    want.push("SATISFIABLE".to_owned());
    assert_eq!(body(&outcome), want);
    assert_eq!(outcome.stderr.matches("between\n").count(), 4);
}

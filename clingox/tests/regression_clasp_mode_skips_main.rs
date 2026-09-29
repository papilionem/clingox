//! With `--mode=clasp` clingo
//! runs clasp's own run loop (`ClingoApp::run`) and never calls the
//! application's `main`, `print_model` or `validate_options` hooks. The run
//! still returns clasp's exit code. This pins the behaviour the documentation
//! of `Application::run`, `main` and `print_model` now states. The one
//! clasp-mode run of the process is allowed.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::print_stdout, reason = "test")]

#[path = "common/child.rs"]
mod child;

use std::sync::atomic::{AtomicBool, Ordering};

use clingox::application::{Application, exit_code};

const ENTRY: &str = "child_entry";

#[test]
fn child_entry() {
    if child::child_case().is_none() {
        return;
    }
    let cnf = child::fixture("skip.cnf", "p cnf 1 1\n1 0\n");
    let (main_called, printed) = (AtomicBool::new(false), AtomicBool::new(false));
    let code = Application::new()
        .main(|_ctl, _files| {
            main_called.store(true, Ordering::SeqCst);
            Ok(())
        })
        .print_model(|_model, _printer| {
            printed.store(true, Ordering::SeqCst);
            Ok(())
        })
        .run(["--mode=clasp", &child::arg(&cnf), "--outf=3"]);
    println!(
        "\nCODE {code:?} MAIN-CALLED {} PRINT-CALLED {}",
        main_called.load(Ordering::SeqCst),
        printed.load(Ordering::SeqCst)
    );
}

#[test]
fn main_and_print_model_are_not_called_in_clasp_mode() {
    let Some(outcome) = child::run_child(ENTRY, "skip") else {
        return;
    };
    let expected = format!(
        "CODE Ok({}) MAIN-CALLED false PRINT-CALLED false",
        exit_code::SATISFIABLE | exit_code::EXHAUSTED
    );
    assert!(outcome.stdout.contains(&expected), "{outcome:?}");
}

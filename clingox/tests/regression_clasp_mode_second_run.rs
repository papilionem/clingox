//! Clasp opens its input stream once
//! per process (`ClaspAppBase::getStream`, a function-local static), so a
//! second `--mode=clasp` run reads the first run's stream. After a first run
//! that failed (128), a second one on an unsatisfiable CNF returned
//! satisfiable, and pyclingo behaves the same. `run` therefore refuses a
//! second clasp-mode run in the process with `ErrorKind::InvalidInput`, before
//! clingo starts; the first one is allowed, whatever its outcome, and runs in
//! the other modes are not affected. Every case runs in a child, one process
//! per sequence of runs.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::print_stdout, reason = "test")]

#[path = "common/child.rs"]
mod child;

use clingox::ErrorKind;
use clingox::application::{Application, Flag, exit_code};

const ENTRY: &str = "child_entry";

fn show(label: &str, result: &clingox::Result<i32>) {
    match result {
        Ok(code) => println!("\n{label} OK {code}"),
        Err(error) => println!(
            "\n{label} ERR {:?} {}",
            error.kind(),
            error.to_string().replace('\n', " ")
        ),
    }
}

#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    let sat = child::arg(&child::fixture("sat.cnf", "p cnf 1 1\n1 0\n"));
    let unsat = child::arg(&child::fixture("unsat.cnf", "p cnf 1 2\n1 0\n-1 0\n"));
    let lp = child::arg(&child::fixture("mode.lp", "a."));
    let clasp = |file: &str| Application::new().run(["--mode=clasp", file, "0", "--outf=3"]);
    match case.as_str() {
        "alone" => show("FIRST", &clasp(&unsat)),
        "after_success" => {
            show("FIRST", &clasp(&sat));
            show("SECOND", &clasp(&unsat));
        }
        "after_failure" => {
            // A command-line error after clasp has opened the first file.
            let first = Application::new().run([
                "--mode=clasp",
                &sat,
                "--lemma-in=/nonexistent/lemmas",
                "--outf=3",
            ]);
            show("FIRST", &first);
            show("SECOND", &clasp(&unsat));
        }
        "spelled_apart" => {
            show("FIRST", &clasp(&sat));
            let second = Application::new().run(["--mode", "CLASP", &unsat, "--outf=3"]);
            show("SECOND", &second);
        }
        "after_refusal" => {
            // Refused inside the register callback, before clasp read anything.
            let flag = Flag::new(false);
            let first = Application::new()
                .register_options(|options| options.add_flag("App", "mine,m", "A flag", &flag))
                .run(["--mode=clasp", &sat, "-mo", "text", "--outf=3"]);
            show("REFUSED", &first);
            show("CLEAN", &clasp(&unsat));
        }
        "other_modes_after" => {
            show("FIRST", &clasp(&sat));
            show("CLINGO", &Application::new().run([&lp, "--outf=3"]));
            show(
                "GRINGO",
                &Application::new().run([&lp, "--mode=gringo", "--outf=3"]),
            );
        }
        "other_modes_before" => {
            show("CLINGO", &Application::new().run([&lp, "--outf=3"]));
            show("FIRST", &clasp(&unsat));
        }
        _ => unreachable!(),
    }
}

fn run(case: &str) -> Option<String> {
    let out = child::run_child(ENTRY, case)?;
    assert_eq!(out.code, Some(0), "{case}: {out:?}");
    Some(out.stdout)
}

fn refused(stdout: &str, label: &str) {
    let line = stdout
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{label} ")))
        .unwrap_or_else(|| panic!("no {label} line: {stdout}"));
    assert!(
        line.starts_with(&format!("ERR {:?}", ErrorKind::InvalidInput)),
        "{label}: {line}"
    );
    assert!(
        line.to_lowercase().contains("clasp"),
        "the message names the reason: {line}"
    );
}

#[test]
fn a_clasp_mode_run_alone_answers_for_its_file() {
    let Some(out) = run("alone") else {
        return;
    };
    assert!(
        out.contains(&format!("FIRST OK {}", exit_code::EXHAUSTED)),
        "{out}"
    );
}

#[test]
fn a_second_clasp_mode_run_is_refused_after_a_success() {
    let Some(out) = run("after_success") else {
        return;
    };
    assert!(
        out.contains(&format!(
            "FIRST OK {}",
            exit_code::SATISFIABLE | exit_code::EXHAUSTED
        )),
        "{out}"
    );
    refused(&out, "SECOND");
}

#[test]
fn a_second_clasp_mode_run_is_refused_after_a_failed_first() {
    let Some(out) = run("after_failure") else {
        return;
    };
    assert!(
        out.contains(&format!("FIRST OK {}", exit_code::NO_RUN)),
        "{out}"
    );
    refused(&out, "SECOND");
}

#[test]
fn the_mode_is_read_from_a_separate_argument_too() {
    let Some(out) = run("spelled_apart") else {
        return;
    };
    refused(&out, "SECOND");
}

#[test]
fn other_modes_are_not_affected_by_a_clasp_mode_run() {
    let Some(after) = run("other_modes_after") else {
        return;
    };
    assert!(after.contains("CLINGO OK "), "{after}");
    assert!(after.contains("GRINGO OK "), "{after}");
    let Some(before) = run("other_modes_before") else {
        return;
    };
    assert!(before.contains("CLINGO OK "), "{before}");
    assert!(
        before.contains(&format!("FIRST OK {}", exit_code::EXHAUSTED)),
        "{before}"
    );
}

#[test]
fn a_clasp_mode_run_refused_before_it_started_does_not_block_a_later_one() {
    let Some(out) = run("after_refusal") else {
        return;
    };
    refused_as_input(&out, "REFUSED");
    assert!(
        out.contains(&format!("CLEAN OK {}", exit_code::EXHAUSTED)),
        "{out}"
    );
}

fn refused_as_input(stdout: &str, label: &str) {
    let line = stdout
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{label} ")))
        .unwrap_or_else(|| panic!("no {label} line: {stdout}"));
    assert!(
        line.starts_with(&format!("ERR {:?}", ErrorKind::InvalidInput)),
        "{label}: {line}"
    );
}

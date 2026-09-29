//! What `Application::run` prints, checked in child processes.
//!
//! `clingo_main` writes to C stdio, which libtest cannot capture, so each case
//! runs the test binary again (`common/child.rs`) and the parent reads the
//! child's standard output and error. Expected texts are from pyclingo 5.8.2
//! calling `clingo_main` without a `main` callback. A child prints `CHILD-RC
//! <n>` with the value `run` returned.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use clingox::ErrorKind;
use clingox::application::Application;

const ENTRY: &str = "child_entry";
const SAT: &str = "a. {b}.";
const WARN: &str = "a :- b. c :- d. e :- f. g :- h.";

fn report(result: clingox::Result<i32>) {
    match result {
        Ok(code) => println!("CHILD-RC {code}"),
        Err(error) => println!("CHILD-ERR {:?}", error.kind()),
    }
}

/// The child side: runs the case named by the environment, or does nothing.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    match case.as_str() {
        "version_custom" => report(
            Application::new()
                .program_name("prog")
                .version("9.9-test")
                .run(["--version"]),
        ),
        "help_custom" => report(
            Application::new()
                .program_name("prog")
                .version("9.9-test")
                .run(["--help"]),
        ),
        "version_default" => report(Application::new().run(["--version"])),
        "logger_replaces_stderr" | "no_logger_uses_stderr" => {
            let file = child::fixture("child_warn.lp", WARN);
            let mut seen = 0;
            let mut app = Application::new();
            if case == "logger_replaces_stderr" {
                app = app.logger(|_, _| seen += 1);
            }
            let result = app.run([child::arg(&file)]);
            println!("CHILD-LOGGED {seen}");
            report(result);
        }
        "fast_exit_refused" => {
            let file = child::fixture("child_fast.lp", SAT);
            let refused = Application::new().run([child::arg(&file), "--fast-exit".to_owned()]);
            println!(
                "CHILD-REFUSED {}",
                refused.unwrap_err().kind() == ErrorKind::InvalidInput
            );
            report(Application::new().run([child::arg(&file)]));
            println!("CHILD-ALIVE");
        }
        "partial_line_first" => {
            let file = child::fixture("child_partial.lp", SAT);
            print!("partial ");
            report(Application::new().run([child::arg(&file)]));
        }
        "empty_arguments_read_stdin" => report(Application::new().run(Vec::<String>::new())),
        other => panic!("unknown child case {other}"),
    }
}

fn run(case: &str) -> Option<child::Outcome> {
    child::run_child(ENTRY, case)
}

#[test]
fn version_shows_the_program_name_and_version() {
    let Some(out) = run("version_custom") else {
        return;
    };
    assert!(
        out.stdout.contains("prog version 9.9-test\n"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("libclingo version 5.8"),
        "{}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("clingo version 5.8.2\nAddress"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("CHILD-RC 0"), "{}", out.stdout);
    assert_eq!(out.code, Some(0));
}

#[test]
fn help_uses_the_program_name() {
    let Some(out) = run("help_custom") else {
        return;
    };
    assert!(
        out.stdout
            .contains("prog version 9.9-test\nusage: prog [number] [options] [files]"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Type 'prog --help=2' for more options"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("CHILD-RC 0"), "{}", out.stdout);
}

#[test]
fn the_default_version_is_clingos() {
    let Some(out) = run("version_default") else {
        return;
    };
    assert!(out.stdout.contains("clingo version 5.8"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-RC 0"), "{}", out.stdout);
}

#[test]
fn a_logger_replaces_the_messages_on_standard_error() {
    let Some(out) = run("logger_replaces_stderr") else {
        return;
    };
    assert!(out.stdout.contains("CHILD-LOGGED 4"), "{}", out.stdout);
    assert!(out.stdout.contains("SATISFIABLE"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-RC 30"), "{}", out.stdout);
    assert!(
        !out.stderr.contains("atom does not occur"),
        "{}",
        out.stderr
    );
}

#[test]
fn without_a_logger_the_messages_go_to_standard_error() {
    let Some(out) = run("no_logger_uses_stderr") else {
        return;
    };
    assert!(out.stdout.contains("CHILD-LOGGED 0"), "{}", out.stdout);
    let messages = out
        .stderr
        .matches("info: atom does not occur in any rule head:")
        .count();
    assert_eq!(messages, 4, "{}", out.stderr);
    assert!(out.stdout.contains("CHILD-RC 30"), "{}", out.stdout);
}

#[test]
fn a_refused_fast_exit_prints_nothing_and_the_process_lives() {
    let Some(out) = run("fast_exit_refused") else {
        return;
    };
    let refused = out.stdout.find("CHILD-REFUSED true").expect(&out.stdout);
    // Nothing of clingo's ran before the refusal was reported.
    let banner = out.stdout.find("clingo version").expect(&out.stdout);
    assert!(refused < banner, "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-RC 10"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-ALIVE"), "{}", out.stdout);
    assert_eq!(out.code, Some(0));
}

#[test]
fn rust_output_written_before_the_run_comes_first() {
    let Some(out) = run("partial_line_first") else {
        return;
    };
    // Rust's stdout holds "partial " until it is flushed; `run` flushes it
    // before clingo prints its first line.
    assert!(
        out.stdout.contains("partial clingo version 5.8"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("CHILD-RC 10"), "{}", out.stdout);
}

#[test]
fn with_no_arguments_clingo_reads_standard_input() {
    let Some(out) = run("empty_arguments_read_stdin") else {
        return;
    };
    // The child's standard input is closed: an empty program, one model.
    assert!(out.stdout.contains("Reading from stdin"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-RC 30"), "{}", out.stdout);
}

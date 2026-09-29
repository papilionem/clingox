//! `Application::run` refuses the options that end the process from inside
//! clingo (smoke finding A1), and lets everything near them through.
//!
//! `--pre` (clingo calls `std::_Exit(0)` once it has printed the program),
//! `--print-portfolio` (clasp calls `exit`) and `--text` combined with
//! `--output` (clingo exits with 128) never return to `run`, so `run` checks
//! the arguments first, as it does for `--fast-exit`, and answers
//! `ErrorKind::InvalidInput` before clingo starts. A call that reaches the
//! option that ends the process kills the test process, so every case runs in
//! a child (`common/child.rs`) and the parent reads what the child printed.
//!
//! Which spellings clasp accepts was measured with pyclingo 5.8.2
//! (`clingo_main`, clingo 5.8.2, clasp 3.4.1) on 2026-09-29, one process per
//! command line:
//!
//! - `--pre`, `--pre=aspif`, `--pre=` and `--pre=ASPIF` print the program and
//!   exit; `--pre=smodels` prints an error and exits. There is no shorter
//!   accepted abbreviation: `--p` and `--pr` are ambiguous (return code 1).
//!   `--pre=text`, `--pre=smod` and `--pre=asp` are invalid values (1).
//! - every prefix of `--print-portfolio` from `--pri` on prints the
//!   portfolio and exits; `--p` and `--pr` are ambiguous (1);
//!   `--print-portfolio=1` "does not take a value" (1).
//! - `--text` (or `--tex`; `--te` is ambiguous) with `--output=text`,
//!   `--output=smodels` or `--output=reify`, in either order, exits with 128
//!   ("mutually exclusive"). `--output` has no shorter abbreviation than
//!   itself (`--outpu` is ambiguous), `--output=aspif` and `--output=none`
//!   are invalid values (1), and either option alone prints the program and
//!   returns 0 (the program is not solved). `-o text`, `-otext`,
//!   `--output text` and `--output=intermediate` count as `--output`, and the
//!   value compares without regard to case.
//! - `--text` or `--output` with `--mode=clasp` or `--mode=clingo` exits with
//!   128 ("can only be used with '--mode=gringo'"); with `--mode=gringo` they
//!   run.
//! - `--lemma-out=<file>` with a file that cannot be opened (`/`, `.`, a missing
//!   directory) exits with 1; `-` and `stdout` run. `--out-atomf=<format>`
//!   exits with 1 unless the format starts with `-` or has exactly one `%s`
//!   or `%0`, also through the abbreviations `--out-a` to `--out-atom` (`x`, `%d`, `%`, `%s%d`, `0`, `no` exit; `%s`, `-%d`, `a%sb`,
//!   `%0`, `-x` run); an empty value is a command-line error (1).

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use clingox::application::Application;
use clingox::{ErrorKind, Part};

const ENTRY: &str = "child_entry";
const SAT: &str = "a. {b}.";

/// Command lines that would end the process, by child case.
fn refused(case: &str) -> Vec<Vec<&'static str>> {
    match case {
        "refused_pre" => vec![
            vec!["--pre"],
            vec!["--pre=aspif"],
            vec!["--pre=smodels"],
            vec!["--pre="],
            vec!["--pre=ASPIF"],
            vec!["--pre", "aspif"],
            vec!["--outf=3", "--pre"],
            vec!["--pre", "--outf=3"],
            vec!["--", "--pre"],
            vec!["--pre=aspif", "--pre=smodels"],
        ],
        "refused_portfolio" => {
            let mut lines = vec![vec!["--print-portfolio"]];
            let full = "--print-portfolio";
            // Every prefix of three letters or more after the dashes.
            for end in "--pri".len()..full.len() {
                lines.push(vec![&full[..end]]);
            }
            lines.push(vec!["--outf=3", "--print-portfolio"]);
            lines.push(vec!["--print-portfolio", "--pre=aspif"]);
            lines.push(vec!["--", "--print-portfolio"]);
            lines
        }
        "refused_text_output" => vec![
            vec!["--text", "--output=text"],
            vec!["--text", "--output=smodels"],
            vec!["--text", "--output=reify"],
            vec!["--tex", "--output=text"],
            vec!["--output=text", "--text"],
            vec!["--output=reify", "--tex"],
            vec!["--outf=3", "--text", "--output=smodels"],
        ],
        "refused_output_forms" => vec![
            vec!["--text", "--output=intermediate"],
            vec!["--text", "--output=TEXT"],
            vec!["-o", "text", "--text"],
            vec!["-otext", "--text"],
            vec!["--output", "text", "--text"],
            vec!["--tex", "-o", "reify"],
        ],
        "refused_mode" => vec![
            vec!["--text", "--mode=clasp"],
            vec!["--text", "--mode=clingo"],
            vec!["--mode=clasp", "--output=text"],
            vec!["--mode=clingo", "-o", "smodels"],
            vec!["--mode=clasp", "--output=intermediate"],
            vec!["--text", "--mode", "CLASP"],
            vec!["--mode", "clasp", "--text"],
        ],
        "refused_setup" => vec![
            vec!["--lemma-out=/nonexistent/dir/x"],
            vec!["--lemma-out=/"],
            vec!["--lemma-out=."],
            vec!["--out-atomf=x"],
            vec!["--out-atomf=%d"],
            vec!["--out-atomf=%"],
            vec!["--out-atomf=%s%d"],
            vec!["--out-atomf=0"],
            vec!["--out-atomf=no"],
            vec!["--out-a=x"],
            vec!["--out-at=x"],
            vec!["--out-ato=x"],
            vec!["--out-atom=x"],
            vec!["--out-a", "x"],
            vec!["--out-atomf=1,2"],
            vec!["--out-atomf=%s%d%%"],
            vec!["--out-atomf=%%-%d"],
            vec!["--out-atomf=/nonexistent/q"],
        ],
        _ => Vec::new(),
    }
}

/// Command lines clingo itself deals with, and the return code it gives
/// (pyclingo 5.8.2): options that only look like the ones above.
const NEAR: &[(&[&str], i32)] = &[
    (&["--p"], 1),
    (&["--pr"], 1),
    (&["--pre=text"], 1),
    (&["--pre=smod"], 1),
    (&["--pre=asp"], 1),
    (&["--no-pre"], 1),
    (&["-pre"], 1),
    (&["--no-print-portfolio"], 1),
    (&["--print-portfolio=1"], 1),
    (&["--te"], 1),
    (&["--text=1"], 1),
    (&["--text", "--output=aspif"], 1),
    (&["--text", "--output=none"], 1),
    (&["--tex", "--outpu=text"], 1),
    (&["--text", "--output"], 1),
    (&["--text"], 0),
    (&["--tex"], 0),
    (&["--output=text"], 0),
    (&["--output=reify"], 0),
    (&["--time-limit=100"], 10),
    // `--text` and `--output` are fine in gringo mode, alone or together with
    // another option; `--lemma-out` and `--out-atomf` with a value clasp takes.
    (&["--text", "--mode=gringo"], 0),
    (&["--mode=gringo", "--output=text"], 0),
    (&["--lemma-out=-"], 10),
    (&["--lemma-out=stdout"], 10),
    (&["--lemma-out"], 1),
    (&["--out-atomf=%s"], 10),
    (&["--out-atomf=-%d"], 10),
    (&["--out-atomf=a%sb"], 10),
    (&["--out-atomf=%0"], 10),
    (&["--out-atomf=-x"], 10),
    (&["--out-atomf=-x%0"], 10),
    // The abbreviations of `--out-atomf` clasp accepts (`--out-a` to
    // `--out-atom`; `--out-` is ambiguous) take a valid format.
    (&["--out-a=%s"], 10),
    (&["--out-at=-%d"], 10),
    (&["--out-ato=%s"], 10),
    (&["--out-atom=-x"], 10),
    (&["--out-at", "%s"], 10),
    (&["--out-at="], 1),
    (&["--out-=x"], 1),
    (&["--out-atomf="], 1),
];

/// A marker on a line of its own, whatever clingo's C stdio left unfinished.
fn mark(text: &str) {
    println!("\nCHILD-{text}");
}

/// The child side: runs the case named by the environment, or does nothing.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    let file = child::fixture("exit_options.lp", SAT);
    let file = child::arg(&file);
    let lines: Vec<Vec<&str>> = if case == "near" {
        NEAR.iter().map(|(args, _)| args.to_vec()).collect()
    } else {
        refused(&case)
    };
    assert!(!lines.is_empty(), "unknown child case {case}");
    for (index, line) in lines.iter().enumerate() {
        let mut args = vec![file.clone(), "--outf=3".to_owned()];
        args.extend(line.iter().map(|s| (*s).to_owned()));
        mark(&format!("START {index}"));
        let result = if case == "near" {
            Application::new().run(args)
        } else {
            Application::new()
                .main(|ctl, files| {
                    mark("MAIN");
                    for path in files {
                        ctl.load(path)?;
                    }
                    ctl.ground(&[Part::base()])?;
                    let _ = ctl.solve(&[])?;
                    Ok(())
                })
                .run(args)
        };
        match result {
            Ok(code) => mark(&format!("RESULT {index} OK {code}")),
            Err(error) => mark(&format!(
                "RESULT {index} ERR {:?} {}",
                error.kind(),
                error.to_string().replace('\n', " ")
            )),
        }
    }
    mark("ALIVE");
}

fn run(case: &str) -> Option<child::Outcome> {
    child::run_child(ENTRY, case)
}

/// The `RESULT` line of command line `index` in the child's output.
fn result_of(stdout: &str, index: usize) -> &str {
    let prefix = format!("CHILD-RESULT {index} ");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| {
            panic!("no result for command line {index}; the process ended?\n{stdout}")
        })
}

fn check_refused(case: &str) {
    let Some(out) = run(case) else {
        return;
    };
    let lines = refused(case);
    for (index, line) in lines.iter().enumerate() {
        let result = result_of(&out.stdout, index);
        let expected = format!("ERR {:?}", ErrorKind::InvalidInput);
        assert!(
            result.starts_with(&expected),
            "{line:?}: expected a refusal, got {result}\n{}",
            out.stdout
        );
        // The message names an option of the command line.
        assert!(
            line.iter()
                .filter(|a| a.starts_with("--") && **a != "--" && !a.starts_with("--outf"))
                .any(|a| result.contains(*a)),
            "{line:?}: the message names none of the options: {result}"
        );
    }
    // Nothing of clingo's or the application's ran, and the process lived.
    assert!(!out.stdout.contains("CHILD-MAIN"), "{}", out.stdout);
    assert!(!out.stdout.contains("clingo version"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-ALIVE"), "{}", out.stdout);
    assert_eq!(out.code, Some(0), "{}\n{}", out.stdout, out.stderr);
}

#[test]
fn pre_in_every_form_is_refused_and_the_process_lives() {
    check_refused("refused_pre");
}

#[test]
fn print_portfolio_and_its_abbreviations_are_refused() {
    check_refused("refused_portfolio");
}

#[test]
fn text_with_output_is_refused() {
    check_refused("refused_text_output");
}

#[test]
fn output_spellings_with_text_are_refused() {
    check_refused("refused_output_forms");
}

#[test]
fn text_or_output_outside_gringo_mode_is_refused() {
    check_refused("refused_mode");
}

#[test]
fn a_lemma_file_or_atom_format_clasp_rejects_is_refused() {
    check_refused("refused_setup");
}

#[test]
fn near_misses_reach_clingo() {
    let Some(out) = run("near") else {
        return;
    };
    for (index, (line, code)) in NEAR.iter().enumerate() {
        let result = result_of(&out.stdout, index);
        assert_eq!(result, format!("OK {code}"), "{line:?}\n{}", out.stderr);
    }
    assert!(out.stdout.contains("CHILD-ALIVE"), "{}", out.stdout);
}

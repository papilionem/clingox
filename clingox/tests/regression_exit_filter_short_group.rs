//! `run` refuses the options that
//! end the process from inside clingo, and its filter must also see the
//! short options the application registered itself.
//!
//! clasp reads a short argument letter by letter (`handleShortOpt`,
//! `libpotassco/src/program_options.cpp`): a flag alias is taken and the
//! parse goes on with the next letter, an option that takes a value ends the
//! group and takes the rest of the argument (or the next argument) as its
//! value. With the application's flag `mine,m`, `-mo text` therefore means
//! `-m -o text` and `-motext` means `-m -o text` as well, and `-o` with
//! `--text` or a mode other than gringo ends the process with exit code 128.
//! Measured with pyclingo 5.8.2 (a probe application registering the flag
//! `mine,m` and the value option `kval,k`), one process per command line:
//!
//! - `-mo text --mode=clingo`, `-motext --text`, `-mo text --text` and
//!   `-mo text --mode=clasp` exit with 128;
//! - `-mkotext --text` and `-kotext --text` return 0: `k` takes `otext` as its
//!   value, so `-o` is never read;
//! - `-ko text --text` returns 128 as a value (`text` is read as a file that
//!   cannot be opened), `-mo=text --text` and `-om --text` return 1 (invalid
//!   value for `output`), `-mp` returns 1 (unknown option); `-mmo text
//!   --text` returns 1 too (the flag twice is a parse error that comes before
//!   the exclusion check), but a filter may refuse it as well, so it is not
//!   pinned;
//! - `-mo text --mode=gringo` returns 0 (`--output` is allowed there).
//!
//! Every case runs in a child, because a filter that misses the group lets
//! clingo end the process.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use clingox::ErrorKind;
use clingox::application::{Application, Flag, OptionSpec};

const ENTRY: &str = "child_entry";

/// Command lines with a grouped short `-o` that the run must refuse.
const REFUSED: &[&[&str]] = &[
    &["-mo", "text", "--mode=clingo"],
    &["-motext", "--text"],
    &["-mo", "text", "--text"],
    &["-mo", "text", "--mode=clasp"],
    &["--text", "-mo", "text"],
    &["-mo", "reify", "--text"],
];

/// Lines for an application flag whose alias is an unusual character (`#` in
/// the line stands for it): the alias must still be seen in a group.
const ODD_ALIAS: &[&[&str]] = &[
    &["-#o", "text", "--text"],
    &["-#otext", "--text"],
    &["-#o", "text", "--mode=clingo"],
];

/// Command lines clingo deals with itself, with the code it returns.
const NEAR: &[(&[&str], i32)] = &[
    (&["-mkotext", "--text"], 0),
    (&["-kotext", "--text"], 0),
    (&["-ko", "text", "--text"], 128),
    (&["-mo=text", "--text"], 1),
    (&["-om", "--text"], 1),
    (&["-mp"], 1),
    (&["-mo", "text", "--mode=gringo"], 0),
    (&["-m"], 30),
];

fn mark(text: &str) {
    println!("\nCHILD-{text}");
}

#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    let file = child::arg(&child::fixture("group.lp", "a."));
    let lines: Vec<Vec<&str>> = match case.as_str() {
        "refused" => REFUSED.iter().map(|l| l.to_vec()).collect(),
        "near" => NEAR.iter().map(|(l, _)| l.to_vec()).collect(),
        "at" | "comma" => ODD_ALIAS.iter().map(|l| l.to_vec()).collect(),
        _ => unreachable!(),
    };
    for (index, line) in lines.iter().enumerate() {
        let mut args = vec![file.as_str(), "--outf=3"];
        args.extend(line);
        let flag = Flag::new(false);
        let key = match case.as_str() {
            "at" => "mine,@",
            "comma" => "mine,,",
            _ => "mine,m",
        };
        let sigil = key.chars().last().unwrap();
        let args: Vec<String> = args
            .iter()
            .map(|a| a.replace('#', &sigil.to_string()))
            .collect();
        mark(&format!("START {index}"));
        let result = Application::new()
            .register_options(|options| {
                options.add_flag("App", key, "A flag", &flag)?;
                let spec = OptionSpec::new("App", "kval,k", "A value").argument("<v>");
                options.add(spec, |_| Ok(()))
            })
            .run(args);
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

fn result_of(stdout: &str, index: usize) -> &str {
    let prefix = format!("CHILD-RESULT {index} ");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| {
            panic!("no result for command line {index}; the process ended?\n{stdout}")
        })
}

#[test]
fn grouped_short_options_with_the_applications_alias_are_refused() {
    let Some(out) = child::run_child(ENTRY, "refused") else {
        return;
    };
    for (index, line) in REFUSED.iter().enumerate() {
        let result = result_of(&out.stdout, index);
        let expected = format!("ERR {:?}", ErrorKind::InvalidInput);
        assert!(
            result.starts_with(&expected),
            "{line:?}: {result}\n{}",
            out.stdout
        );
    }
    assert!(!out.stdout.contains("clingo version"), "{}", out.stdout);
    assert!(
        out.stdout.contains("CHILD-ALIVE"),
        "{}\n{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(out.code, Some(0), "{}\n{}", out.stdout, out.stderr);
}

#[test]
fn near_misses_of_the_grouped_form_reach_clingo() {
    let Some(out) = child::run_child(ENTRY, "near") else {
        return;
    };
    for (index, (line, code)) in NEAR.iter().enumerate() {
        let result = result_of(&out.stdout, index);
        assert_eq!(result, format!("OK {code}"), "{line:?}\n{}", out.stderr);
    }
    assert!(out.stdout.contains("CHILD-ALIVE"), "{}", out.stdout);
}

fn check_odd_alias(case: &str) {
    let Some(out) = child::run_child(ENTRY, case) else {
        return;
    };
    for (index, line) in ODD_ALIAS.iter().enumerate() {
        let result = result_of(&out.stdout, index);
        let expected = format!("ERR {:?}", ErrorKind::InvalidInput);
        assert!(
            result.starts_with(&expected),
            "{case} {line:?}: {result}\n{}",
            out.stdout
        );
    }
    assert!(
        out.stdout.contains("CHILD-ALIVE"),
        "{}\n{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(out.code, Some(0), "{}\n{}", out.stdout, out.stderr);
}

#[test]
fn an_alias_of_at_sign_is_seen_in_a_group() {
    check_odd_alias("at");
}

#[test]
fn an_alias_of_comma_is_seen_in_a_group() {
    check_odd_alias("comma");
}

//! What clingo prints for registered options, checked in child processes.
//!
//! `clingo_main` writes with C stdio, which libtest cannot capture, so the help
//! output and the error lines are read from a child (`common/child.rs`).
//! Expected values are from pyclingo 5.8.2 (clingo 5.8.2, clasp 3.4.1) at the C
//! level. Help lines are compared with their whitespace collapsed, because
//! the column of the colon depends on clingo's own longest entry.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use std::panic::{AssertUnwindSafe, catch_unwind};

use clingox::application::{Application, Flag, OptionSpec};
use clingox::{Error, ErrorKind, Result};

const ENTRY: &str = "child_entry";
const ERROR_LINE: &str = "*** ERROR: (clingo): ";
const TRY_LINE: &str = "Try '--help' for usage information";

#[allow(
    clippy::unnecessary_wraps,
    reason = "the signature of a parse callback"
)]
fn noop(_: &str) -> Result<()> {
    Ok(())
}

fn spec(name: &str) -> OptionSpec<'_> {
    OptionSpec::new("Test", name, "a test option")
}

/// An application whose callbacks print markers, so the parent can see which
/// of them ran.
fn marked() -> Application<'static> {
    Application::new()
        .register_options(|o| {
            println!("CHILD-REGISTER");
            o.add(OptionSpec::new("MyG", "zzopt", "an option"), noop)
        })
        .validate_options(|| {
            println!("CHILD-VALIDATE");
            Ok(())
        })
        .main(|_ctl, _files| {
            println!("CHILD-MAIN");
            Ok(())
        })
}

/// The help layout application: two groups, the first used at two times.
fn layout() -> Application<'static> {
    let flag = Flag::new(false);
    Application::new().register_options(move |o| {
        o.add(OptionSpec::new("MyG", "plain", "plain option"), noop)?;
        o.add(OptionSpec::new("Other", "oth", "other option"), noop)?;
        o.add(
            OptionSpec::new("MyG", "al,a", "alias option").argument("<n>"),
            noop,
        )?;
        o.add(
            OptionSpec::new("MyG", "mu", "multi opt")
                .multi()
                .argument("<x>"),
            noop,
        )?;
        o.add(
            OptionSpec::new("MyG", "ar", "arg no brackets").argument("val"),
            noop,
        )?;
        o.add(OptionSpec::new("MyG", "ae", "empty arg").argument(""), noop)?;
        o.add_flag("MyG", "fl", "a flag", &flag)
    })
}

fn levels() -> Application<'static> {
    Application::new().register_options(|o| {
        o.add(OptionSpec::new("MyG", "plain", "plain option"), noop)?;
        o.add(OptionSpec::new("MyG", "lvl1,@1", "level one option"), noop)?;
        o.add(OptionSpec::new("MyG", "lvl2,@2", "level two option"), noop)?;
        o.add(
            OptionSpec::new("MyG", "lvl3,@3", "level three option"),
            noop,
        )
    })
}

fn percent() -> Application<'static> {
    let flag = Flag::new(false);
    Application::new().register_options(move |o| {
        o.add(
            OptionSpec::new("MyG", "pct", "100% sure %A %D %% end%"),
            noop,
        )?;
        o.add_flag("MyG", "pf", "flag 100% sure %A %% end%", &flag)
    })
}

fn report(result: Result<i32>) {
    match result {
        Ok(code) => println!("CHILD-RC {code}"),
        Err(error) => println!("CHILD-ERR {:?} {error}", error.kind()),
    }
}

/// The child side: runs the case named by the environment, or does nothing.
#[test]
#[allow(clippy::too_many_lines, reason = "one arm per child case")]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    match case.as_str() {
        "help_levels_1" => report(levels().run(["--help"])),
        "help_levels_2" => report(levels().run(["--help=2"])),
        "help_levels_3" => report(levels().run(["--help=3"])),
        "help_layout" => report(layout().run(["--help"])),
        "help_percent" => report(percent().run(["--help"])),
        "events_help" => report(marked().run(["--help"])),
        "events_version" => report(marked().run(["--version"])),
        "events_unknown" => report(marked().run(["--zznosuch"])),
        "parse_fails" => report(
            Application::new()
                .register_options(|o| {
                    o.add(spec("zzmyopt"), |_| {
                        Err(Error::new(ErrorKind::Runtime, "my parse message"))
                    })
                })
                .main(|_ctl, _files| Ok(()))
                .run(["--zzmyopt=zz"]),
        ),
        "register_fails" => report(
            Application::new()
                .register_options(|_o| Err(Error::new(ErrorKind::Runtime, "my register message")))
                .main(|_ctl, _files| Ok(()))
                .run(["--outf=3"]),
        ),
        "validate_fails" => report(
            Application::new()
                .validate_options(|| Err(Error::new(ErrorKind::Logic, "my validate message")))
                .main(|_ctl, _files| Ok(()))
                .run(["--outf=3"]),
        ),
        "duplicate" => report(
            Application::new()
                .register_options(|o| o.add(spec("models"), noop))
                .main(|_ctl, _files| Ok(()))
                .run(["--outf=3"]),
        ),
        "alias_clash" => report(
            Application::new()
                .register_options(|o| o.add(spec("zzt,t"), noop))
                .main(|_ctl, _files| Ok(()))
                .run(["--outf=3"]),
        ),
        "ambiguous" => report(
            Application::new()
                .register_options(|o| {
                    o.add(spec("zzzab"), noop)?;
                    o.add(spec("zzzac"), noop)
                })
                .main(|_ctl, _files| Ok(()))
                .run(["--outf=3", "--zzza=1"]),
        ),
        "requires_value" => report(
            Application::new()
                .register_options(|o| o.add(spec("zzv"), noop))
                .main(|_ctl, _files| Ok(()))
                .run(["--outf=3", "--zzv"]),
        ),
        "flag_value" => {
            let flag = Flag::new(false);
            report(
                Application::new()
                    .register_options(|o| o.add_flag("Test", "zzfl", "a flag", &flag))
                    .main(|_ctl, _files| Ok(()))
                    .run(["--outf=3", "--zzfl=yes"]),
            );
        }
        "multiple" => report(
            Application::new()
                .register_options(|o| o.add(spec("zzs"), noop))
                .main(|_ctl, _files| Ok(()))
                .run(["--outf=3", "--zzs=1", "--zzs=2"]),
        ),
        "panic_parse" | "panic_register" | "panic_validate" => {
            let which = case.clone();
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                Application::new()
                    .register_options(|o| {
                        assert!(which != "panic_register", "boom in register");
                        o.add(spec("zzo"), |_| panic!("boom in parse"))
                    })
                    .validate_options(|| panic!("boom in validate"))
                    .main(|_ctl, _files| Ok(()))
                    .run(match which.as_str() {
                        "panic_parse" => vec!["--outf=3", "--zzo=1"],
                        _ => vec!["--outf=3"],
                    })
            }));
            match outcome {
                Ok(result) => report(result),
                Err(panic) => {
                    let text = panic
                        .downcast_ref::<&str>()
                        .map(ToString::to_string)
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    println!("CHILD-CAUGHT {text}");
                }
            }
        }
        other => panic!("unknown child case {other}"),
    }
}

fn run(case: &str) -> Option<child::Outcome> {
    child::run_child(ENTRY, case)
}

/// The lines of `text` with their whitespace collapsed.
fn normalised(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

/// The normalised lines of the help section that starts at the header `group`
/// (`MyG:`) and ends at the next header, a line that does not start with
/// whitespace.
fn section(stdout: &str, group: &str) -> Vec<String> {
    let header = format!("{group}:");
    let mut lines = stdout.lines().skip_while(|line| *line != header);
    assert!(lines.next().is_some(), "no `{header}` in\n{stdout}");
    lines
        .take_while(|line| line.is_empty() || line.starts_with(' '))
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect()
}

fn error_lines(stderr: &str) -> Vec<&str> {
    stderr.lines().filter(|l| l.contains(ERROR_LINE)).collect()
}

// ---- help output ----

#[test]
fn options_are_listed_by_help_level() {
    let cases = [
        ("help_levels_1", vec!["--plain=<arg> : plain option"]),
        (
            "help_levels_2",
            vec![
                "--plain=<arg> : plain option",
                "--lvl1=<arg> : level one option",
            ],
        ),
        (
            "help_levels_3",
            vec![
                "--plain=<arg> : plain option",
                "--lvl1=<arg> : level one option",
                "--lvl2=<arg> : level two option",
            ],
        ),
    ];
    for (case, expected) in cases {
        let Some(out) = run(case) else {
            return;
        };
        assert!(out.stdout.contains("CHILD-RC 0"), "{case}: {}", out.stdout);
        assert_eq!(section(&out.stdout, "MyG"), expected, "{case}");
        // Level 3 is never listed.
        assert!(!out.stdout.contains("lvl3"), "{case}");
    }
}

#[test]
fn help_shows_group_alias_placeholder_multi_and_flag() {
    let Some(out) = run("help_layout") else {
        return;
    };
    assert!(out.stdout.contains("CHILD-RC 0"), "{}", out.stdout);
    // Options added to `MyG` at different times form one section, in
    // registration order; the other group is separate.
    assert_eq!(
        out.stdout.lines().filter(|l| *l == "MyG:").count(),
        1,
        "{}",
        out.stdout
    );
    assert_eq!(
        section(&out.stdout, "MyG"),
        [
            "--plain=<arg> : plain option",
            "--al,-a <n> : alias option",
            "--mu=<x> : multi opt",
            "--ar=val : arg no brackets",
            "--ae= : empty arg",
            "--[no-]fl : a flag",
        ]
    );
    assert_eq!(
        section(&out.stdout, "Other"),
        ["--oth=<arg> : other option"]
    );
    // The group headers are the caller's text, with no suffix added.
    assert!(!out.stdout.contains("MyG Options"), "{}", out.stdout);
}

#[test]
fn a_percent_sign_in_a_description_is_printed_literally() {
    let Some(out) = run("help_percent") else {
        return;
    };
    // Without escaping, clingo's help formatter would print `100 sure <arg>  %
    // end` (measured): `%` and the next character are an escape there.
    assert_eq!(
        section(&out.stdout, "MyG"),
        [
            "--pct=<arg> : 100% sure %A %D %% end%",
            // A flag's description goes through the same formatter
            // (pyclingo 5.8.2: unescaped `100% sure %A end%` prints
            // `100 sure  end`).
            "--[no-]pf : flag 100% sure %A %% end%",
        ],
        "{}",
        out.stdout
    );
}

#[test]
fn register_runs_for_help_version_and_unknown_options_but_validate_does_not() {
    for (case, code) in [
        ("events_help", "CHILD-RC 0"),
        ("events_version", "CHILD-RC 0"),
        ("events_unknown", "CHILD-RC 1"),
    ] {
        let Some(out) = run(case) else {
            return;
        };
        assert!(out.stdout.contains(code), "{case}: {}", out.stdout);
        assert_eq!(
            out.stdout.matches("CHILD-REGISTER").count(),
            1,
            "{case}: {}",
            out.stdout
        );
        assert!(!out.stdout.contains("CHILD-VALIDATE"), "{case}");
        assert!(!out.stdout.contains("CHILD-MAIN"), "{case}");
    }
    let help = run("events_help").unwrap();
    assert!(help.stdout.contains("--zzopt=<arg>"), "{}", help.stdout);
    let version = run("events_version").unwrap();
    assert!(!version.stdout.contains("zzopt"), "{}", version.stdout);
    assert!(
        version.stdout.contains("libclingo version"),
        "{}",
        version.stdout
    );
}

// ---- what clingo prints when a callback fails ----

#[test]
fn a_failing_parse_prints_clingos_text_and_run_returns_the_callbacks_error() {
    let Some(out) = run("parse_fails") else {
        return;
    };
    // clingo discards the callback's message (F7): its own text names the value
    // and the option. `run` still returns the callback's error.
    assert!(
        out.stderr.contains("'zz' invalid value for: 'zzmyopt'"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains(TRY_LINE), "{}", out.stderr);
    assert!(!out.stderr.contains("my parse message"), "{}", out.stderr);
    assert!(
        out.stdout.contains("CHILD-ERR Runtime my parse message"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("CHILD-RC"), "{}", out.stdout);
}

#[test]
fn a_failing_register_options_prints_its_message_and_run_returns_it() {
    let Some(out) = run("register_fails") else {
        return;
    };
    let lines = error_lines(&out.stderr);
    assert_eq!(lines.len(), 1, "{}", out.stderr);
    assert!(lines[0].contains("my register message"), "{}", lines[0]);
    assert!(out.stderr.contains(TRY_LINE), "{}", out.stderr);
    assert!(
        out.stdout.contains("CHILD-ERR Runtime my register message"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_failing_validate_options_prints_its_message_and_run_returns_it() {
    let Some(out) = run("validate_fails") else {
        return;
    };
    let lines = error_lines(&out.stderr);
    assert_eq!(lines.len(), 1, "{}", out.stderr);
    assert!(lines[0].contains("my validate message"), "{}", lines[0]);
    assert!(out.stderr.contains(TRY_LINE), "{}", out.stderr);
    // clingo itself would report exit code 0 here (U39); `run` returns the
    // error.
    assert!(
        out.stdout.contains("CHILD-ERR Logic my validate message"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("CHILD-RC"), "{}", out.stdout);
}

#[test]
fn clingos_own_command_line_errors_are_exit_code_one_with_its_text() {
    let cases = [
        ("duplicate", "duplicate option: 'models'"),
        ("alias_clash", "duplicate option: 'zzt'"),
        ("ambiguous", "ambiguous option: 'zzza'"),
        ("requires_value", "'zzv' requires a value"),
        ("flag_value", "'zzfl' does not take a value"),
        ("multiple", "multiple occurrences: 'zzs'"),
    ];
    for (case, text) in cases {
        let Some(out) = run(case) else {
            return;
        };
        assert!(out.stdout.contains("CHILD-RC 1"), "{case}: {}", out.stdout);
        assert!(out.stderr.contains(text), "{case}: {}", out.stderr);
        assert!(out.stderr.contains(TRY_LINE), "{case}: {}", out.stderr);
    }
}

#[test]
fn a_panic_in_an_options_callback_is_resumed_after_clingo_printed_one_error() {
    for (case, text) in [
        ("panic_register", "boom in register"),
        ("panic_parse", "boom in parse"),
        ("panic_validate", "boom in validate"),
    ] {
        let Some(out) = run(case) else {
            return;
        };
        assert!(
            out.stdout.contains(&format!("CHILD-CAUGHT {text}")),
            "{case}: {}",
            out.stdout
        );
        assert_eq!(error_lines(&out.stderr).len(), 1, "{case}: {}", out.stderr);
        assert_eq!(out.code, Some(0), "{case}: the process is alive");
    }
}

#[test]
fn normalising_collapses_whitespace() {
    assert_eq!(normalised("  --a=<b>      :  text \n"), ["--a=<b> : text"]);
}

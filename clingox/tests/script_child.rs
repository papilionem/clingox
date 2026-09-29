//! Custom scripting languages that need a fresh registry, C standard output or
//! error, or an exit code, so each case runs in a child process.
//!
//! Oracle: pyclingo 5.8.2 (clingo 5.8.2), `clingo_register_script` and
//! `clingo_main` at the C level. The registry and its freeze rule are per
//! process, so no two cases may share one. A child asserts inside and prints
//! `CHILD-DONE` when every assertion held; the parent checks the exit code,
//! that marker and the C output. Cases are skipped visibly where `can_spawn()`
//! is false (WebAssembly, Miri).

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::too_many_lines,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;
#[path = "common/script.rs"]
mod probe;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use clingox::application::Application;
use clingox::ast::Span;
use clingox::script::{self, Script};
use clingox::{Control, ErrorKind, Part, Result, Symbol};
use probe::{Event, Fail, Probe};

const ENTRY: &str = "child_entry";

/// A language with nothing to do.
struct Noop;

impl Script for Noop {
    fn execute(&self, _span: &Span, _code: &str) -> Result<()> {
        Ok(())
    }
}

/// Answers `f` with one number, and logs who was asked.
struct Fixed {
    label: &'static str,
    value: i32,
    log: Arc<Mutex<Vec<String>>>,
}

impl Script for Fixed {
    fn execute(&self, _span: &Span, _code: &str) -> Result<()> {
        Ok(())
    }

    fn callable(&self, name: &str) -> Result<bool> {
        self.log
            .lock()
            .unwrap()
            .push(format!("{} callable {name}", self.label));
        Ok(name == "f")
    }

    fn call(&self, _span: &Span, _name: &str, _arguments: &[Symbol]) -> Result<Vec<Symbol>> {
        Ok(vec![Symbol::number(self.value)])
    }
}

/// Writes `DROPPED` when dropped: it must never be, `free` is NULL.
struct Loud;

impl Script for Loud {
    fn execute(&self, _span: &Span, _code: &str) -> Result<()> {
        Ok(())
    }

    fn callable(&self, name: &str) -> Result<bool> {
        Ok(name == "loud_f")
    }

    fn call(&self, _span: &Span, _name: &str, _arguments: &[Symbol]) -> Result<Vec<Symbol>> {
        Ok(vec![Symbol::number(1)])
    }
}

impl Drop for Loud {
    fn drop(&mut self) {
        println!("DROPPED");
    }
}

fn done() {
    println!("CHILD-DONE");
}

fn file(name: &str, text: &str) -> String {
    child::arg(&child::fixture(name, text))
}

fn refused(name: &str) -> bool {
    script::register(name, "1", Noop).is_err_and(|e| e.kind() == ErrorKind::InvalidInput)
}

/// The child side: runs the case named by the environment, or does nothing.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    match case.as_str() {
        "freeze_new" | "freeze_with_args" | "freeze_builder" => {
            script::register("s1", "1", Noop).unwrap();
            let ctl = match case.as_str() {
                "freeze_new" => Control::new(),
                "freeze_with_args" => Control::with_args(["0"]),
                _ => Control::builder().build(),
            }
            .unwrap();
            drop(ctl);
            // Sticky: the control is gone and registration is still refused.
            assert!(refused("s2"));
            assert_eq!(script::version("s1").as_deref(), Some("1"));
            assert_eq!(script::version("s2"), None);
            done();
        }
        "freeze_run" => {
            script::register("s1", "1", Noop).unwrap();
            let path = file("script_child_run.lp", "a. {b}.");
            let code = Application::new().run([path.as_str(), "--outf=3"]).unwrap();
            assert_eq!(code, 10);
            assert!(refused("s2"));
            done();
        }
        "no_freeze_on_rejected_run" => {
            // The argument check rejects the run before clingo is reached.
            let err = Application::new().run(["--fast-exit"]).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            script::register("s1", "1", Noop).unwrap();
            done();
        }
        "freeze_on_failed_control" => {
            // clingo was called and refused the option: the freeze is set.
            assert!(Control::with_args(["--nosuch"]).is_err());
            assert!(refused("s1"));
            done();
        }
        "precedence_both" | "precedence_only_second" => {
            let log = Arc::new(Mutex::new(Vec::new()));
            for (label, value) in [("foo", 1), ("bar", 2)] {
                let script = Fixed {
                    label,
                    value,
                    log: Arc::clone(&log),
                };
                script::register(label, "1", script).unwrap();
            }
            let mut ctl = Control::builder().args(["0"]).build().unwrap();
            let (program, expect, unasked) = if case == "precedence_both" {
                (
                    "#script (bar) y #end. #script (foo) x #end. p(@f(0)).",
                    "p(1)",
                    "bar callable f",
                )
            } else {
                ("#script (bar) y #end. p(@f(0)).", "p(2)", "foo callable f")
            };
            ctl.add_base(program).unwrap();
            ctl.ground(&[Part::base()]).unwrap();
            let (_, models) = ctl.solve_all().unwrap();
            let shown: Vec<String> = models[0]
                .symbols()
                .iter()
                .map(ToString::to_string)
                .collect();
            assert_eq!(shown, [expect]);
            let log = log.lock().unwrap();
            if case == "precedence_both" {
                // Registration order, not the order the blocks ran in.
                assert_eq!(*log, ["foo callable f"]);
            } else {
                // Only a script that has run a block is asked at all.
                assert_eq!(*log, ["bar callable f"]);
            }
            assert!(!log.iter().any(|l| l == unasked), "{log:?}");
            done();
        }
        "banner_registered" | "banner_none" | "banner_other_name" => {
            if case == "banner_registered" {
                script::register("python", "9.9", Noop).unwrap();
                script::register("lua", "8.8", Noop).unwrap();
                assert_eq!(script::version("python").as_deref(), Some("9.9"));
            } else if case == "banner_other_name" {
                script::register("foo", "7.7", Noop).unwrap();
            }
            let code = Application::new().run(["--version"]).unwrap();
            println!("CHILD-RC {code}");
            done();
        }
        "main_nothing" | "main_fails" => {
            let (probe, state) = Probe::new("mn_");
            script::register("mn", "1", probe).unwrap();
            state.want_main(true);
            if case == "main_fails" {
                state.set_main(Some(Arc::new(|_: &mut clingox::ScopedControl<'_>| {
                    Err(clingox::Error::new(
                        ErrorKind::Runtime,
                        "script main failed",
                    ))
                })));
            }
            let path = file("script_child_main.lp", "#script (mn) hi #end. a.");
            match Application::new().run([path.as_str()]) {
                Ok(code) => println!("CHILD-RC {code}"),
                Err(err) => {
                    assert_eq!(err.kind(), ErrorKind::Runtime);
                    assert!(err.to_string().contains("script main failed"), "{err}");
                    println!("CHILD-ERR");
                }
            }
            println!("CHILD-ALIVE");
            done();
        }
        "main_hijack" => {
            let (probe, state) = Probe::new("mn_");
            script::register("mn", "1", probe).unwrap();
            state.want_main(true);
            let with_block = file("script_child_hijack1.lp", "#script (mn) hi #end. a.");
            let without = file("script_child_hijack2.lp", "a.");
            let count = || state.take().iter().filter(|e| **e == Event::Main).count();
            assert_eq!(
                Application::new()
                    .run([with_block.as_str(), "--outf=3"])
                    .unwrap(),
                0
            );
            assert_eq!(count(), 1);
            // The flag "a script has run a block" is process-wide and never
            // reset, so a run with no block at all is taken over too.
            assert_eq!(
                Application::new()
                    .run([without.as_str(), "--outf=3"])
                    .unwrap(),
                0
            );
            assert_eq!(count(), 1, "the second run reached the script main");
            // An application main callback wins.
            let code = Application::new()
                .main(|_, _| Ok(()))
                .run([without.as_str(), "--outf=3"])
                .unwrap();
            assert_eq!(code, 0);
            assert_eq!(count(), 0, "the script main did not run");
            done();
        }
        "exit_clean" => {
            script::register("loud", "1", Loud).unwrap();
            let mut ctl = Control::new().unwrap();
            ctl.add_base("#script (loud) x #end. p(@loud_f(0)).")
                .unwrap();
            ctl.ground(&[Part::base()]).unwrap();
            let grounded = thread::spawn(|| {
                let mut ctl = Control::new().unwrap();
                ctl.add_base("#script (loud) y #end. q(@loud_f(0)).")
                    .unwrap();
                ctl.ground(&[Part::base()]).unwrap();
            });
            grounded.join().unwrap();
            drop(ctl);
            done();
        }
        "script_error_in_run" => {
            let (probe, state) = Probe::new("mn_");
            script::register("mn", "1", probe).unwrap();
            state.fail_exec(Some((Fail::Runtime, "exec died in run")));
            let path = file("script_child_execfail.lp", "#script (mn) hi #end. a.");
            let err = Application::new().run([path.as_str()]).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Runtime);
            assert!(err.to_string().contains("exec died in run"), "{err}");
            println!("CHILD-ERR");
            done();
        }
        "register_race" => {
            if !clingox_sys::HAS_THREADS {
                println!("CHILD-SKIPPED");
                done();
                return;
            }
            let barrier = Arc::new(Barrier::new(2));
            let creator = {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    Control::new().unwrap()
                })
            };
            barrier.wait();
            let attempted = AtomicUsize::new(0);
            let mut results = Vec::new();
            for n in 0..2000 {
                let name = format!("r{n}");
                let accepted = script::register(&name, "1", Noop).is_ok();
                attempted.fetch_add(1, Ordering::SeqCst);
                results.push((name, accepted));
                if !accepted {
                    break;
                }
            }
            let ctl = creator.join().unwrap();
            // Accepted ones form a prefix, all visible; the first refusal, if
            // any, is the last attempt and is not registered.
            let accepted = results.iter().take_while(|(_, ok)| *ok).count();
            assert!(results[accepted..].len() <= 1);
            for (name, ok) in &results {
                assert_eq!(script::version(name).is_some(), *ok, "{name}");
            }
            assert!(refused("late"), "a control exists now");
            drop(ctl);
            done();
        }
        other => panic!("unknown child case {other}"),
    }
}

fn run(case: &str) -> Option<child::Outcome> {
    child::run_child(ENTRY, case)
}

/// A child that must have passed every assertion.
fn passed(case: &str) -> Option<child::Outcome> {
    let out = run(case)?;
    assert_eq!(out.code, Some(0), "{case}: {}{}", out.stdout, out.stderr);
    assert_eq!(out.signal, None, "{case}");
    assert!(
        out.stdout.contains("CHILD-DONE"),
        "{case}: {}{}",
        out.stdout,
        out.stderr
    );
    Some(out)
}

#[test]
fn registration_is_refused_after_any_constructor_made_a_control() {
    for case in ["freeze_new", "freeze_with_args", "freeze_builder"] {
        passed(case);
    }
}

#[test]
fn registration_is_refused_after_an_application_ran() {
    passed("freeze_run");
}

#[test]
fn a_run_rejected_before_clingo_does_not_freeze_registration() {
    passed("no_freeze_on_rejected_run");
}

#[test]
fn a_control_clingo_refused_still_freezes_registration() {
    passed("freeze_on_failed_control");
}

#[test]
fn scripts_are_asked_in_registration_order_and_only_after_running_a_block() {
    passed("precedence_both");
    passed("precedence_only_second");
}

#[test]
fn version_lists_python_and_lua_scripts_only() {
    let Some(out) = passed("banner_registered") else {
        return;
    };
    assert!(
        out.stdout
            .contains("Configuration: with Python 9.9, with Lua 8.8"),
        "{}",
        out.stdout
    );
    for case in ["banner_none", "banner_other_name"] {
        let out = passed(case).unwrap();
        assert!(
            out.stdout
                .contains("Configuration: without Python, without Lua"),
            "{case}: {}",
            out.stdout
        );
    }
}

#[test]
fn a_script_main_that_does_nothing_ends_the_run_unsolved() {
    let Some(out) = passed("main_nothing") else {
        return;
    };
    // The file was read, nothing was solved: clingo's summary shows it.
    assert!(out.stdout.contains("Reading from"), "{}", out.stdout);
    assert!(out.stdout.contains("UNKNOWN"), "{}", out.stdout);
    assert!(out.stdout.contains("Models       : 0+"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-RC 0"), "{}", out.stdout);
    assert!(!out.stdout.contains("Answer"), "{}", out.stdout);
}

#[test]
fn a_failing_script_main_is_reported_and_returned() {
    let Some(out) = passed("main_fails") else {
        return;
    };
    assert!(
        out.stderr.contains("*** ERROR: (clingo): ") && out.stderr.contains("script main failed"),
        "{}",
        out.stderr
    );
    assert!(out.stdout.contains("CHILD-ERR"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-ALIVE"), "{}", out.stdout);
}

#[test]
fn a_script_main_takes_over_every_later_default_run_and_yields_to_an_application_main() {
    passed("main_hijack");
}

#[test]
fn the_process_exits_cleanly_and_a_script_is_never_dropped() {
    let Some(out) = passed("exit_clean") else {
        return;
    };
    assert!(!out.stdout.contains("DROPPED"), "{}", out.stdout);
    assert!(!out.stderr.contains("DROPPED"), "{}", out.stderr);
}

#[test]
fn a_script_error_during_the_default_parse_is_reported_and_returned() {
    let Some(out) = passed("script_error_in_run") else {
        return;
    };
    assert!(
        out.stderr.contains("*** ERROR: (clingo): ") && out.stderr.contains("exec died in run"),
        "{}",
        out.stderr
    );
}

#[test]
fn registering_while_another_thread_creates_a_control_is_consistent() {
    passed("register_race");
}

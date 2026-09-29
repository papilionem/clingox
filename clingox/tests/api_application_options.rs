//! `Application::register_options`, `validate_options`, `Options`,
//! `OptionSpec` and `Flag`, in process.
//!
//! Expected values come from pyclingo 5.8.2 (clingo 5.8.2, clasp 3.4.1)
//! driving `clingo_main` at the C level. The help output and the exact
//! standard-error texts are checked in `application_options_child.rs`.
//!
//! Option names start with `zz` so that clingo's prefix matching never
//! collides with one of its own options. Callbacks that share state use
//! `RefCell`: `parse`, `validate_options` and `main` are separate closures, and
//! two of them cannot borrow one variable mutably (the compile-fail case
//! `application_options_shared_mut`).
//!
//! `run` refuses a second run while one is in progress, so every test takes one
//! lock. No test spawns a thread, so the file runs unchanged on WebAssembly.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test helpers fail loudly, and a skipped case says so"
)]

#[path = "common/child.rs"]
mod child;

use std::cell::{Cell, RefCell};
use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, MutexGuard};

use clingox::application::{Application, Flag, OptionSpec, Options};
use clingox::{Error, ErrorKind, MessageCode, Part, Result, ShowType};

const SAT: &str = "a. {b}.";
const ORACLE: &str = "1 {a; b; c(1/0)}.";
const PARTS: &str = "#program base. b. #program foo. f. {g}. #program bar(k). h(k).";

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

type Log = RefCell<Vec<String>>;

fn spec(name: &str) -> OptionSpec<'_> {
    OptionSpec::new("Test", name, "a test option")
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "the signature of a parse callback"
)]
fn noop(_: &str) -> Result<()> {
    Ok(())
}

fn kind_of(result: Result<()>) -> Option<ErrorKind> {
    result.err().map(|error| error.kind())
}

/// Every argument list here starts with the quiet switch; there is no file, so
/// nothing is read from standard input (an application with a `main`).
fn args<'a>(rest: &[&'a str]) -> Vec<&'a str> {
    let mut all = vec!["--outf=3"];
    all.extend_from_slice(rest);
    all
}

/// An application with a `main` that does not solve (exit code 0), whose
/// options are `(key, multi)`; it logs `register`, `<name>=<value>` for every
/// parsed value, `validate` and `main <files>`.
fn probe<'a>(log: &'a Log, options: &[(&str, bool)]) -> Application<'a> {
    let options: Vec<(String, bool)> = options
        .iter()
        .map(|(key, multi)| ((*key).to_owned(), *multi))
        .collect();
    Application::new()
        .register_options(move |o| {
            log.borrow_mut().push("register".into());
            for (key, multi) in &options {
                let mut option = spec(key);
                if *multi {
                    option = option.multi();
                }
                let label = key.split(',').next().unwrap().to_owned();
                o.add(option, move |value| {
                    log.borrow_mut().push(format!("{label}={value}"));
                    Ok(())
                })?;
            }
            Ok(())
        })
        .validate_options(move || {
            log.borrow_mut().push("validate".into());
            Ok(())
        })
        .main(move |_ctl, files| {
            log.borrow_mut().push(format!("main {}", files.len()));
            Ok(())
        })
}

fn logged(log: &Log) -> Vec<String> {
    log.borrow().clone()
}

/// A run that must work: proves the run flag is free and nothing is left over.
fn sane() {
    let code = Application::new()
        .main(|_ctl, _files| Ok(()))
        .run(args(&[]))
        .unwrap();
    assert_eq!(code, 0);
}

// ---- A1: the oracle port ----

type Oracle = (
    i32,
    Vec<String>,
    Vec<Vec<String>>,
    Vec<(MessageCode, String)>,
);

fn oracle(with_flag: bool) -> Oracle {
    let file = child::fixture("opts_oracle.lp", ORACLE);
    let path = child::arg(&file);
    let events: Log = RefCell::new(Vec::new());
    let models: RefCell<Vec<Vec<String>>> = RefCell::new(Vec::new());
    let messages = Mutex::new(Vec::new());
    let flag = Flag::new(false);
    let mut arguments = vec![path.as_str(), "--outf=3", "0", "--test=x"];
    if with_flag {
        arguments.push("--flag");
    }
    let code = Application::new()
        .program_name("test")
        .version("1.2.3")
        .message_limit(17)
        .logger(|code, text| messages.lock().unwrap().push((code, text.to_owned())))
        .register_options(|o| {
            events.borrow_mut().push("register".into());
            o.add(
                OptionSpec::new("Clingo.Test", "test", "an option"),
                |value| {
                    events.borrow_mut().push(format!("parse {value}"));
                    Ok(())
                },
            )?;
            o.add_flag("Clingo.Test", "flag", "a flag", &flag)?;
            Ok(())
        })
        .validate_options(|| {
            events.borrow_mut().push("validate".into());
            events.borrow_mut().push(format!("flag {}", flag.get()));
            Ok(())
        })
        .main(|ctl, files| {
            events.borrow_mut().push(format!("main {}", files.len()));
            for file in files {
                ctl.load(file)?;
            }
            ctl.ground(&[Part::base()])?;
            let _ = ctl.for_each_model(&[], |model| {
                let mut symbols: Vec<String> = model
                    .symbols(ShowType::SHOWN)?
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                symbols.sort();
                models.borrow_mut().push(symbols);
                Ok(ControlFlow::Continue(()))
            })?;
            Ok(())
        })
        .run(arguments)
        .unwrap();
    assert_eq!(flag.get(), with_flag, "the flag after the run");
    (
        code,
        events.into_inner(),
        models.into_inner(),
        messages.into_inner().unwrap(),
    )
}

#[test]
fn the_oracle_test_app_matches() {
    let _lock = serial();
    let (code, events, mut models, messages) = oracle(true);
    assert_eq!(code, 30);
    assert_eq!(
        events,
        ["register", "parse x", "validate", "flag true", "main 1"]
    );
    // The emission order on this build is a, b, a b; the set is
    // what the upstream test means.
    models.sort();
    assert_eq!(
        models,
        [["a"].to_vec(), ["a", "b"].to_vec(), ["b"].to_vec()]
    );
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].0, MessageCode::OperationUndefined);
    assert!(
        messages[0]
            .1
            .ends_with("1:12-15: info: operation undefined:\n  (1/0)"),
        "{:?}",
        messages[0].1
    );
}

#[test]
fn without_the_flag_argument_the_flag_reads_false() {
    let _lock = serial();
    let (code, events, _models, _messages) = oracle(false);
    assert_eq!(code, 30);
    assert_eq!(
        events,
        ["register", "parse x", "validate", "flag false", "main 1"]
    );
}

// ---- A2: multi options and the order of parsing ----

#[test]
fn a_multi_option_delivers_every_occurrence_in_order() {
    let _lock = serial();
    let log = Log::default();
    let code = probe(&log, &[("zzmulti", true)])
        .run(args(&["--zzmulti=1", "--zzmulti=2", "--zzmulti=3"]))
        .unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        logged(&log),
        [
            "register",
            "zzmulti=1",
            "zzmulti=2",
            "zzmulti=3",
            "validate",
            "main 0"
        ]
    );
}

#[test]
fn a_multi_option_that_is_absent_is_never_called() {
    let _lock = serial();
    let log = Log::default();
    probe(&log, &[("zzmulti", true)]).run(args(&[])).unwrap();
    assert_eq!(logged(&log), ["register", "validate", "main 0"]);
}

#[test]
fn a_single_option_given_twice_ends_in_exit_code_one() {
    let _lock = serial();
    let log = Log::default();
    let code = probe(&log, &[("zzsingle", false)])
        .run(args(&["--zzsingle=1", "--zzsingle=2"]))
        .unwrap();
    assert_eq!(code, 1, "multiple occurrences");
    // The first value was delivered, the second was not, and neither validate
    // nor main ran.
    assert_eq!(logged(&log), ["register", "zzsingle=1"]);
}

#[test]
fn values_arrive_in_command_line_order_across_options() {
    let _lock = serial();
    let log = Log::default();
    probe(&log, &[("zzaa", true), ("zzbb", true)])
        .run(args(&["--zzaa=1", "--zzbb=2", "--zzaa=3"]))
        .unwrap();
    assert_eq!(
        logged(&log),
        [
            "register", "zzaa=1", "zzbb=2", "zzaa=3", "validate", "main 0"
        ]
    );
    // Not registration order: `zzbb` was given first.
    let log = Log::default();
    probe(&log, &[("zzaa", false), ("zzbb", false)])
        .run(args(&["--zzbb=2", "--zzaa=1"]))
        .unwrap();
    assert_eq!(
        logged(&log),
        ["register", "zzbb=2", "zzaa=1", "validate", "main 0"]
    );
}

// ---- A3: a failing parse callback ----

#[test]
fn a_failing_parse_returns_the_callbacks_own_error() {
    let _lock = serial();
    for kind in [ErrorKind::Runtime, ErrorKind::Logic, ErrorKind::Callback] {
        let log = Log::default();
        let error = Application::new()
            .register_options(|o| {
                o.add(spec("zzbad"), |value| {
                    log.borrow_mut().push(format!("bad={value}"));
                    Err(Error::new(kind, "port is out of range"))
                })?;
                o.add(spec("zzlater"), |value| {
                    log.borrow_mut().push(format!("later={value}"));
                    Ok(())
                })?;
                Ok(())
            })
            .validate_options(|| {
                log.borrow_mut().push("validate".into());
                Ok(())
            })
            .main(|_ctl, _files| {
                log.borrow_mut().push("main".into());
                Ok(())
            })
            .run(args(&["--zzbad=x", "--zzlater=y"]))
            .unwrap_err();
        assert_eq!(error.kind(), kind);
        assert!(
            error.to_string().contains("port is out of range"),
            "{error}"
        );
        // Parsing stopped at the failure: the later option, validate and main
        // never ran.
        assert_eq!(logged(&log), ["bad=x"], "{kind:?}");
        sane();
    }
}

#[test]
fn a_multi_option_that_fails_on_its_second_value_never_sees_the_third() {
    let _lock = serial();
    let seen = Cell::new(0_u32);
    let error = Application::new()
        .register_options(|o| {
            o.add(spec("zzm").multi(), |_| {
                seen.set(seen.get() + 1);
                if seen.get() == 2 {
                    return Err(Error::new(ErrorKind::Runtime, "second value refused"));
                }
                Ok(())
            })
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--zzm=1", "--zzm=2", "--zzm=3"]))
        .unwrap_err();
    assert!(
        error.to_string().contains("second value refused"),
        "{error}"
    );
    assert_eq!(seen.get(), 2);
    sane();
}

// ---- A4: flags ----

/// Runs an application with one flag `zzfl` and returns the exit code, the
/// value `validate_options` saw, the value `main` saw and the value after.
fn flag_run(initial: bool, arguments: &[&str]) -> (i32, Option<bool>, Option<bool>, bool) {
    let flag = Flag::new(initial);
    let in_validate = Cell::new(None);
    let in_main = Cell::new(None);
    let code = Application::new()
        .register_options(|o| o.add_flag("Test", "zzfl", "a flag", &flag))
        .validate_options(|| {
            in_validate.set(Some(flag.get()));
            Ok(())
        })
        .main(|_ctl, _files| {
            in_main.set(Some(flag.get()));
            Ok(())
        })
        .run(args(arguments))
        .unwrap();
    (code, in_validate.get(), in_main.get(), flag.get())
}

#[test]
fn a_flag_keeps_its_initial_value_and_follows_the_arguments() {
    let _lock = serial();
    let cases: [(bool, &[&str], bool); 6] = [
        (false, &[], false),
        (true, &[], true),
        (false, &["--zzfl"], true),
        (true, &["--zzfl"], true),
        (true, &["--no-zzfl"], false),
        // An empty value is accepted as "given" (upstream quirk, measured).
        (false, &["--zzfl="], true),
    ];
    for (initial, arguments, expected) in cases {
        let (code, in_validate, in_main, after) = flag_run(initial, arguments);
        let context = format!("initial {initial}, {arguments:?}");
        assert_eq!(code, 0, "{context}");
        assert_eq!(in_validate, Some(expected), "{context}");
        assert_eq!(in_main, Some(expected), "{context}");
        assert_eq!(after, expected, "{context}");
    }
}

#[test]
fn a_flag_used_wrongly_ends_in_exit_code_one_and_stays_unchanged() {
    let _lock = serial();
    let cases: [(bool, &[&str]); 6] = [
        (false, &["--zzfl=yes"]),
        (false, &["--zzfl=1"]),
        (false, &["--zzfl", "--zzfl"]),
        (false, &["--zzfl", "--no-zzfl"]),
        (true, &["--no-zzfl", "--no-zzfl"]),
        (true, &["--zzfl", "--no-zzfl"]),
    ];
    for (initial, arguments) in cases {
        let (code, in_validate, in_main, after) = flag_run(initial, arguments);
        let context = format!("initial {initial}, {arguments:?}");
        assert_eq!(code, 1, "{context}");
        assert_eq!(in_validate, None, "validate is not reached: {context}");
        assert_eq!(in_main, None, "{context}");
        // The value of a run that ended early is not published.
        assert_eq!(after, initial, "{context}");
    }
}

#[test]
fn aliases_of_flags_bundle() {
    let _lock = serial();
    let first = Flag::new(false);
    let second = Flag::new(false);
    let third = Flag::new(false);
    let code = Application::new()
        .register_options(|o| {
            o.add_flag("Test", "zzfa,y", "first", &first)?;
            o.add_flag("Test", "zzfb,z", "second", &second)?;
            o.add_flag("Test", "zzfc,@1", "third, hidden", &third)
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["-yz", "--zzfc"]))
        .unwrap();
    assert_eq!(code, 0);
    assert!(first.get() && second.get() && third.get());
    let code = Application::new()
        .register_options(|o| {
            o.add_flag("Test", "zzfa,y", "first", &first)?;
            o.add_flag("Test", "zzfb,z", "second", &second)
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--no-zzfa"]))
        .unwrap();
    assert_eq!(code, 0);
    assert!(!first.get() && second.get(), "flags are independent");
}

// ---- A5: the flag's lifetime and reuse ----

#[test]
fn a_flag_dropped_inside_register_options_is_still_published() {
    let _lock = serial();
    // The run keeps its own handle: the only one the caller had is gone before
    // clingo parses anything (ASan).
    let code = Application::new()
        .register_options(|o| {
            let flag = Flag::new(false);
            o.add_flag("Test", "zzfl", "a flag", &flag)?;
            drop(flag);
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--zzfl"]))
        .unwrap();
    assert_eq!(code, 0);

    // With a clone kept outside, the value is visible to validate and after.
    let outside = Flag::new(false);
    let in_validate = Cell::new(false);
    let code = Application::new()
        .register_options(|o| {
            let inner = outside.clone();
            o.add_flag("Test", "zzfl", "a flag", &inner)?;
            drop(inner);
            Ok(())
        })
        .validate_options(|| {
            in_validate.set(outside.get());
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--zzfl"]))
        .unwrap();
    assert_eq!(code, 0);
    assert!(in_validate.get());
    assert!(outside.get());
}

#[test]
fn the_same_flag_cannot_be_registered_twice_in_a_run() {
    let _lock = serial();
    let flag = Flag::new(false);
    let kinds = RefCell::new(Vec::new());
    let code = Application::new()
        .register_options(|o| {
            kinds
                .borrow_mut()
                .push(kind_of(o.add_flag("Test", "zzone", "one", &flag)));
            kinds
                .borrow_mut()
                .push(kind_of(o.add_flag("Test", "zztwo", "two", &flag)));
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--zzone"]))
        .unwrap();
    assert_eq!(code, 0, "the run goes on after a refused registration");
    assert_eq!(*kinds.borrow(), [None, Some(ErrorKind::InvalidInput)]);
    assert!(flag.get());
}

#[test]
fn a_flag_used_in_a_second_run_starts_from_the_first_result() {
    let _lock = serial();
    let flag = Flag::new(false);
    let seen = RefCell::new(Vec::new());
    let run = |arguments: &[&str]| {
        let code = Application::new()
            .register_options(|o| o.add_flag("Test", "zzfl", "a flag", &flag))
            .validate_options(|| {
                seen.borrow_mut().push(flag.get());
                Ok(())
            })
            .main(|_ctl, _files| Ok(()))
            .run(args(arguments))
            .unwrap();
        assert_eq!(code, 0);
    };
    run(&["--zzfl"]);
    run(&[]);
    run(&["--no-zzfl"]);
    run(&[]);
    assert_eq!(*seen.borrow(), [true, true, false, false]);
}

#[test]
fn two_flags_in_one_run_are_independent() {
    let _lock = serial();
    let (one, two) = (Flag::new(false), Flag::new(true));
    Application::new()
        .register_options(|o| {
            o.add_flag("Test", "zzone", "one", &one)?;
            o.add_flag("Test", "zztwo", "two", &two)
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--zzone", "--no-zztwo"]))
        .unwrap();
    assert!(one.get());
    assert!(!two.get());
}

#[test]
fn a_flag_is_a_shared_default_false_boolean() {
    fn assert_traits<T: Clone + Send + Sync + std::fmt::Debug + Default>() {}
    assert_traits::<Flag>();
    assert!(!Flag::default().get());
    assert!(Flag::new(true).get());
    assert!(!Flag::new(false).get());
    let flag = Flag::new(true);
    let copy = flag.clone();
    assert!(copy.get());
    assert!(format!("{flag:?}").contains("Flag"));
}

// ---- A6: validate_options ----

#[test]
fn a_failing_validate_returns_the_callbacks_own_error() {
    let _lock = serial();
    for kind in [ErrorKind::Runtime, ErrorKind::Logic, ErrorKind::Callback] {
        let main_ran = Cell::new(false);
        let error = Application::new()
            .validate_options(|| Err(Error::new(kind, "the options do not agree")))
            .main(|_ctl, _files| {
                main_ran.set(true);
                Ok(())
            })
            .run(args(&[]))
            .unwrap_err();
        assert_eq!(error.kind(), kind);
        assert!(
            error.to_string().contains("the options do not agree"),
            "{error}"
        );
        assert!(!main_ran.get(), "{kind:?}");
        sane();
    }
}

#[test]
fn validate_sees_what_parse_wrote_and_the_flags() {
    let _lock = serial();
    let value = RefCell::new(String::new());
    let flag = Flag::new(false);
    let saw = RefCell::new(None);
    let calls = Cell::new(0);
    let code = Application::new()
        .register_options(|o| {
            o.add(spec("zzval"), |text| {
                *value.borrow_mut() = text.to_owned();
                Ok(())
            })?;
            o.add_flag("Test", "zzfl", "a flag", &flag)
        })
        .validate_options(|| {
            calls.set(calls.get() + 1);
            *saw.borrow_mut() = Some((value.borrow().clone(), flag.get()));
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--zzval=seven", "--zzfl"]))
        .unwrap();
    assert_eq!(code, 0);
    assert_eq!(calls.get(), 1);
    assert_eq!(*saw.borrow(), Some(("seven".to_owned(), true)));
}

// ---- A7: register_options failing ----

#[test]
fn a_failing_register_options_returns_its_error_and_parses_nothing() {
    let _lock = serial();
    for arguments in [args(&["--zzopt=1"]), args(&["--help"])] {
        let log = Log::default();
        let error = Application::new()
            .register_options(|o| {
                o.add(spec("zzopt"), |value| {
                    log.borrow_mut().push(format!("parse {value}"));
                    Ok(())
                })?;
                Err(Error::new(ErrorKind::Runtime, "registration refused"))
            })
            .validate_options(|| {
                log.borrow_mut().push("validate".into());
                Ok(())
            })
            .main(|_ctl, _files| {
                log.borrow_mut().push("main".into());
                Ok(())
            })
            .run(arguments)
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Runtime);
        assert!(
            error.to_string().contains("registration refused"),
            "{error}"
        );
        assert!(logged(&log).is_empty(), "{:?}", logged(&log));
        sane();
    }
}

// ---- A8: duplicates and clashes ----

#[test]
fn the_wrapper_refuses_names_it_registered_before() {
    let _lock = serial();
    let flag = Flag::new(false);
    let other = Flag::new(false);
    let kinds = RefCell::new(Vec::new());
    let messages = RefCell::new(Vec::new());
    let log = Log::default();
    let code = Application::new()
        .register_options(|o| {
            let note = |result: Result<()>| {
                if let Err(error) = &result {
                    messages.borrow_mut().push(error.to_string());
                }
                kinds.borrow_mut().push(kind_of(result));
            };
            note(o.add(spec("zzdup"), |value| {
                log.borrow_mut().push(format!("first={value}"));
                Ok(())
            }));
            note(o.add(spec("zzdup"), |value| {
                log.borrow_mut().push(format!("second={value}"));
                Ok(())
            }));
            // Options and flags share one namespace of long names.
            note(o.add_flag("Test", "zzdup", "a flag", &flag));
            note(o.add_flag("Test", "zzown", "a flag", &flag));
            note(o.add(spec("zzown"), noop));
            // Aliases are a namespace of their own.
            note(o.add(spec("zzp,x"), noop));
            note(o.add(spec("zzq,x"), noop));
            note(o.add_flag("Test", "zzr,x", "a flag", &other));
            // An alias may equal another option's long name.
            note(o.add(spec("zzc,k"), noop));
            note(o.add(spec("k"), noop));
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--zzdup=1", "-k", "v"]))
        .unwrap();
    let dup = Some(ErrorKind::InvalidInput);
    assert_eq!(
        *kinds.borrow(),
        [None, dup, dup, None, dup, None, dup, dup, None, None]
    );
    assert!(messages.borrow()[0].contains("zzdup"), "{messages:?}");
    // The first registration still works and the run went on.
    assert_eq!(code, 0);
    assert_eq!(logged(&log), ["first=1"]);
}

#[test]
fn a_refused_registration_leaves_nothing_behind() {
    let _lock = serial();
    let log = Log::default();
    let kinds = RefCell::new(Vec::new());
    let code = Application::new()
        .register_options(|o| {
            // clingo rejects the key, so the name is still free afterwards.
            kinds
                .borrow_mut()
                .push(kind_of(o.add(spec("zzx,ll"), noop)));
            kinds.borrow_mut().push(kind_of(o.add(spec("zzx"), |value| {
                log.borrow_mut().push(format!("zzx={value}"));
                Ok(())
            })));
            // The same after a NUL and after a name the wrapper refuses.
            kinds.borrow_mut().push(kind_of(o.add(spec("zzy\0"), noop)));
            kinds.borrow_mut().push(kind_of(o.add(spec("zzy"), noop)));
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--zzx=1"]))
        .unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        *kinds.borrow(),
        [Some(ErrorKind::Logic), None, Some(ErrorKind::Nul), None]
    );
    assert_eq!(logged(&log), ["zzx=1"]);
}

#[test]
fn clashes_with_clingos_own_names_are_clingos_error_not_the_wrappers() {
    let _lock = serial();
    // `add` succeeds, and the run ends in exit code 1 at the start of parsing,
    // without a single option on the command line.
    for key in ["models", "zzt,t", "zzn,n", "zzh,h"] {
        // `-t` (`--parallel-mode`) exists only in a build with threads.
        if key == "zzt,t" && !clingox_sys::HAS_THREADS {
            eprintln!("SKIPPED: alias clash with -t: this build of clingo has no threads");
            continue;
        }
        let log = Log::default();
        let kinds = RefCell::new(Vec::new());
        let code = Application::new()
            .register_options(|o| {
                kinds.borrow_mut().push(kind_of(o.add(spec(key), noop)));
                Ok(())
            })
            .validate_options(|| {
                log.borrow_mut().push("validate".into());
                Ok(())
            })
            .main(|_ctl, _files| {
                log.borrow_mut().push("main".into());
                Ok(())
            })
            .run(args(&[]))
            .unwrap();
        assert_eq!(*kinds.borrow(), [None], "{key}");
        assert_eq!(code, 1, "{key}");
        assert!(logged(&log).is_empty(), "{key}: {:?}", logged(&log));
    }
    // A flag named like a clingo option is the same.
    let flag = Flag::new(false);
    let code = Application::new()
        .register_options(|o| o.add_flag("Test", "quiet", "a flag", &flag))
        .main(|_ctl, _files| Ok(()))
        .run(args(&[]))
        .unwrap();
    assert_eq!(code, 1);
}

// ---- A9: the key grammar and errors of `add` ----

#[test]
fn keys_clingo_accepts_can_be_used() {
    let _lock = serial();
    let log = Log::default();
    let kinds = RefCell::new(Vec::new());
    let code = Application::new()
        .register_options(|o| {
            for key in [
                "zzk1",
                "zzk2,a",
                "zzk3,@1",
                "zzk4,b,@2",
                "zzk5,@0",
                "é",
                "1x",
                "no-x",
                "Foo",
            ] {
                let label = key.split(',').next().unwrap().to_owned();
                let log = &log;
                kinds
                    .borrow_mut()
                    .push(kind_of(o.add(spec(key), move |value| {
                        log.borrow_mut().push(format!("{label}={value}"));
                        Ok(())
                    })));
            }
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&[
            "--zzk1=1", "--zzk2=2", "--zzk3=4", "-b", "5", "--zzk5=6", "--é=7", "--1x=8",
            "--no-x=9", "--Foo=10",
        ]))
        .unwrap();
    assert_eq!(code, 0);
    assert!(kinds.borrow().iter().all(Option::is_none), "{kinds:?}");
    assert_eq!(
        logged(&log),
        [
            "zzk1=1", "zzk2=2", "zzk3=4", "zzk4=5", "zzk5=6", "é=7", "1x=8", "no-x=9", "Foo=10"
        ]
    );
}

#[test]
fn keys_are_case_sensitive() {
    let _lock = serial();
    let log = Log::default();
    let code = probe(&log, &[("Foo", false)])
        .run(args(&["--foo=1"]))
        .unwrap();
    assert_eq!(code, 1, "unknown option: 'foo'");
    assert_eq!(logged(&log), ["register"]);
}

#[test]
fn keys_clingo_rejects_are_logic_errors_with_its_message() {
    let _lock = serial();
    let results = RefCell::new(Vec::new());
    let code = Application::new()
        .register_options(|o| {
            for key in [
                "zzr,",
                "zzr,ll",
                "zzr,@9",
                "zzr,@a",
                "zzr,@1,y",
                "a,b,c",
                "zzr,@1,@2",
                "",
                ",l",
            ] {
                results
                    .borrow_mut()
                    .push((key, o.add(spec(key), noop).unwrap_err()));
            }
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&[]))
        .unwrap();
    assert_eq!(code, 0, "nothing was registered, so the run is unaffected");
    for (key, error) in results.borrow().iter() {
        assert_eq!(error.kind(), ErrorKind::Logic, "{key:?}");
        let text = error.to_string();
        let expected = if key.is_empty() || key.starts_with(',') {
            "Invalid empty option name"
        } else {
            "Invalid Key"
        };
        assert!(text.contains(expected), "{key:?}: {text}");
    }
}

#[test]
fn names_that_can_never_be_typed_are_refused_by_the_wrapper() {
    let _lock = serial();
    let kinds = RefCell::new(Vec::new());
    Application::new()
        .register_options(|o| {
            for key in ["--x", "-y", "a=b", "zz=", "--x,q"] {
                kinds.borrow_mut().push(kind_of(o.add(spec(key), noop)));
            }
            kinds.borrow_mut().push(kind_of(o.add_flag(
                "Test",
                "--f",
                "a flag",
                &Flag::new(false),
            )));
            kinds.borrow_mut().push(kind_of(o.add_flag(
                "Test",
                "f=g",
                "a flag",
                &Flag::new(false),
            )));
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&[]))
        .unwrap();
    assert!(
        kinds
            .borrow()
            .iter()
            .all(|kind| *kind == Some(ErrorKind::InvalidInput)),
        "{kinds:?}"
    );
}

#[test]
fn a_nul_in_any_string_is_a_nul_error() {
    let _lock = serial();
    let kinds = RefCell::new(Vec::new());
    let flag = Flag::new(false);
    Application::new()
        .register_options(|o| {
            let mut add = |group: &str, key: &str, description: &str, argument: Option<&str>| {
                let mut option = OptionSpec::new(group, key, description);
                if let Some(argument) = argument {
                    option = option.argument(argument);
                }
                kinds.borrow_mut().push(kind_of(o.add(option, noop)));
            };
            add("G\0", "zzn1", "d", None);
            add("G", "zzn2\0", "d", None);
            add("G", "zzn3", "d\0", None);
            add("G", "zzn4", "d", Some("<a\0>"));
            let mut add_flag = |group: &str, key: &str, description: &str| {
                kinds
                    .borrow_mut()
                    .push(kind_of(o.add_flag(group, key, description, &flag)));
            };
            add_flag("G\0", "zzn5", "d");
            add_flag("G", "zzn6\0", "d");
            add_flag("G", "zzn7", "d\0");
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&[]))
        .unwrap();
    assert_eq!(*kinds.borrow(), [Some(ErrorKind::Nul); 7]);
}

#[test]
fn an_empty_group_description_and_argument_are_accepted() {
    let _lock = serial();
    let log = Log::default();
    let code = Application::new()
        .register_options(|o| {
            o.add(OptionSpec::new("", "zzeg", ""), |value| {
                log.borrow_mut().push(format!("eg={value}"));
                Ok(())
            })?;
            o.add(OptionSpec::new("G", "zzea", "d").argument(""), |value| {
                log.borrow_mut().push(format!("ea={value}"));
                Ok(())
            })
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--zzeg=1", "--zzea=2"]))
        .unwrap();
    assert_eq!(code, 0);
    assert_eq!(logged(&log), ["eg=1", "ea=2"]);
}

// ---- A10: what runs when ----

#[test]
fn the_callbacks_run_in_order_register_parse_validate_main() {
    let _lock = serial();
    let log = Log::default();
    let code = probe(&log, &[("zzo", false)])
        .run(args(&["0", "x.lp", "--zzo=1"]))
        .unwrap();
    assert_eq!(code, 0);
    // `main` receives the one file, not the number or the options. (With two
    // missing files clingo ends with exit code 128 before `validate`: it opens
    // the last one itself.)
    assert_eq!(logged(&log), ["register", "zzo=1", "validate", "main 1"]);
}

#[test]
fn a_missing_file_still_reaches_validate_and_main() {
    let _lock = serial();
    let log = Log::default();
    let code = probe(&log, &[("zzo", false)])
        .run(args(&["/nonexistent/clingox/none.lp"]))
        .unwrap();
    assert_eq!(code, 0);
    assert_eq!(logged(&log), ["register", "validate", "main 1"]);
}

#[test]
fn an_unknown_option_or_a_failed_parse_skips_validate_and_main() {
    let _lock = serial();
    let log = Log::default();
    let code = probe(&log, &[("zzo", false)])
        .run(args(&["--zznosuch"]))
        .unwrap();
    assert_eq!(code, 1);
    assert_eq!(logged(&log), ["register"], "register runs once, first");

    let log = Log::default();
    let code = probe(&log, &[("zzo", false)])
        .run(args(&["--zzo=1", "--zznosuch"]))
        .unwrap();
    assert_eq!(code, 1);
    // clingo checks the whole command line before it delivers any value.
    assert_eq!(logged(&log), ["register"]);
}

// ---- A11: values ----

fn value_arrives(key: &str, arguments: &[&str], expected: &str) {
    let log = Log::default();
    let label = key.split(',').next().unwrap();
    let code = probe(&log, &[(key, false)]).run(args(arguments)).unwrap();
    assert_eq!(code, 0, "{arguments:?}");
    assert_eq!(
        logged(&log),
        [
            "register".to_owned(),
            format!("{label}={expected}"),
            "validate".to_owned(),
            "main 0".to_owned()
        ],
        "{arguments:?}"
    );
}

#[test]
fn values_arrive_exactly_as_typed() {
    let _lock = serial();
    let long = "x".repeat(100_000);
    let long_argument = format!("--zzv={long}");
    let cases: [(&str, &str); 7] = [
        ("--zzv=héllo ✓", "héllo ✓"),
        ("--zzv=a=b", "a=b"),
        ("--zzv=a b  c", "a b  c"),
        ("--zzv=-5", "-5"),
        ("--zzv=--zzv", "--zzv"),
        ("--zzv=%s%d", "%s%d"),
        (&long_argument, &long),
    ];
    for (argument, expected) in cases {
        value_arrives("zzv", &[argument], expected);
    }
    // A value in the next argument may start with a dash.
    value_arrives("zzv", &["--zzv", "-5"], "-5");
    value_arrives("zzv", &["--zzv", "--foo"], "--foo");
}

#[test]
fn an_alias_takes_its_value_in_three_forms() {
    let _lock = serial();
    value_arrives("zzp,p", &["-p", "2"], "2");
    value_arrives("zzp,p", &["-p3"], "3");
    // clasp's own quirk: the `=` belongs to the value after a short alias.
    value_arrives("zzp,p", &["-p=3"], "=3");
    value_arrives("zzp,p", &["--zzp=4"], "4");
}

#[test]
fn an_option_without_a_value_is_a_command_line_error() {
    let _lock = serial();
    for argument in ["--zzv", "--zzv="] {
        let log = Log::default();
        let code = probe(&log, &[("zzv", false)])
            .run(args(&[argument]))
            .unwrap();
        assert_eq!(code, 1, "{argument}");
        assert_eq!(
            logged(&log),
            ["register"],
            "{argument}: parse is not called"
        );
    }
}

#[test]
fn a_unique_prefix_selects_the_option_and_an_ambiguous_one_fails() {
    let _lock = serial();
    value_arrives("zzzzlong", &["--zzz=2"], "2");
    let log = Log::default();
    let code = probe(&log, &[("zzzab", false), ("zzzac", false)])
        .run(args(&["--zzza=1"]))
        .unwrap();
    assert_eq!(code, 1, "ambiguous option");
    assert_eq!(logged(&log), ["register"]);
}

// ---- A12: panics ----

fn payload(result: std::thread::Result<Result<i32>>) -> String {
    let panic = result.expect_err("the panic must reach the caller of run");
    panic
        .downcast_ref::<&str>()
        .map(ToString::to_string)
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap()
}

#[test]
fn a_panic_in_a_callback_is_resumed_and_nothing_after_it_runs() {
    let _lock = serial();
    // register_options
    let log = Log::default();
    let result = catch_unwind(AssertUnwindSafe(|| {
        Application::new()
            .register_options(|_o| {
                log.borrow_mut().push("register".into());
                panic!("boom in register");
            })
            .validate_options(|| {
                log.borrow_mut().push("validate".into());
                Ok(())
            })
            .run(args(&[]))
    }));
    assert_eq!(payload(result), "boom in register");
    assert_eq!(logged(&log), ["register"]);
    sane();

    // parse
    let log = Log::default();
    let result = catch_unwind(AssertUnwindSafe(|| {
        Application::new()
            .register_options(|o| {
                o.add(spec("zzo"), |_| panic!("boom in parse"))?;
                Ok(())
            })
            .validate_options(|| {
                log.borrow_mut().push("validate".into());
                Ok(())
            })
            .main(|_ctl, _files| {
                log.borrow_mut().push("main".into());
                Ok(())
            })
            .run(args(&["--zzo=1"]))
    }));
    assert_eq!(payload(result), "boom in parse");
    assert!(logged(&log).is_empty(), "{:?}", logged(&log));
    sane();

    // validate_options
    let log = Log::default();
    let result = catch_unwind(AssertUnwindSafe(|| {
        Application::new()
            .validate_options(|| panic!("boom in validate"))
            .main(|_ctl, _files| {
                log.borrow_mut().push("main".into());
                Ok(())
            })
            .run(args(&[]))
    }));
    assert_eq!(payload(result), "boom in validate");
    assert!(logged(&log).is_empty(), "{:?}", logged(&log));
    sane();
}

// ---- A13: many options ----

#[test]
fn five_hundred_options_and_two_hundred_flags_in_one_run() {
    let _lock = serial();
    let hits: RefCell<Vec<(usize, String)>> = RefCell::new(Vec::new());
    let flags: Vec<Flag> = (0..200).map(|_| Flag::new(false)).collect();
    let after_parse = RefCell::new(Vec::new());
    let code = Application::new()
        .register_options(|o| {
            for i in 0..500 {
                let name = format!("zzopt{i}");
                let hits = &hits;
                o.add(OptionSpec::new("Many", &name, "an option"), move |value| {
                    hits.borrow_mut().push((i, value.to_owned()));
                    Ok(())
                })?;
            }
            for (i, flag) in flags.iter().enumerate() {
                o.add_flag("Many", &format!("zzflag{i}"), "a flag", flag)?;
            }
            Ok(())
        })
        .validate_options(|| {
            after_parse
                .borrow_mut()
                .extend([0_usize, 5, 199].map(|i| flags[i].get()));
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&[
            "--zzopt499=last",
            "--zzflag0",
            "--zzopt0=first",
            "--zzflag199",
            "--zzopt250=middle",
        ]))
        .unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        *hits.borrow(),
        [
            (499, "last".to_owned()),
            (0, "first".to_owned()),
            (250, "middle".to_owned())
        ]
    );
    assert_eq!(*after_parse.borrow(), [true, false, true]);
    assert!(flags[0].get() && flags[199].get() && !flags[5].get());
}

// ---- A14: nested runs ----

#[test]
fn a_run_started_from_an_options_callback_is_refused() {
    let _lock = serial();
    let inner = RefCell::new(Vec::new());
    let attempt = |from: &'static str| {
        let result = Application::new()
            .main(|_ctl, _files| Ok(()))
            .run(args(&[]));
        let error = result.unwrap_err();
        inner
            .borrow_mut()
            .push((from, error.kind(), error.to_string()));
    };
    let attempt = &attempt;
    let code = Application::new()
        .register_options(move |o| {
            attempt("register");
            o.add(spec("zzo"), move |_| {
                attempt("parse");
                Ok(())
            })
        })
        .validate_options(move || {
            attempt("validate");
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&["--zzo=1"]))
        .unwrap();
    assert_eq!(code, 0, "the outer run completes");
    let inner = inner.borrow();
    let from: Vec<&str> = inner.iter().map(|(from, _, _)| *from).collect();
    assert_eq!(from, ["register", "parse", "validate"]);
    for (from, kind, text) in inner.iter() {
        assert_eq!(*kind, ErrorKind::InvalidInput, "{from}");
        assert!(text.contains("in progress"), "{from}: {text}");
    }
}

// ---- A15: examples/c/application.c ----

fn example_run(arguments: &[&str]) -> (i32, Vec<Vec<String>>, String) {
    let file = child::fixture("opts_parts.lp", PARTS);
    let path = child::arg(&file);
    let program = RefCell::new(None::<String>);
    let models: RefCell<Vec<Vec<String>>> = RefCell::new(Vec::new());
    let mut all = vec![path.as_str(), "--outf=3"];
    all.extend_from_slice(arguments);
    let code = Application::new()
        .program_name("example")
        .version("1.0.0")
        .register_options(|o| {
            o.add(
                OptionSpec::new(
                    "Example",
                    "program",
                    "Override the default program part to ground",
                )
                .argument("<prog>"),
                |value| {
                    *program.borrow_mut() = Some(value.to_owned());
                    Ok(())
                },
            )
        })
        .main(|ctl, files| {
            for file in files {
                ctl.load(file)?;
            }
            if files.is_empty() {
                ctl.load("-")?;
            }
            let name = program
                .borrow()
                .clone()
                .unwrap_or_else(|| "base".to_owned());
            ctl.ground(&[Part::new(&name, &[])?])?;
            let _ = ctl.for_each_model(&[], |model| {
                let mut symbols: Vec<String> = model
                    .symbols(ShowType::SHOWN)?
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                symbols.sort();
                models.borrow_mut().push(symbols);
                Ok(ControlFlow::Continue(()))
            })?;
            Ok(())
        })
        .run(all)
        .unwrap();
    let name = program.into_inner().unwrap_or_default();
    let mut models = models.into_inner();
    models.sort();
    (code, models, name)
}

#[test]
fn the_c_example_selects_the_program_part_with_an_option() {
    let _lock = serial();
    // Default: the base part, one model, `SATISFIABLE`.
    assert_eq!(
        example_run(&[]),
        (10, vec![vec!["b".to_owned()]], String::new())
    );
    // `--program=foo` with every model: `f` and `f g`.
    let (code, models, name) = example_run(&["--program=foo", "0"]);
    assert_eq!(code, 30);
    assert_eq!(name, "foo");
    assert_eq!(
        models,
        [vec!["f".to_owned()], vec!["f".to_owned(), "g".to_owned()]]
    );
    // The value may be a separate argument, and `base` is the default again.
    let (code, models, name) = example_run(&["--program", "base"]);
    assert_eq!((code, name.as_str()), (10, "base"));
    assert_eq!(models, [vec!["b".to_owned()]]);
}

// ---- A16: Debug ----

#[test]
fn debug_names_the_option_callbacks_that_are_set() {
    let plain = format!("{:?}", Application::new());
    assert!(plain.contains("register_options: false"), "{plain}");
    assert!(plain.contains("validate_options: false"), "{plain}");
    // The handle type is exported next to `Application`.
    assert!(None::<&Options<'static>>.is_none());
    let register = format!("{:?}", Application::new().register_options(|_o| Ok(())));
    assert!(register.contains("register_options: true"), "{register}");
    assert!(register.contains("validate_options: false"), "{register}");
    let both = format!(
        "{:?}",
        Application::new()
            .register_options(|_o| Ok(()))
            .validate_options(|| Ok(()))
    );
    assert!(both.contains("register_options: true"), "{both}");
    assert!(both.contains("validate_options: true"), "{both}");
}

// ---- A17: borrowing and sequential runs ----

#[test]
fn callbacks_borrow_the_callers_state_and_the_state_is_usable_afterwards() {
    let _lock = serial();
    let seen = RefCell::new(Vec::<String>::new());
    let count = Cell::new(0_u32);
    for round in 0..2 {
        let argument = format!("--zzo=round{round}");
        Application::new()
            .register_options(|o| {
                o.add(spec("zzo"), |value| {
                    seen.borrow_mut().push(value.to_owned());
                    Ok(())
                })
            })
            .validate_options(|| {
                count.set(count.get() + 1);
                Ok(())
            })
            .main(|_ctl, _files| Ok(()))
            .run(args(&[argument.as_str()]))
            .unwrap();
    }
    assert_eq!(*seen.borrow(), ["round0", "round1"]);
    assert_eq!(count.get(), 2);
}

// ---- A18: setters and the default main ----

#[test]
fn the_last_setter_wins() {
    let _lock = serial();
    let log = Log::default();
    Application::new()
        .register_options(|_o| {
            log.borrow_mut().push("first register".into());
            Ok(())
        })
        .register_options(|_o| {
            log.borrow_mut().push("second register".into());
            Ok(())
        })
        .validate_options(|| {
            log.borrow_mut().push("first validate".into());
            Ok(())
        })
        .validate_options(|| {
            log.borrow_mut().push("second validate".into());
            Ok(())
        })
        .main(|_ctl, _files| Ok(()))
        .run(args(&[]))
        .unwrap();
    assert_eq!(logged(&log), ["second register", "second validate"]);
}

#[test]
fn options_work_with_clingos_default_main_and_the_other_setters() {
    let _lock = serial();
    let file = child::fixture("opts_default_main.lp", SAT);
    let path = child::arg(&file);
    let log = Log::default();
    let messages = Mutex::new(0_usize);
    let code = Application::new()
        .program_name("prog")
        .version("9.9")
        .message_limit(5)
        .logger(|_code, _text| *messages.lock().unwrap() += 1)
        .register_options(|o| {
            o.add(spec("zzo"), |value| {
                log.borrow_mut().push(format!("parse {value}"));
                Ok(())
            })
        })
        .validate_options(|| {
            log.borrow_mut().push("validate".into());
            Ok(())
        })
        .run([path.as_str(), "0", "--outf=3", "--zzo=1"])
        .unwrap();
    // Two models and an exhausted search, solved by clingo's own main.
    assert_eq!(code, 30);
    assert_eq!(logged(&log), ["parse 1", "validate"]);
    assert_eq!(*messages.lock().unwrap(), 0);
}

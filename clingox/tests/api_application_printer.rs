//! `Application::print_model` and `DefaultPrinter`, in process.
//!
//! Expected values come from pyclingo 5.8.2 (clingo 5.8.2, clasp 3.4.1)
//! calling `clingo_main` at the C level with a `printer` callback. A failing
//! printer never returns false to clingo; the error is stored, the closure and
//! the default printer are skipped for every later model, the search is
//! interrupted, and `run` returns the stored error.
//!
//! The printer only runs for the default text output, so `--outf=3` (which the
//! earlier application tests use) cannot be used here. Every run passes
//! `--verbose=0`, and a printer that does not call `print()` writes no model
//! text; the one `SATISFIABLE` line per run that remains goes to C standard
//! output, which libtest cannot capture. Tests that assert output are children
//! (`application_printer_child.rs`). No test spawns a thread or passes `-t N`.
//!
//! `run` refuses a second run while one is in progress, so the tests of this
//! file take one lock and never overlap.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test helpers fail loudly; the watchdog explains its abort"
)]

#[path = "common/child.rs"]
mod child;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use clingox::application::{Application, DefaultPrinter};
use clingox::{
    Error, ErrorKind, Model, ModelKind, Part, Result, ScopedControl, ShowType, SolveResult, Symbol,
};

const X3: &str = "{x(1..3)}. #show x/1.";
/// 2^30 models: enumerating them all takes far longer than any test, so an
/// interrupt from the printer always lands before the search can finish on
/// its own, however loaded the machine is.
const X30: &str = "{x(1..30)}. #show x/1.";
const X2: &str = "{x(1..2)}. y. #show x/1.";
const AB: &str = "{a;b}. #show a/0. #show b/0.";
const OPT: &str = "{a;b;c}. #minimize{1,a:a;1,b:b;1,c:c}. #minimize{2@2,a:a}. :- not a, not b.";
const ALL: [&str; 2] = ["--verbose=0", "0"];
const LIMIT: Duration = Duration::from_secs(30);

static SERIAL: Mutex<()> = Mutex::new(());

/// Aborts the test process if a test runs longer than `LIMIT`. Without
/// threads (WebAssembly) there is no watchdog.
struct Watchdog(Option<std::sync::mpsc::Sender<()>>);

impl Watchdog {
    fn start() -> Watchdog {
        if !clingox_sys::HAS_THREADS {
            return Watchdog(None);
        }
        let (done, wait) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if wait.recv_timeout(LIMIT) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                eprintln!("the test hung for {LIMIT:?}: run did not return; aborting the process");
                std::process::abort();
            }
        });
        Watchdog(Some(done))
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Some(done) = self.0.take() {
            let _ = done.send(());
        }
    }
}

fn serial() -> (MutexGuard<'static, ()>, Watchdog) {
    let lock = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    (lock, Watchdog::start())
}

fn solve(ctl: &mut ScopedControl<'_>, program: &str) -> Result<SolveResult> {
    ctl.add_base(program)?;
    ctl.ground(&[Part::base()])?;
    ctl.solve(&[])
}

/// An application whose `main` grounds `program` and solves once.
fn app<'a, F>(program: &'a str, printer: F) -> Application<'a>
where
    F: for<'p> Fn(&Model, &mut DefaultPrinter<'p>) -> Result<()> + Send + Sync + 'a,
{
    Application::new()
        .main(move |ctl, _files| solve(ctl, program).map(|_| ()))
        .print_model(printer)
}

/// What one printer call saw of its model.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    number: u64,
    shown: Vec<String>,
    atoms: Vec<String>,
    cost: Vec<i64>,
    priorities: Vec<i32>,
    optimal: bool,
    kind: ModelKind,
    thread: u32,
}

fn strings(symbols: &[Symbol]) -> Vec<String> {
    let mut out: Vec<String> = symbols.iter().map(ToString::to_string).collect();
    out.sort();
    out
}

fn look(model: &Model) -> Result<Seen> {
    Ok(Seen {
        number: model.number(),
        shown: strings(&model.symbols(ShowType::SHOWN)?),
        atoms: strings(&model.symbols(ShowType::ATOMS)?),
        cost: model.cost()?,
        priorities: model.priorities()?,
        optimal: model.optimality_proven()?,
        kind: model.kind()?,
        thread: model.thread_id(),
    })
}

fn set(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// Runs `program` with `args` and a recording printer that does not print.
fn record(program: &str, args: &[&str]) -> (Result<i32>, Vec<Seen>) {
    let seen = Mutex::new(Vec::new());
    let result = app(program, |model, _printer| {
        seen.lock().unwrap().push(look(model)?);
        Ok(())
    })
    .run(args.iter().copied());
    (result, seen.into_inner().unwrap())
}

// ---- refusal without main ----

#[test]
fn p1_print_model_without_main_is_refused_before_anything_runs() {
    let _guard = serial();
    let printer_calls = AtomicUsize::new(0);
    let logger_calls = AtomicUsize::new(0);
    let register_calls = AtomicUsize::new(0);
    let validate_calls = AtomicUsize::new(0);
    let result = Application::new()
        .logger(|_, _| {
            logger_calls.fetch_add(1, Ordering::SeqCst);
        })
        .register_options(|_options| {
            register_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .validate_options(|| {
            validate_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        // The closure never reads the model: a mutation that lets the run go
        // on must fail this test by `Ok`, not by a crash inside clingo (U40).
        .print_model(|_model, _printer| {
            printer_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .run(["--verbose=0"]);
    let error = result.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    let text = error.to_string();
    assert!(
        text.contains("print_model") && text.contains("main"),
        "{text}"
    );
    for (name, count) in [
        ("printer", &printer_calls),
        ("logger", &logger_calls),
        ("register_options", &register_calls),
        ("validate_options", &validate_calls),
    ] {
        assert_eq!(count.load(Ordering::SeqCst), 0, "{name} must not run");
    }
}

#[test]
fn p2_the_refusal_applies_to_help_version_and_bad_options_too() {
    let _guard = serial();
    for argument in ["--help", "--version", "--nosuch"] {
        let error = Application::new()
            .print_model(|_model, _printer| Ok(()))
            .run([argument])
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{argument}");
    }
}

#[test]
fn p3_setters_commute_and_the_last_print_model_wins() {
    let _guard = serial();
    let first = AtomicUsize::new(0);
    let second = AtomicUsize::new(0);
    // Four models of `X2`; `print_model` before `main`.
    let code = Application::new()
        .print_model(|_model, _printer| {
            first.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .print_model(|_model, _printer| {
            second.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .main(|ctl, _files| solve(ctl, X2).map(|_| ()))
        .run(ALL)
        .unwrap();
    assert_eq!(code, 30);
    assert_eq!(first.load(Ordering::SeqCst), 0);
    assert_eq!(second.load(Ordering::SeqCst), 4);
    let (code, seen) = record(X2, &ALL);
    assert_eq!((code.unwrap(), seen.len()), (30, 4));
}

// ---- what the printer sees ----

#[test]
fn p4_the_model_reads_match_the_oracle() {
    let _guard = serial();
    let (result, seen) = record(X3, &ALL);
    assert_eq!(result.unwrap(), 30);
    // pyclingo 5.8.2, `{x(1..3)}. #show x/1.` with `--models=0`, one thread:
    // eight models, numbered 1 to 8, each seen once.
    let expected: [&[&str]; 8] = [
        &[],
        &["x(2)"],
        &["x(3)"],
        &["x(2)", "x(3)"],
        &["x(1)"],
        &["x(1)", "x(3)"],
        &["x(1)", "x(2)"],
        &["x(1)", "x(2)", "x(3)"],
    ];
    let mut by_number = seen.clone();
    by_number.sort_by_key(|s| s.number);
    assert_eq!(
        by_number.iter().map(|s| s.number).collect::<Vec<_>>(),
        (1..=8).collect::<Vec<_>>()
    );
    let mut got: Vec<Vec<String>> = by_number.iter().map(|s| s.shown.clone()).collect();
    let mut want: Vec<Vec<String>> = expected.iter().map(|m| set(m)).collect();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    for one in &seen {
        assert_eq!(one.atoms, one.shown, "no hidden atom in this program");
        assert!(one.cost.is_empty() && one.priorities.is_empty());
        assert!(!one.optimal, "no objective: nothing is proven optimal");
        assert_eq!(one.kind, ModelKind::StableModel);
        assert_eq!(one.thread, 0, "one thread");
    }
}

#[test]
fn p5_measured_order_on_this_build() {
    let _guard = serial();
    // The order is clasp's on this build (the drafter's `e22`), not a
    // contract: this is the only test that pins it.
    let (_, seen) = record(X3, &ALL);
    let order: Vec<Vec<String>> = seen.iter().map(|s| s.shown.clone()).collect();
    let expected: [&[&str]; 8] = [
        &[],
        &["x(2)"],
        &["x(3)"],
        &["x(2)", "x(3)"],
        &["x(1)"],
        &["x(1)", "x(3)"],
        &["x(1)", "x(2)"],
        &["x(1)", "x(2)", "x(3)"],
    ];
    assert_eq!(
        order,
        expected.iter().map(|m| set(m)).collect::<Vec<_>>(),
        "numbers in call order: {:?}",
        seen.iter().map(|s| s.number).collect::<Vec<_>>()
    );
}

#[test]
fn p5b_the_other_reads_agree_inside_the_printer() {
    let _guard = serial();
    let x1 = Symbol::function("x", &[Symbol::number(1)]).unwrap();
    let agreed = AtomicUsize::new(0);
    let result = app(X3, |model, _printer| {
        let shown = model.symbols(ShowType::SHOWN)?;
        assert_eq!(model.contains(x1)?, shown.contains(&x1));
        let snapshot = model.snapshot()?;
        assert_eq!(snapshot.number(), model.number());
        assert_eq!(strings(snapshot.symbols()), strings(&shown));
        agreed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .run(ALL);
    assert_eq!(result.unwrap(), 30);
    assert_eq!(agreed.load(Ordering::SeqCst), 8);
}

#[test]
fn p6_costs_priorities_and_the_second_call_under_optn() {
    let _guard = serial();
    // Default `--opt-mode=opt`: one call, the optimum not yet proven.
    let (result, seen) = record(OPT, &ALL);
    assert_eq!(result.unwrap(), 30);
    assert_eq!(seen.len(), 1);
    let only = &seen[0];
    assert_eq!(only.number, 1);
    assert_eq!(only.cost, [0, 1]);
    assert_eq!(only.priorities, [2, 0]);
    assert!(!only.optimal);
    assert_eq!(only.shown, set(&["b"]));
    // `--opt-mode=optN`: the printer runs twice for the optimum, with the
    // same number, unproven and then proven.
    let (result, seen) = record(OPT, &["--verbose=0", "0", "--opt-mode=optN"]);
    assert_eq!(result.unwrap(), 30);
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].number, 1);
    assert_eq!(seen[1].number, 1);
    assert_eq!(seen[0].cost, [0, 1]);
    assert_eq!(seen[1].cost, [0, 1]);
    assert!(!seen[0].optimal);
    assert!(seen[1].optimal);
}

#[test]
fn p7_brave_and_cautious_enumeration_report_their_kind() {
    let _guard = serial();
    // pyclingo 5.8.2: brave gives three running unions, cautious one
    // intersection (shown `[]`).
    let (_, seen) = record(X2, &["--verbose=0", "0", "--enum-mode=brave"]);
    assert_eq!(
        seen.iter().map(|s| (s.number, s.kind)).collect::<Vec<_>>(),
        [
            (1, ModelKind::BraveConsequences),
            (2, ModelKind::BraveConsequences),
            (3, ModelKind::BraveConsequences)
        ]
    );
    assert_eq!(
        seen.iter().map(|s| s.shown.clone()).collect::<Vec<_>>(),
        [set(&[]), set(&["x(2)"]), set(&["x(1)", "x(2)"])]
    );
    let (_, seen) = record(X2, &["--verbose=0", "0", "--enum-mode=cautious"]);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].kind, ModelKind::CautiousConsequences);
    assert_eq!(seen[0].shown, set(&[]));
}

#[test]
fn p8_add_clause_from_the_printer_narrows_the_enumeration() {
    let _guard = serial();
    let a = Symbol::function("a", &[]).unwrap();
    let seen = Mutex::new(Vec::new());
    let result = app(AB, |model, _printer| {
        seen.lock()
            .unwrap()
            .push(strings(&model.symbols(ShowType::SHOWN)?));
        if model.number() == 1 {
            let context = model.context();
            let atoms = context.symbolic_atoms()?;
            let literal = atoms.find(a)?.expect("a is an atom").literal();
            context.add_clause(&[literal])?;
        }
        Ok(())
    })
    .run(ALL);
    assert_eq!(result.unwrap(), 30);
    // pyclingo 5.8.2: after the clause `[a]` on the first (empty) model only
    // the models with `a` follow; `{b}` is never delivered.
    assert_eq!(
        seen.into_inner().unwrap(),
        [set(&[]), set(&["a"]), set(&["a", "b"])]
    );
}

// ---- when the printer runs ----

#[test]
fn p9_the_printer_is_not_called_where_clingo_prints_no_text() {
    let _guard = serial();
    // (arguments, exit code), pyclingo 5.8.2: no call at all.
    let silent: [(&[&str], i32); 6] = [
        (&["-q"], 30),
        (&["--quiet=2"], 30),
        (&["--outf=2"], 30),
        (&["--outf=3"], 30),
        (&["--mode=gringo"], 0),
        (&["--text"], 0),
    ];
    for (extra, code) in silent {
        let mut args = vec!["--verbose=0", "0"];
        args.extend_from_slice(extra);
        let (result, seen) = record(X2, &args);
        assert_eq!(result.unwrap(), code, "{extra:?}");
        assert!(seen.is_empty(), "{extra:?}: {seen:?}");
    }
}

#[test]
fn p9b_the_last_model_only_and_no_call_for_unsatisfiable() {
    let _guard = serial();
    for extra in ["--quiet=1", "--outf=1"] {
        let (result, seen) = record(X2, &["--verbose=0", "0", extra]);
        assert_eq!(result.unwrap(), 30, "{extra}");
        assert_eq!(seen.len(), 1, "{extra}");
        assert_eq!(seen[0].number, 4, "{extra}");
        assert_eq!(seen[0].shown, set(&["x(1)", "x(2)"]), "{extra}");
    }
    let (result, seen) = record("a. :- a.", &ALL);
    assert_eq!(result.unwrap(), 20);
    assert!(seen.is_empty());
}

// ---- failure ----

/// A printer that fails on its second call; `main` keeps what its solve call
/// returned.
fn failing_run(
    error: fn() -> Error,
    main_after: fn(Result<SolveResult>) -> Result<()>,
) -> (Result<i32>, usize) {
    let calls = AtomicUsize::new(0);
    let result = Application::new()
        .main(move |ctl, _files| main_after(solve(ctl, X3)))
        .print_model(|_model, _printer| {
            if calls.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
                Err(error())
            } else {
                Ok(())
            }
        })
        .run(ALL);
    (result, calls.load(Ordering::SeqCst))
}

fn printer_failed() -> Error {
    Error::new(ErrorKind::Runtime, "printer failed")
}

/// Runs the failing printer of P10 and returns what `main`'s solve call
/// reported, the closure's call count and `run`'s result.
fn failing_second(program: &str) -> (Option<Result<bool>>, usize, Result<i32>) {
    let interrupted = Mutex::new(None);
    let calls = AtomicUsize::new(0);
    let result = Application::new()
        .main(|ctl, _files| {
            let solved = solve(ctl, program);
            *interrupted.lock().unwrap() = Some(solved.map(|r| r.is_interrupted()));
            Ok(())
        })
        .print_model(|_model, _printer| {
            if calls.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
                Err(printer_failed())
            } else {
                Ok(())
            }
        })
        .run(ALL);
    (
        interrupted.into_inner().unwrap(),
        calls.load(Ordering::SeqCst),
        result,
    )
}

fn assert_printers_error(result: Result<i32>) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Runtime);
    assert!(error.to_string().contains("printer failed"), "{error}");
}

#[test]
fn p10_an_error_stops_the_search_and_run_returns_it() {
    let _guard = serial();
    if !clingox_sys::HAS_THREADS {
        eprintln!("SKIPPED: p10: a blocking solve cannot be interrupted without threads");
        return;
    }
    // Without the interrupt the search would enumerate 2^30 models: turn
    // that regression into a bounded failure instead of a hung suite.
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watchdog = {
        let done = std::sync::Arc::clone(&done);
        std::thread::spawn(move || {
            for _ in 0..600 {
                if done.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            eprintln!("p10: the failing printer did not stop the search within 60 s");
            std::process::abort();
        })
    };
    let (outcome, calls, result) = failing_second(X30);
    done.store(true, Ordering::SeqCst);
    watchdog.join().unwrap();
    assert_printers_error(result);
    // The failing call is the last: the closure is not called for later
    // models and the search was interrupted through the run's handle.
    assert_eq!(calls, 2);
    let outcome = outcome.expect("main ran");
    assert!(
        matches!(outcome, Ok(true)),
        "the solve call reports the interruption: {outcome:?}"
    );
}

#[test]
fn p10c_without_threads_the_search_runs_to_its_end_but_nothing_more_is_printed() {
    let _guard = serial();
    if clingox_sys::HAS_THREADS {
        eprintln!("SKIPPED: p10c: this build has threads (p10 covers it)");
        return;
    }
    let (outcome, calls, result) = failing_second(X3);
    assert_printers_error(result);
    // Documented: no interruption is possible, so clingo still finds all
    // eight models of X3. The closure fails on its second call, which is not
    // the last model, and the count stays 2: it is not called for the six
    // models found after the failure.
    assert_eq!(calls, 2, "no closure call after the failing second call");
    let outcome = outcome.expect("main ran");
    assert!(matches!(outcome, Ok(false)), "{outcome:?}");
}

#[test]
fn p10b_the_printers_error_wins_over_mains_own_and_survives_a_swallowing_main() {
    let _guard = serial();
    // `main` returns its own error after solving: the printer's is first.
    let (result, calls) = failing_run(printer_failed, |_solved| {
        Err(Error::new(ErrorKind::Logic, "main failed"))
    });
    let error = result.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Runtime);
    assert!(error.to_string().contains("printer failed"), "{error}");
    assert_eq!(calls, 2);
    // `main` swallows whatever its solve call returned.
    let (result, calls) = failing_run(printer_failed, |_solved| Ok(()));
    let error = result.unwrap_err();
    assert!(error.to_string().contains("printer failed"), "{error}");
    assert_eq!(calls, 2);
}

#[test]
fn p11_the_kind_of_the_printers_error_is_kept() {
    let _guard = serial();
    let (result, _) = failing_run(
        || Error::new(ErrorKind::Logic, "logic in printer"),
        |_| Ok(()),
    );
    let error = result.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Logic);
    assert!(error.to_string().contains("logic in printer"), "{error}");
}

#[test]
fn p12_a_panic_in_the_printer_is_resumed_after_the_run_is_cleaned_up() {
    let _guard = serial();
    let calls = AtomicUsize::new(0);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        app(X3, |_model, _printer| {
            assert!(
                calls.fetch_add(1, Ordering::SeqCst) + 1 != 2,
                "printer panic"
            );
            Ok(())
        })
        .run(ALL)
    }));
    let payload = outcome.expect_err("the panic must reach the caller of run");
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    assert!(text.contains("printer panic"), "{text}");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "no call after the panic");
    // The run lock and the nesting flag are free again.
    let file = child::fixture("printer_after_panic.lp", "a. {b}.");
    let code = Application::new()
        .run([child::arg(&file).as_str(), "--verbose=0"])
        .unwrap();
    assert_eq!(code, 10);
}

#[test]
fn p13_a_panic_beats_the_error_main_returns_afterwards() {
    let _guard = serial();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        Application::new()
            .main(|ctl, _files| {
                let _ = solve(ctl, X3);
                Err(Error::new(ErrorKind::Runtime, "main failed"))
            })
            .print_model(|_model, _printer| panic!("printer panic"))
            .run(ALL)
    }));
    assert!(outcome.is_err(), "the panic wins over main's error");
}

#[test]
fn p14_a_nested_run_from_the_printer_is_refused() {
    let _guard = serial();
    let refused = Mutex::new(Vec::new());
    let result = app(X2, |_model, _printer| {
        let inner = Application::new().run(["--verbose=0"]);
        refused.lock().unwrap().push(inner);
        Ok(())
    })
    .run(ALL);
    assert_eq!(result.unwrap(), 30, "the outer run finishes normally");
    let refused = refused.into_inner().unwrap();
    assert_eq!(refused.len(), 4);
    for inner in refused {
        let error = inner.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(error.to_string().contains("nested"), "{error}");
    }
}

// ---- API shape ----

#[test]
fn p15_debug_shows_whether_a_printer_is_set() {
    let _guard = serial();
    let without = format!("{:?}", Application::new());
    assert!(without.contains("print_model: false"), "{without}");
    let with = format!("{:?}", Application::new().print_model(|_m, _p| Ok(())));
    assert!(with.contains("print_model: true"), "{with}");
}

#[test]
fn p16_the_closure_borrows_from_the_caller_and_the_borrow_ends_with_the_application() {
    let _guard = serial();
    let numbers = Mutex::new(Vec::<u64>::new());
    let calls = AtomicUsize::new(0);
    let application = app(X2, |model, _printer| {
        numbers.lock().unwrap().push(model.number());
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    assert_eq!(application.run(ALL).unwrap(), 30);
    let mut numbers = numbers.into_inner().unwrap();
    numbers.sort_unstable();
    assert_eq!(numbers, [1, 2, 3, 4]);
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

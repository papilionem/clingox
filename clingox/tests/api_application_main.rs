//! `Application::main`, the control clingo lends to it, and what happens to the
//! state registered on that control, in process.
//!
//! Expected values come from pyclingo 5.8.2 (clingo 5.8.2, clasp 3.4.1) calling
//! `clingo_main` with a `main` callback. The control is a lifetime-branded
//! `ScopedControl<'r>`, so the attacks a runtime lease would have had to catch
//! are compile-fail cases under `tests/ui/application_main_*`; this file checks
//! behaviour. Every run passes `--outf=3`.
//!
//! `run` refuses a second run while one is in progress, so the tests of this
//! file take one lock and never overlap.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::mem_forget,
    clippy::print_stderr,
    reason = "test helpers fail loudly; one test forgets a guard on purpose"
)]

#[path = "common/child.rs"]
mod child;

use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use clingox::application::Application;
use clingox::observer::GroundProgramObserver;
use clingox::propagate::{ClauseType, PropagateControl, PropagateInit, Propagator, SolverLiteral};
use clingox::{
    Control, Error, ErrorKind, InterruptHandle, MessageCode, Part, Result, ShowType, Signature,
    SolveEventHandler, SolveResult, Symbol,
};

const SAT: &str = "a. {b}.";
const WARN: &str = "a :- b. c :- d. e :- f. g :- h.";
/// 11 pigeons in 10 holes: 12.66 s to refute unrestricted, so a
/// search left running is visible as a hang, and a cancelled one as a return.
const PIGEONS: &str =
    "p(1..11). h(1..10). 1 {in(P,H): h(H)} 1 :- p(P). :- in(P1,H), in(P2,H), P1 < P2.";
const BAD: &str = "x :- (.";
const PROGRAM: &str = "a. b :- a. {c}. d :- c.";
const GUARD: Duration = Duration::from_secs(20);

const LIMIT: Duration = Duration::from_secs(30);

static SERIAL: Mutex<()> = Mutex::new(());

/// Aborts the test process if a test runs longer than `LIMIT`: a search left
/// open and not closed by the borrowed control's `Drop` hangs `run`, and a
/// hung test would stall CI with no message. The watchdog thread ends when the
/// guard is dropped. Without threads (WebAssembly) there is no watchdog.
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

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn quiet() -> [&'static str; 1] {
    ["--outf=3"]
}

fn all_models() -> [&'static str; 2] {
    ["--outf=3", "0"]
}

/// Counts its own drops.
struct DropCount(Arc<AtomicUsize>);

impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// ---- the callback's inputs and the exit code ----

#[test]
fn models_files_and_the_exit_code_match_the_oracle() {
    let _lock = serial();
    let _dog = Watchdog::start();
    // Contract test 1: `1 {a; b; c(1/0)}.` with `["file", "--outf=3", "0"]`.
    let file = child::fixture("main_models.lp", "1 {a; b; c(1/0)}.");
    let path = child::arg(&file);
    let mut seen_files = Vec::new();
    let mut models: Vec<Vec<String>> = Vec::new();
    let mut warnings = Vec::new();
    let result = Application::new()
        .logger(|code, _text| warnings.push(code))
        .main(|ctl, files| {
            seen_files.extend(files.iter().map(|f| (*f).to_owned()));
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
                models.push(symbols);
                Ok(ControlFlow::Continue(()))
            })?;
            Ok(())
        })
        .run([path.as_str(), "--outf=3", "0"]);
    assert_eq!(result.unwrap(), 30);
    assert_eq!(seen_files, [path]);
    // The order of this build, not the upstream test's.
    let expected: Vec<Vec<String>> = vec![
        vec!["a".into()],
        vec!["b".into()],
        vec!["a".into(), "b".into()],
    ];
    assert_eq!(models, expected);
    assert!(warnings.contains(&MessageCode::OperationUndefined));
}

#[test]
fn files_hold_the_positional_arguments_only() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let one = child::fixture("main_files_1.lp", SAT);
    let two = child::fixture("main_files_2.lp", "c.");
    let (p1, p2) = (child::arg(&one), child::arg(&two));
    let mut seen: Vec<Vec<String>> = Vec::new();
    let mut record = |files: &[&str]| seen.push(files.iter().map(|f| (*f).to_owned()).collect());
    // The model limit and the options are not files; the order is kept.
    Application::new()
        .main(|_ctl, files| {
            record(files);
            Ok(())
        })
        .run(["--outf=3", "0", p1.as_str(), p2.as_str()])
        .unwrap();
    Application::new()
        .main(|_ctl, files| {
            record(files);
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
    assert_eq!(seen, vec![vec![p1, p2], Vec::<String>::new()]);
}

#[test]
fn main_is_not_called_for_unknown_and_ambiguous_options() {
    let _lock = serial();
    let _dog = Watchdog::start();
    for option in ["--nosuch", "--f"] {
        let drops = Arc::new(AtomicUsize::new(0));
        let guard = DropCount(Arc::clone(&drops));
        let mut calls = 0_u32;
        let code = Application::new()
            .main(|_ctl, _files| {
                let _keep = &guard;
                calls += 1;
                Ok(())
            })
            .run([option])
            .unwrap();
        assert_eq!(code, 1, "{option}");
        assert_eq!(calls, 0, "{option}");
        // The closure was never run and is dropped with the application.
        assert_eq!(drops.load(Ordering::SeqCst), 0, "{option}: guard is ours");
        drop(guard);
        assert_eq!(drops.load(Ordering::SeqCst), 1, "{option}");
    }
}

#[test]
fn main_runs_once_on_the_calling_thread_and_the_last_setter_wins() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let caller = std::thread::current().id();
    let mut on_thread = Vec::new();
    let first_drops = Arc::new(AtomicUsize::new(0));
    let first_guard = DropCount(Arc::clone(&first_drops));
    Application::new()
        .main(move |_ctl, _files| {
            let _keep = &first_guard;
            panic!("the replaced closure must not run")
        })
        .main(|_ctl, _files| {
            on_thread.push(std::thread::current().id());
            Ok(())
        })
        .run(quiet())
        .unwrap();
    assert_eq!(on_thread, [caller]);
    // The replaced closure never ran; it is dropped when replaced.
    assert_eq!(first_drops.load(Ordering::SeqCst), 1);
}

// ---- a failing or panicking main ----

#[test]
fn a_main_error_comes_back_unchanged() {
    let _lock = serial();
    let _dog = Watchdog::start();
    for kind in [
        ErrorKind::Runtime,
        ErrorKind::Logic,
        ErrorKind::InvalidInput,
        ErrorKind::Unknown,
    ] {
        let result = Application::new()
            .main(move |_ctl, _files| Err(Error::new(kind, "my main failed")))
            .run(quiet());
        let error = result.unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(
            error.to_string(),
            Error::new(kind, "my main failed").to_string()
        );
        assert_eq!(error.messages(), []);
    }
}

#[test]
fn a_main_panic_is_resumed_and_the_next_run_works() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let literal = catch_unwind(AssertUnwindSafe(|| {
        Application::new()
            .main(|_ctl, _files| panic!("boom in main"))
            .run(quiet())
    }))
    .unwrap_err();
    assert_eq!(literal.downcast_ref::<&str>(), Some(&"boom in main"));
    let formatted = catch_unwind(AssertUnwindSafe(|| {
        Application::new()
            .main(|_ctl, _files| panic!("boom {}", std::hint::black_box(7)))
            .run(quiet())
    }))
    .unwrap_err();
    assert_eq!(formatted.downcast_ref::<String>().unwrap(), "boom 7");
    // The run flag and the signal dispositions are back: a run works.
    let file = child::fixture("main_after_panic.lp", SAT);
    let code = Application::new()
        .run([child::arg(&file).as_str(), "0", "--outf=3"])
        .unwrap();
    assert_eq!(code, 30);
}

#[test]
fn a_logger_panic_wins_over_a_main_error() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        Application::new()
            .logger(|_code, _text| panic!("logger boom"))
            .main(|ctl, _files| {
                ctl.add_base(WARN)?;
                // The logger panics on the first warning; the panic is stored
                // and grounding still returns.
                let _ = ctl.ground(&[Part::base()]);
                Err(Error::new(ErrorKind::Runtime, "main error"))
            })
            .run(quiet())
    }))
    .unwrap_err();
    assert_eq!(outcome.downcast_ref::<&str>(), Some(&"logger boom"));
}

#[test]
fn a_run_inside_main_is_refused_and_the_outer_run_finishes() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let mut inner = None;
    let outer = Application::new()
        .main(|_ctl, _files| {
            inner = Some(Application::new().run(quiet()));
            Ok(())
        })
        .run(quiet());
    assert!(outer.is_ok());
    let error = inner.unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(error.to_string().contains("nested"), "{error}");
}

// ---- the control inside main ----

#[test]
fn two_program_parts_ground_and_solve_in_main() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let mut models: Vec<Vec<String>> = Vec::new();
    let code = Application::new()
        .main(|ctl, _files| {
            ctl.add_base("a.")?;
            ctl.add("step", &["n"], "v(n).")?;
            ctl.ground(&[
                Part::base(),
                Part::new("step", &[Symbol::number(1)])?,
                Part::new("step", &[Symbol::number(2)])?,
            ])?;
            let (result, found) = ctl.solve_all()?;
            assert!(result.is_sat());
            for model in &found {
                let mut symbols: Vec<String> =
                    model.symbols().iter().map(ToString::to_string).collect();
                symbols.sort();
                models.push(symbols);
            }
            Ok(())
        })
        .run(all_models())
        .unwrap();
    assert_eq!(models, [["a", "v(1)", "v(2)"]]);
    // The exit code is the run's own when `main` does not solve for clingo.
    assert_eq!(code, 30);
}

#[test]
fn a_syntax_error_is_parse_with_empty_messages_and_reaches_the_logger() {
    let _lock = serial();
    let _dog = Watchdog::start();
    // What an owned control reports for the same program.
    let owned = Control::new().unwrap().add_base(BAD).unwrap_err();
    assert_eq!(owned.kind(), ErrorKind::Parse);
    let mut logged: Vec<(MessageCode, String)> = Vec::new();
    let mut from_main = None;
    Application::new()
        .logger(|code, text| logged.push((code, text.to_owned())))
        .main(|ctl, _files| {
            let error = ctl.add_base(BAD).unwrap_err();
            from_main = Some((error.kind(), error.to_string(), error.messages().len()));
            Ok(())
        })
        .run(quiet())
        .unwrap();
    let (kind, text, messages) = from_main.unwrap();
    assert_eq!(kind, owned.kind());
    // Without a capture logger the text is the context and clingo's short
    // report, not the owned control's full message.
    assert!(text.ends_with("parsing failed"), "{text}");
    // No capture logger on this control: clingo delivers the detail to the
    // application's logger instead (pyclingo 5.8.2, notes section 3.5).
    assert_eq!(messages, 0);
    assert!(
        logged.iter().any(|(code, text)| *code == MessageCode::RuntimeError
            && text.contains("syntax error")),
        "{logged:?}"
    );
}

#[test]
fn warnings_in_main_reach_the_application_logger_within_the_limit() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let mut delivered = Vec::new();
    Application::new()
        .message_limit(2)
        .logger(|code, _text| delivered.push(code))
        .main(|ctl, _files| {
            ctl.add_base(WARN)?;
            ctl.ground(&[Part::base()])?;
            Ok(())
        })
        .run(quiet())
        .unwrap();
    assert_eq!(delivered, [MessageCode::AtomUndefined; 2]);
}

#[test]
fn configuration_and_statistics_work_on_the_application_control() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let file = child::fixture("main_config.lp", SAT);
    let path = child::arg(&file);
    let mut seen = None;
    Application::new()
        .main(|ctl, files| {
            let models = ctl.configuration().get("solve.models")?;
            for file in files {
                ctl.load(file)?;
            }
            ctl.ground(&[Part::base()])?;
            // `solve_all` overrides the model limit by design; `for_each_model`
            // honours the configuration.
            let mut all = 0_usize;
            let _ = ctl.for_each_model(&[], |_| {
                all += 1;
                Ok(ControlFlow::Continue(()))
            })?;
            let enumerated = ctl.statistics()?.value("summary.models.enumerated")?;
            ctl.configuration().set("solve.models", "1")?;
            let limited = ctl.configuration().get("solve.models")?;
            let mut one = 0_usize;
            let _ = ctl.for_each_model(&[], |_| {
                one += 1;
                Ok(ControlFlow::Continue(()))
            })?;
            seen = Some((models, all, enumerated, limited, one));
            Ok(())
        })
        .run([path.as_str(), "--outf=3", "0"])
        .unwrap();
    assert_eq!(
        seen.unwrap(),
        (Some("0".to_owned()), 2, 2.0, Some("1".to_owned()), 1)
    );
}

#[test]
fn main_may_borrow_locals() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let mut lines: Vec<String> = vec!["before".to_owned()];
    Application::new()
        .main(|ctl, _files| {
            ctl.add_base("a.")?;
            lines.push("during".to_owned());
            Ok(())
        })
        .run(quiet())
        .unwrap();
    assert_eq!(lines, ["before", "during"]);
}

#[test]
fn an_owned_control_made_in_main_can_leave_it() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let mut kept: Option<Control> = None;
    Application::new()
        .main(|_ctl, _files| {
            kept = Some(Control::new()?);
            Ok(())
        })
        .run(quiet())
        .unwrap();
    let mut ctl = kept.expect("main stored a control");
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_poisoned_control_in_main_does_not_fail_the_run() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let mut after = None;
    let result = Application::new()
        .main(|ctl, _files| {
            ctl.add_base("p(@f(1)).")?;
            // A ground callback error poisons the control.
            let failed = ctl.ground_with(&[Part::base()], |_call| {
                Err(Error::new(ErrorKind::Runtime, "callback failed"))
            });
            assert!(failed.is_err());
            after = Some(ctl.add_base("q.").unwrap_err().kind());
            Ok(())
        })
        .run(quiet());
    assert!(result.is_ok());
    assert_eq!(after, Some(ErrorKind::Poisoned));
}

// ---- registered state ----

struct NoTwoAdjacent {
    lits: OnceLock<Vec<SolverLiteral>>,
}

impl NoTwoAdjacent {
    fn neighbour(&self, lit: SolverLiteral) -> Option<SolverLiteral> {
        let lits = self.lits.get().expect("init ran first");
        let i = lits.iter().position(|&l| l == lit)?;
        lits.get(i + 1).copied()
    }
}

impl Propagator for NoTwoAdjacent {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let mut by_argument = Vec::new();
        for atom in init.symbolic_atoms()?.by_signature(Signature::new("x", 1)?) {
            let atom = atom?;
            let n = atom.symbol().arguments().unwrap()[0]
                .as_number()
                .expect("x/1's own argument is a number");
            by_argument.push((n, atom.literal()));
        }
        by_argument.sort_by_key(|&(n, _)| n);
        let mut lits = Vec::new();
        for (_, plit) in by_argument {
            let slit = init.solver_literal(plit)?;
            init.add_watch(slit)?;
            lits.push(slit);
        }
        let _ = self.lits.set(lits);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        for &lit in changes {
            if let Some(next) = self.neighbour(lit)
                && control
                    .add_clause(&[-lit, -next], ClauseType::Learnt)?
                    .is_stop()
            {
                return Ok(());
            }
        }
        Ok(())
    }
}

#[test]
fn a_propagator_registered_in_main_gives_the_thirteen_independent_sets() {
    let _lock = serial();
    let _dog = Watchdog::start();
    // The fixture and the expected sets of `propagator_no_two_adjacent.rs`
    // (pyclingo 5.8.2), now through `Application::main`.
    let mut models: Vec<Vec<String>> = Vec::new();
    Application::new()
        .main(|ctl, _files| {
            ctl.add_base("{x(1..5)}.")?;
            ctl.ground(&[Part::base()])?;
            ctl.register_propagator(NoTwoAdjacent {
                lits: OnceLock::new(),
            })?;
            let _ = ctl.for_each_model(&[], |model| {
                let mut symbols: Vec<String> = model
                    .symbols(ShowType::SHOWN)?
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                symbols.sort();
                models.push(symbols);
                Ok(ControlFlow::Continue(()))
            })?;
            Ok(())
        })
        .run(all_models())
        .unwrap();
    models.sort();
    let expected: Vec<Vec<&str>> = vec![
        vec![],
        vec!["x(1)"],
        vec!["x(1)", "x(3)"],
        vec!["x(1)", "x(3)", "x(5)"],
        vec!["x(1)", "x(4)"],
        vec!["x(1)", "x(5)"],
        vec!["x(2)"],
        vec!["x(2)", "x(4)"],
        vec!["x(2)", "x(5)"],
        vec!["x(3)"],
        vec!["x(3)", "x(5)"],
        vec!["x(4)"],
        vec!["x(5)"],
    ];
    assert_eq!(models, expected);
}

struct CountRules {
    rules: Arc<AtomicUsize>,
    drops: Option<Arc<AtomicUsize>>,
}

impl GroundProgramObserver for CountRules {
    fn rule(
        &mut self,
        _choice: bool,
        _head: &[clingox::backend::Atom],
        _body: &[clingox::ProgramLiteral],
    ) -> Result<()> {
        self.rules.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl Drop for CountRules {
    fn drop(&mut self) {
        if let Some(drops) = &self.drops {
            drops.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[test]
fn an_observer_registered_in_main_sees_the_same_program_as_on_an_owned_control() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let plain = Arc::new(AtomicUsize::new(0));
    let mut ctl = Control::new().unwrap();
    ctl.register_observer(
        CountRules {
            rules: Arc::clone(&plain),
            drops: None,
        },
        false,
    )
    .unwrap();
    ctl.add_base(PROGRAM).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    drop(ctl);
    let in_main = Arc::new(AtomicUsize::new(0));
    Application::new()
        .main(|ctl, _files| {
            ctl.register_observer(
                CountRules {
                    rules: Arc::clone(&in_main),
                    drops: None,
                },
                false,
            )?;
            ctl.add_base(PROGRAM)?;
            ctl.ground(&[Part::base()])?;
            Ok(())
        })
        .run(quiet())
        .unwrap();
    assert!(plain.load(Ordering::SeqCst) > 0);
    assert_eq!(in_main.load(Ordering::SeqCst), plain.load(Ordering::SeqCst));
}

struct DroppingPropagator {
    _drops: DropCount,
}

impl Propagator for DroppingPropagator {}

#[test]
fn registered_state_is_dropped_once_and_only_after_main() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let observer_drops = Arc::new(AtomicUsize::new(0));
    let propagator_drops = Arc::new(AtomicUsize::new(0));
    let mut during = None;
    Application::new()
        .main(|ctl, _files| {
            ctl.register_observer(
                CountRules {
                    rules: Arc::new(AtomicUsize::new(0)),
                    drops: Some(Arc::clone(&observer_drops)),
                },
                false,
            )?;
            ctl.register_propagator(DroppingPropagator {
                _drops: DropCount(Arc::clone(&propagator_drops)),
            })?;
            ctl.add_base("{x(1..3)}.")?;
            ctl.ground(&[Part::base()])?;
            let _ = ctl.solve(&[])?;
            during = Some((
                observer_drops.load(Ordering::SeqCst),
                propagator_drops.load(Ordering::SeqCst),
            ));
            Ok(())
        })
        .run(quiet())
        .unwrap();
    // Alive while the callback runs (clingo may still call them until it has
    // torn the control down), each dropped exactly once by the time `run` is
    // back.
    assert_eq!(during, Some((0, 0)));
    assert_eq!(observer_drops.load(Ordering::SeqCst), 1);
    assert_eq!(propagator_drops.load(Ordering::SeqCst), 1);
}

// ---- a search left open at the end of main ----

struct Logging(Arc<Mutex<Vec<&'static str>>>);

impl SolveEventHandler for Logging {
    fn on_finish(&mut self, _result: SolveResult) -> Result<ControlFlow<()>> {
        self.0.lock().unwrap().push("finish");
        Ok(ControlFlow::Continue(()))
    }
}

impl Drop for Logging {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("drop");
    }
}

#[test]
fn an_async_search_left_open_by_main_is_closed_and_its_handler_finished_then_dropped() {
    let _lock = serial();
    let _dog = Watchdog::start();
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let started = Instant::now();
    Application::new()
        .main(|ctl, _files| {
            ctl.add_base(PIGEONS)?;
            ctl.ground(&[Part::base()])?;
            let handle = ctl.solve_async_with_events(&[], Logging(Arc::clone(&log)))?;
            // The guard is forgotten: only the control's own `Drop` can close
            // the search now (DESIGN S4).
            std::mem::forget(handle);
            Ok(())
        })
        .run(quiet())
        .unwrap();
    assert!(
        started.elapsed() < GUARD,
        "the search was cancelled, not run to its end (12 s)"
    );
    // clingo delivers the finish event when the search is cancelled; the
    // handler is dropped after it and never called again (a call after the
    // drop is a heap use-after-free under ASan).
    assert_eq!(*log.lock().unwrap(), ["finish", "drop"]);
}

#[test]
fn a_yield_search_left_open_by_main_is_closed_the_same_way() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut first_model = false;
    Application::new()
        .main(|ctl, _files| {
            ctl.add_base("{p(1..20)}.")?;
            ctl.ground(&[Part::base()])?;
            let mut handle = ctl.solve_yield_with_events(&[], Logging(Arc::clone(&log)))?;
            first_model = handle.next_model()?.is_some();
            std::mem::forget(handle);
            Ok(())
        })
        .run(quiet())
        .unwrap();
    assert!(first_model);
    assert_eq!(*log.lock().unwrap(), ["finish", "drop"]);
}

// ---- interrupting ----

#[test]
fn an_interrupt_handle_works_during_main_and_is_inert_after_run() {
    let _lock = serial();
    let _dog = Watchdog::start();
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let mut kept: Option<InterruptHandle> = None;
    let mut interrupted = None;
    Application::new()
        .main(|ctl, _files| {
            ctl.add_base(PIGEONS)?;
            ctl.ground(&[Part::base()])?;
            let stop = ctl.interrupt_handle();
            let handle = ctl.solve_async(&[])?;
            let accepted = stop.interrupt();
            let result = handle.close()?;
            interrupted = Some((accepted, result.is_interrupted()));
            kept = Some(stop);
            Ok(())
        })
        .run(quiet())
        .unwrap();
    assert_eq!(interrupted, Some((true, true)));
    // The control is gone: the handle reaches nothing and says so.
    assert!(!kept.unwrap().interrupt());
}

// ---- many runs ----

#[test]
fn two_hundred_runs_with_main_leave_nothing_behind() {
    let _lock = serial();
    let _dog = Watchdog::start();
    for round in 0..200 {
        let mut count = 0;
        let code = Application::new()
            .main(|ctl, _files| {
                ctl.add_base(SAT)?;
                ctl.ground(&[Part::base()])?;
                let (_, models) = ctl.solve_all()?;
                count = models.len();
                Ok(())
            })
            .run(all_models())
            .unwrap();
        assert_eq!((code, count), (30, 2), "round {round}");
    }
}

// ---- solve events on the application's control (all targets) ----

/// Counts the statistics and finish events of a search.
struct EventCounts {
    statistics: Arc<AtomicUsize>,
    finish: Arc<AtomicUsize>,
}

impl SolveEventHandler for EventCounts {
    fn on_statistics(
        &mut self,
        _step: &mut clingox::MutableStatistics<'_>,
        _accumulated: &mut clingox::MutableStatistics<'_>,
    ) -> Result<ControlFlow<()>> {
        self.statistics.fetch_add(1, Ordering::SeqCst);
        Ok(ControlFlow::Continue(()))
    }

    fn on_finish(&mut self, _result: SolveResult) -> Result<ControlFlow<()>> {
        self.finish.fetch_add(1, Ordering::SeqCst);
        Ok(ControlFlow::Continue(()))
    }
}

fn counted_search(ctl: &mut clingox::ScopedControl<'_>) -> Result<(usize, usize)> {
    let statistics = Arc::new(AtomicUsize::new(0));
    let finish = Arc::new(AtomicUsize::new(0));
    ctl.add_base(SAT)?;
    ctl.ground(&[Part::base()])?;
    let _ = ctl.solve_with_events(
        clingox::SolveOptions::new(),
        EventCounts {
            statistics: Arc::clone(&statistics),
            finish: Arc::clone(&finish),
        },
    )?;
    Ok((
        statistics.load(Ordering::SeqCst),
        finish.load(Ordering::SeqCst),
    ))
}

#[test]
fn a_blocking_search_in_main_delivers_statistics_and_finish_once_like_an_owned_control() {
    let _lock = serial();
    let _dog = Watchdog::start();
    let owned = counted_search(&mut Control::new().unwrap()).unwrap();
    assert_eq!(owned, (1, 1), "the owned control's own log");
    let mut in_main = None;
    Application::new()
        .main(|ctl, _files| {
            in_main = Some(counted_search(ctl)?);
            Ok(())
        })
        .run(quiet())
        .unwrap();
    assert_eq!(in_main, Some(owned));
}

//! Custom scripting languages, in process.
//!
//! Oracle: pyclingo 5.8.2 (clingo 5.8.2), `clingo_register_script` and
//! `clingo_script_version` at the C level through `clingo._internal`, with
//! `clingo_control_add`, `load`, `ground` and `clingo_main`.
//!
//! Registration is process-global and clingo's script registry is
//! unsynchronised, so the crate refuses it once any control exists or an
//! application has run. This file is therefore **one** test that registers
//! every language first and then runs its steps in a fixed order, so the order
//! cannot depend on libtest and the file runs the same in the one process of a
//! WebAssembly host. Cases that need a fresh registry, C standard output or an
//! exit code are in `script_child.rs`.
//!
//! Every language that has run a block is asked about every `@` term for the
//! rest of the process, so each probe answers `callable` only for its own
//! prefix, and the one language with a `main` answers `callable("main")` from
//! a switch that is on only in step 10 (a true answer takes over every
//! default-main run).

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::print_stderr,
    reason = "one test runs numbered steps and fails loudly"
)]

#[path = "common/child.rs"]
mod child;
#[path = "common/script.rs"]
mod probe;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use clingox::application::Application;
use clingox::ast::{self, Ast};
use clingox::script::{self, Script};
use clingox::{Control, Error, ErrorKind, Part, Result, ScopedControl, Symbol};
use probe::{Event, Fail, Probe, State, carries};

/// A language with nothing to do, for registrations that only test refusal.
struct Noop;

impl Script for Noop {
    fn execute(&self, _span: &ast::Span, _code: &str) -> Result<()> {
        Ok(())
    }
}

/// Counts its own drops: a refused script is dropped, an accepted one never.
struct Dropper(Arc<AtomicUsize>);

impl Script for Dropper {
    fn execute(&self, _span: &ast::Span, _code: &str) -> Result<()> {
        Ok(())
    }
}

impl Drop for Dropper {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Langs {
    calc: Arc<State>,
    err: Arc<State>,
    thr: Arc<State>,
    mn: Arc<State>,
    kept_drops: Arc<AtomicUsize>,
}

impl Langs {
    fn reset(&self) {
        for state in [&self.calc, &self.err, &self.thr, &self.mn] {
            state.reset();
            state.take();
            state.take_threads();
        }
    }
}

/// A control with every model wanted and a logger that swallows the
/// "operation undefined" messages the steps provoke.
fn control() -> Control {
    Control::builder()
        .args(["0"])
        .logger(|_, _| {})
        .build()
        .unwrap()
}

fn ground(ctl: &mut Control) -> Result<()> {
    ctl.ground(&[Part::base()])
}

/// The shown atoms of every model, each model sorted.
fn models(ctl: &mut Control) -> Vec<Vec<String>> {
    let (_, models) = ctl.solve_all().unwrap();
    models
        .iter()
        .map(|m| {
            let mut atoms: Vec<String> = m.symbols().iter().map(ToString::to_string).collect();
            atoms.sort();
            atoms
        })
        .collect()
}

fn sorted(mut atoms: Vec<&str>) -> Vec<String> {
    atoms.sort_unstable();
    atoms.into_iter().map(str::to_owned).collect()
}

/// Whether the control refuses everything: it is poisoned.
fn is_poisoned(ctl: &mut Control) -> bool {
    ctl.add_base("z.")
        .is_err_and(|e| e.kind() == ErrorKind::Poisoned)
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> Option<&str> {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
}

#[test]
fn scripts_in_process() {
    let langs = step_0_registration();
    step_1_events(&langs);
    step_2_values(&langs);
    step_3_unknown_type(&langs);
    step_4_load_and_builder(&langs);
    step_5_errors(&langs);
    step_6_panics(&langs);
    step_7_ground_callback_wins(&langs);
    step_8_freeze(&langs);
    step_9_threads(&langs);
    step_10_main(&langs);
    step_11_leak(&langs);
}

// ---- step 0: registrations, before any control exists in the process ----

fn step_0_registration() -> Langs {
    let (calc, calc_state) = Probe::new("calc_");
    let (err, err_state) = Probe::new("err_");
    let (thr, thr_state) = Probe::new("thr_");
    let (mn, mn_state) = Probe::new("mn_");
    script::register("calc", "calc-1.0", calc).unwrap();
    script::register("err", "err-1.0", err).unwrap();
    script::register("thr", "thr-1.0", thr).unwrap();
    script::register("mn", "mn-1.0", mn).unwrap();

    let kept_drops = Arc::new(AtomicUsize::new(0));
    script::register("dropper", "dropper-1.0", Dropper(Arc::clone(&kept_drops))).unwrap();

    assert_eq!(script::version("calc").as_deref(), Some("calc-1.0"));
    assert_eq!(script::version("dropper").as_deref(), Some("dropper-1.0"));
    assert_eq!(script::version("nope"), None);
    assert_eq!(script::version("Calc"), None, "lookup is case sensitive");

    // Any name registers, reachable from a program or not.
    script::register("", "empty-1", Noop).unwrap();
    script::register("Not Reachable", "nr-1", Noop).unwrap();
    assert_eq!(script::version("").as_deref(), Some("empty-1"));
    assert_eq!(script::version("Not Reachable").as_deref(), Some("nr-1"));

    // A duplicate is refused and its script dropped on the spot; the accepted
    // one has not been dropped.
    let refused_drops = Arc::new(AtomicUsize::new(0));
    let err = script::register("calc", "other", Dropper(Arc::clone(&refused_drops))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(
        err.to_string().contains("calc"),
        "names the language: {err}"
    );
    assert_eq!(refused_drops.load(Ordering::SeqCst), 1, "refused: dropped");
    assert_eq!(
        kept_drops.load(Ordering::SeqCst),
        0,
        "accepted: never dropped"
    );
    assert_eq!(script::version("calc").as_deref(), Some("calc-1.0"));

    // A NUL byte is refused before anything is registered.
    let err = script::register("bad\0name", "1", Noop).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    let err = script::register("nulversion", "1\0", Noop).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    assert_eq!(script::version("nulversion"), None);
    assert_eq!(script::version("bad\0name"), None);

    Langs {
        calc: calc_state,
        err: err_state,
        thr: thr_state,
        mn: mn_state,
        kept_drops,
    }
}

// ---- step 1: the events and what the scripts see (test 2) ----

fn step_1_events(langs: &Langs) {
    langs.reset();
    let mut ctl = control();
    ctl.add_base("#script (calc) code here. #end. p(@calc_f(1)).")
        .unwrap();
    // `execute` runs while the program is parsed, not at ground.
    assert_eq!(
        langs.calc.take(),
        [Event::exec("code here.", "<block>:1:1-1:32")]
    );
    ground(&mut ctl).unwrap();
    assert_eq!(
        langs.calc.take(),
        [
            Event::callable("calc_f"),
            Event::call("calc_f", &["1"], "<block>:1:35-1:45"),
        ]
    );
    assert_eq!(models(&mut ctl), [["p(42)"]]);

    // The code text and the span, three layouts (oracle).
    for (program, code, span) in [
        ("#script (calc) x #end.", "x", "<block>:1:1-1:23"),
        ("#script (calc)\nx\n#end.", "\nx\n", "<block>:1:1-3:6"),
        (
            "#script (calc)\n  line1\n   line2 #end.",
            "\n  line1\n   line2",
            "<block>:1:1-3:15",
        ),
    ] {
        let mut ctl = control();
        ctl.add_base(program).unwrap();
        assert_eq!(langs.calc.take(), [Event::exec(code, span)], "{program:?}");
    }
}

// ---- step 2: values (test 2, oracle "values") ----

fn step_2_values(langs: &Langs) {
    langs.reset();
    let mut ctl = control();
    ctl.add_base(
        "#script (calc) x #end. a. p(@calc_none(1)). q(@calc_pool(1)). t(@calc_pool(1), 9). \
         r(@calc_echo(1,\"a\",b,(1,2),-3,#sup,#inf,f(g(2)))). s(@calc_g()). u(@calc_g(1)). \
         w(@zzz(1)).",
    )
    .unwrap();
    ground(&mut ctl).unwrap();
    // No `p` (zero values drop the rule), a pool of three and its cross
    // product, the arguments reversed by `echo`, an empty argument slice for
    // `calc_g()`, and no error for a name no script owns.
    assert_eq!(
        models(&mut ctl),
        [sorted(vec![
            "a",
            "q(1)",
            "q(2)",
            "q(3)",
            "r(\"a\")",
            "r(#inf)",
            "r(#sup)",
            "r((1,2))",
            "r(-3)",
            "r(1)",
            "r(b)",
            "r(f(g(2)))",
            "s(42)",
            "t(1,9)",
            "t(2,9)",
            "t(3,9)",
            "u(42)",
        ])]
    );
    let events = langs.calc.take();
    assert!(
        events.iter().any(|e| matches!(e,
            Event::Call { name, args, .. }
                if name == "calc_echo"
                    && args == &["1", "\"a\"", "b", "(1,2)", "-3", "#sup", "#inf", "f(g(2))"])),
        "every argument kind arrives: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(e,
            Event::Call { name, args, .. } if name == "calc_g" && args.is_empty())),
        "an empty argument slice: {events:?}"
    );
    assert!(
        events.contains(&Event::callable("zzz")),
        "the script was asked, and said no: {events:?}"
    );
}

// ---- step 3: an unknown language (test 3) ----

fn step_3_unknown_type(langs: &Langs) {
    langs.reset();
    let mut ctl = control();
    let err = ctl
        .add_base("#script (calc_nonexistent) x #end.")
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse, "add remaps clingo's runtime");
    assert!(
        err.to_string()
            .contains("calc_nonexistent support not available"),
        "{err}"
    );
    assert!(is_poisoned(&mut ctl), "a failed add poisons");
    let file = child::fixture(
        "script_unknown.lp",
        "#script (calc_nonexistent) x #end.\na.\n",
    );
    let mut ctl = control();
    let err = ctl.load(&file).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime, "load keeps clingo's kind");
    assert!(
        err.to_string()
            .contains("calc_nonexistent support not available"),
        "{err}"
    );
    assert!(is_poisoned(&mut ctl), "a failed load poisons");
}

// ---- step 4: `load` and the program builder ----

fn step_4_load_and_builder(langs: &Langs) {
    langs.reset();
    let file = child::fixture(
        "script_block.lp",
        "#script (calc)\nfile code\n#end.\np(@calc_f(1)).\n",
    );
    let path = child::arg(&file);
    let mut ctl = control();
    ctl.load(&file).unwrap();
    assert_eq!(
        langs.calc.take(),
        [Event::exec("\nfile code\n", &format!("{path}:1:1-3:6"))]
    );
    ground(&mut ctl).unwrap();
    assert_eq!(
        langs.calc.take(),
        [
            Event::callable("calc_f"),
            Event::call("calc_f", &["1"], &format!("{path}:4:3-4:13")),
        ]
    );
    assert_eq!(models(&mut ctl), [["p(42)"]]);

    // Parsing a tree runs nothing; adding the statement through the program
    // builder runs `execute`, with the file name clingo gives a string.
    let mut statements: Vec<Ast> = Vec::new();
    ast::parse_string("#script (calc) x #end.", |statement| {
        statements.push(statement);
        Ok(())
    })
    .unwrap();
    assert!(langs.calc.take().is_empty(), "parse_string runs no script");
    let mut ctl = control();
    ctl.with_program_builder(|builder| {
        for statement in &statements {
            builder.add(statement)?;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        langs.calc.take(),
        [Event::exec("x", "<string>:1:1-1:23")],
        "the builder runs the block"
    );
}

// ---- step 5: returned errors (test 6) ----

const KINDS: [Fail; 3] = [Fail::Callback, Fail::Runtime, Fail::Logic];

fn step_5_errors(langs: &Langs) {
    let err = &langs.err;
    for fail in KINDS {
        let text = "exec failed";
        // `add`: the error comes back as returned, its kind unchanged (a
        // Runtime error is not remapped to Parse), and the second block does
        // not run.
        langs.reset();
        err.fail_exec(Some((fail, text)));
        let mut ctl = control();
        let e = ctl
            .add_base("#script (err) A #end. a. #script (err) B #end. b.")
            .unwrap_err();
        assert_eq!(e.kind(), fail.kind(), "{fail:?} from add: {e}");
        assert!(carries(&e, text), "{fail:?}: {e:?}");
        assert_eq!(
            err.take(),
            [Event::exec("A", "<block>:1:1-1:22")],
            "{fail:?}: the second block must not run"
        );
        err.fail_exec(None);
        // Any execute error poisons: clingo keeps the rest of the
        // add's input queued (`b.`) and the next add would resume it, so the
        // hazard is closed by refusing the next call, whatever the kind.
        assert!(is_poisoned(&mut ctl), "{fail:?}: an execute error poisons");
        assert_eq!(
            ground(&mut ctl).unwrap_err().kind(),
            ErrorKind::Poisoned,
            "{fail:?}: `b` can never be grounded"
        );

        // `load`.
        let file = child::fixture("script_exec_fail.lp", "#script (err) A #end.\na.\n");
        err.fail_exec(Some((fail, text)));
        let mut ctl = control();
        let e = ctl.load(&file).unwrap_err();
        assert_eq!(e.kind(), fail.kind(), "{fail:?} from load: {e}");
        assert!(carries(&e, text), "{fail:?}: {e:?}");
        assert!(is_poisoned(&mut ctl), "{fail:?}: a failed load poisons");
        err.fail_exec(None);
        err.take();

        // The program builder.
        let mut statements = Vec::new();
        ast::parse_string("#script (err) A #end.", |s| {
            statements.push(s);
            Ok(())
        })
        .unwrap();
        err.fail_exec(Some((fail, text)));
        let mut ctl = control();
        let e = ctl
            .with_program_builder(|builder| {
                for statement in &statements {
                    builder.add(statement)?;
                }
                Ok(())
            })
            .unwrap_err();
        assert_eq!(e.kind(), fail.kind(), "{fail:?} from the builder: {e}");
        assert!(carries(&e, text), "{fail:?}: {e:?}");
        err.fail_exec(None);
        err.take();

        // `callable` and `call` fail grounding, whatever the kind, and poison.
        for (which, text) in [("callable", "callable failed"), ("call", "call failed")] {
            langs.reset();
            let mut ctl = control();
            ctl.add_base("#script (err) x #end. p(@err_f(1)). q(@err_f(2)).")
                .unwrap();
            err.take();
            if which == "callable" {
                err.fail_callable(Some((fail, text)));
            } else {
                err.fail_call(Some((fail, text)));
            }
            let e = ground(&mut ctl).unwrap_err();
            assert_eq!(e.kind(), fail.kind(), "{fail:?} from {which}: {e}");
            assert!(carries(&e, text), "{which} {fail:?}: {e:?}");
            let events = err.take();
            let asked = events
                .iter()
                .filter(|ev| matches!(ev, Event::Callable(_)))
                .count();
            let called = events
                .iter()
                .filter(|ev| matches!(ev, Event::Call { .. }))
                .count();
            assert_eq!(
                (asked, called),
                (1, usize::from(which == "call")),
                "{which} {fail:?}: grounding stops at the first failure: {events:?}"
            );
            assert!(is_poisoned(&mut ctl), "{which} {fail:?}: poisons");
            // A fresh control is unaffected once the failure is off.
            langs.reset();
            let mut ctl = control();
            ctl.add_base("#script (err) x #end. p(@err_f(1)).").unwrap();
            ground(&mut ctl).unwrap();
            assert_eq!(models(&mut ctl), [["p(42)"]]);
        }
    }
}

// ---- step 6: panics ----

fn step_6_panics(langs: &Langs) {
    let err = &langs.err;
    for which in ["execute", "callable", "call"] {
        langs.reset();
        let text = "boom";
        let mut ctl = control();
        let payload = if which == "execute" {
            err.fail_exec(Some((Fail::Panic, text)));
            catch_unwind(AssertUnwindSafe(|| {
                let _ = ctl.add_base("#script (err) A #end. #script (err) B #end.");
            }))
            .unwrap_err()
        } else {
            ctl.add_base("#script (err) x #end. p(@err_f(1)). q(@err_f(2)).")
                .unwrap();
            err.take();
            if which == "callable" {
                err.fail_callable(Some((Fail::Panic, text)));
            } else {
                err.fail_call(Some((Fail::Panic, text)));
            }
            catch_unwind(AssertUnwindSafe(|| {
                let _ = ground(&mut ctl);
            }))
            .unwrap_err()
        };
        assert_eq!(panic_text(payload.as_ref()), Some(text), "{which}");
        assert!(is_poisoned(&mut ctl), "{which}: a panic poisons");
        let events = err.take();
        assert_eq!(
            events
                .iter()
                .filter(|e| match which {
                    "execute" => matches!(e, Event::Exec { .. }),
                    "callable" => matches!(e, Event::Callable(_)),
                    _ => matches!(e, Event::Call { .. }),
                })
                .count(),
            1,
            "{which}: nothing ran after the panic: {events:?}"
        );

        // The slot is clear: a new control grounds normally.
        langs.reset();
        let mut ctl = control();
        ctl.add_base("#script (err) x #end. p(@err_f(1)).").unwrap();
        ground(&mut ctl).unwrap();
        assert_eq!(models(&mut ctl), [["p(42)"]], "{which}: fresh control");
    }
}

// ---- step 7: a ground callback wins over every script ----

fn step_7_ground_callback_wins(langs: &Langs) {
    langs.reset();
    let program = "#script (calc) x #end. p(@calc_f(1)). q(@calc_g(2)).";

    let mut ctl = control();
    ctl.add_base(program).unwrap();
    langs.calc.take();
    ctl.ground_with(&[Part::base()], |call| call.push(Symbol::number(7)))
        .unwrap();
    assert!(
        langs.calc.take().is_empty(),
        "the script is not asked while a callback is given"
    );
    assert_eq!(models(&mut ctl), [["p(7)", "q(7)"]]);

    // A callback that pushes nothing drops the rule, and the script is still
    // not asked: there is no fallthrough.
    let mut ctl = control();
    ctl.add_base(program).unwrap();
    langs.calc.take();
    ctl.ground_with(&[Part::base()], |_| Ok(())).unwrap();
    assert!(langs.calc.take().is_empty());
    assert_eq!(models(&mut ctl), [Vec::<String>::new()]);

    // A plain `ground` does ask it.
    let mut ctl = control();
    ctl.add_base(program).unwrap();
    langs.calc.take();
    ground(&mut ctl).unwrap();
    assert!(
        langs.calc.take().contains(&Event::callable("calc_f")),
        "plain ground consults the script"
    );
    assert_eq!(models(&mut ctl), [["p(42)", "q(42)"]]);
}

// ---- step 8: registration is refused once a control exists (test 5) ----

fn step_8_freeze(_langs: &Langs) {
    let late = Arc::new(AtomicUsize::new(0));
    let err = script::register("late", "1", Dropper(Arc::clone(&late))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(err.to_string().contains("Control"), "{err}");
    assert_eq!(
        late.load(Ordering::SeqCst),
        1,
        "the refused script is dropped"
    );
    assert_eq!(script::version("late"), None);
    assert_eq!(script::version("calc").as_deref(), Some("calc-1.0"));
}

// ---- step 9: threads (test 9) ----

fn step_9_threads(langs: &Langs) {
    if !clingox_sys::HAS_THREADS {
        eprintln!("SKIPPED: step 9 needs threads");
        return;
    }
    let thr = &langs.thr;

    // With four solver threads and sixteen models, every script callback ran
    // on the thread that grounded.
    langs.reset();
    let mut ctl = Control::builder()
        .args(["0", "-t4"])
        .logger(|_, _| {})
        .build()
        .unwrap();
    ctl.add_base("#script (thr) x #end. {a;b;c;d}. p(@thr_f(1)).")
        .unwrap();
    ground(&mut ctl).unwrap();
    assert_eq!(models(&mut ctl).len(), 16);
    let seen = thr.take_threads();
    assert!(!seen.is_empty());
    assert!(
        seen.iter().all(|t| *t == thread::current().id()),
        "callbacks run on the grounding thread only"
    );

    // Two threads inside `call` at the same time (the overlap is required: each
    // waits for the other, up to five seconds), and a failure on one does not
    // reach the other.
    langs.reset();
    let inside = Arc::new(AtomicUsize::new(0));
    let overlapped = Arc::new(Mutex::new(Vec::new()));
    {
        let (inside, overlapped) = (Arc::clone(&inside), Arc::clone(&overlapped));
        thr.set_hook(Some(Arc::new(move |_name: &str, args: &[Symbol]| {
            inside.fetch_add(1, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while inside.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            overlapped
                .lock()
                .unwrap()
                .push(inside.load(Ordering::SeqCst) >= 2);
            if args[0].as_number() == Some(1) {
                return Err(Error::new(ErrorKind::Logic, "thread one fails"));
            }
            Ok(())
        })));
    }
    let results: Vec<Result<Vec<Vec<String>>>> = thread::scope(|scope| {
        let handles: Vec<_> = [1, 2]
            .into_iter()
            .map(|n| {
                scope.spawn(move || -> Result<Vec<Vec<String>>> {
                    let mut ctl = control();
                    ctl.add_base(&format!("#script (thr) x #end. p(@thr_f({n}))."))?;
                    ground(&mut ctl)?;
                    Ok(models(&mut ctl))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(
        *overlapped.lock().unwrap(),
        [true, true],
        "both were inside"
    );
    let e = results[0].as_ref().unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Logic);
    assert!(e.to_string().contains("thread one fails"), "{e}");
    assert_eq!(results[1].as_ref().unwrap(), &[["p(42)"]]);
    langs.reset();
}

// ---- step 10: a script's `main` through `Application::run` (test 7) ----

fn run_file(path: &str, extra: &[&str]) -> Result<i32> {
    let mut args = vec![path, "0", "--outf=3"];
    args.extend_from_slice(extra);
    Application::new().run(args)
}

fn step_10_main(langs: &Langs) {
    let mn = &langs.mn;
    let file = child::fixture("script_main.lp", "#script (mn) x #end. a.");
    let path = child::arg(&file);

    // The script's main gets a live control and can add, ground and solve.
    langs.reset();
    mn.want_main(true);
    let solved = Arc::new(Mutex::new(Vec::new()));
    {
        let solved = Arc::clone(&solved);
        mn.set_main(Some(Arc::new(move |ctl: &mut ScopedControl<'_>| {
            ctl.add_base("y.")?;
            ctl.ground(&[Part::base()])?;
            let (_, models) = ctl.solve_all()?;
            for m in &models {
                solved.lock().unwrap().push(
                    m.symbols()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                );
            }
            Ok(())
        })));
    }
    // Oracle: with `0` models requested and one model, exhausted: 30.
    assert_eq!(run_file(&path, &[]).unwrap(), 30);
    assert_eq!(*solved.lock().unwrap(), [["a", "y"]]);
    let events = mn.take();
    assert_eq!(events[0], Event::exec("x", &format!("{path}:1:1-1:21")));
    assert_eq!(
        &events[1..],
        [
            Event::callable("main"),
            Event::callable("main"),
            Event::Main
        ],
        "callable is asked twice, then main runs"
    );

    // A main that does nothing: exit code 0 (oracle: UNKNOWN summary).
    mn.set_main(None);
    assert_eq!(run_file(&path, &[]).unwrap(), 0);
    mn.take();

    // An application `main` wins, and the script is not asked about main.
    let seen = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&seen);
    let code = Application::new()
        .main(move |ctl, files| {
            counted.fetch_add(1, Ordering::SeqCst);
            for file in files {
                ctl.load(file)?;
            }
            Ok(())
        })
        .run([path.as_str(), "--outf=3"])
        .unwrap();
    assert_eq!(code, 0);
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert_eq!(
        mn.take(),
        [Event::exec("x", &format!("{path}:1:1-1:21"))],
        "only the block ran: no callable(main), no script main"
    );

    // An error from the script's main is what `run` returns, kind kept.
    for fail in [Fail::Callback, Fail::Runtime, Fail::Logic] {
        mn.set_main(Some(Arc::new(
            move |_: &mut ScopedControl<'_>| -> Result<()> {
                Err(match fail {
                    Fail::Callback => Error::callback(std::io::Error::other("main failed")),
                    Fail::Runtime => Error::new(ErrorKind::Runtime, "main failed"),
                    _ => Error::new(ErrorKind::Logic, "main failed"),
                })
            },
        )));
        let e = run_file(&path, &[]).unwrap_err();
        assert_eq!(e.kind(), fail.kind(), "{fail:?}: {e}");
        assert!(carries(&e, "main failed"), "{fail:?}: {e:?}");
    }
    mn.take();

    // A panic in the script's main is resumed by `run`, and the next run works.
    mn.set_main(Some(Arc::new(|_: &mut ScopedControl<'_>| -> Result<()> {
        panic!("boom main")
    })));
    let payload = catch_unwind(AssertUnwindSafe(|| {
        let _ = run_file(&path, &[]);
    }))
    .unwrap_err();
    assert_eq!(panic_text(payload.as_ref()), Some("boom main"));
    mn.set_main(None);
    assert_eq!(run_file(&path, &[]).unwrap(), 0, "the next run works");
    mn.take();

    // A failing call made inside main, on the control it received, returns its
    // error there; main swallows it and the run is not failed by it.
    let inner = Arc::new(Mutex::new(None));
    {
        let inner = Arc::clone(&inner);
        let failing = Arc::clone(&langs.err);
        failing.fail_exec(Some((Fail::Runtime, "inner failure")));
        mn.set_main(Some(Arc::new(move |ctl: &mut ScopedControl<'_>| {
            let e = ctl.add_base("#script (err) x #end. b.").unwrap_err();
            *inner.lock().unwrap() = Some((e.kind(), e.to_string()));
            Ok(())
        })));
    }
    assert_eq!(
        run_file(&path, &[]).unwrap(),
        0,
        "the inner error stays inside"
    );
    let (kind, text) = inner.lock().unwrap().take().unwrap();
    assert_eq!(kind, ErrorKind::Runtime);
    assert!(text.contains("inner failure"), "{text}");
    langs.reset();

    // A failing `execute` during the run's own parse, and a failing `call`
    // during its own ground: `run` returns the error, never `Ok(65)`.
    mn.fail_exec(Some((Fail::Runtime, "exec in run")));
    let e = run_file(&path, &[]).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Runtime);
    assert!(e.to_string().contains("exec in run"), "{e}");
    mn.reset();
    mn.take();
    let calling = child::fixture(
        "script_main_call.lp",
        "#script (mn) x #end. a. p(@mn_f(1)).",
    );
    mn.fail_call(Some((Fail::Logic, "call in run")));
    let e = run_file(&child::arg(&calling), &[]).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Logic);
    assert!(e.to_string().contains("call in run"), "{e}");
    langs.reset();

    // Switch off: a default run solves again (two models, code 30).
    let plain = child::fixture("script_main_plain.lp", "#script (mn) x #end. a. {b}.");
    assert_eq!(run_file(&child::arg(&plain), &[]).unwrap(), 30);
}

// ---- step 11: the leak (test 10) ----

fn step_11_leak(langs: &Langs) {
    assert_eq!(
        langs.kept_drops.load(Ordering::SeqCst),
        0,
        "an accepted script is never dropped"
    );
}

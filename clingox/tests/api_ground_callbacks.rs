//! External functions called during grounding: `ground_with` and
//! `FunctionCall`.
//!
//! Expected models were checked against the Python module `clingo` 5.8.2 with
//! a context object that implements the same functions.

#![forbid(unsafe_code)]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

// The prelude alone must be enough for ground callbacks.
use clingox::prelude::*;
use clingox::{Error, ErrorKind};

/// The shown symbols of every model, each sorted, as texts.
fn models(ctl: &mut Control) -> Vec<Vec<String>> {
    let (result, models) = ctl.solve_all().expect("the program solves");
    assert!(result.is_exhausted());
    models
        .iter()
        .map(|m| m.symbols().iter().map(ToString::to_string).collect())
        .collect()
}

fn texts(sets: &[&[&str]]) -> Vec<Vec<String>> {
    sets.iter()
        .map(|set| set.iter().map(|s| (*s).to_owned()).collect())
        .collect()
}

/// Implements `@double(n)`, `@seq(n)` (the numbers 1 to n), `@none()` (no
/// value) and `@pair(a, b)` (the tuple `(a,b)`).
fn functions(call: &mut FunctionCall<'_>) -> clingox::Result<()> {
    match (call.name(), call.args()) {
        ("double", [n]) => {
            let n = n.as_number().expect("double takes a number");
            call.push(Symbol::number(n * 2))
        }
        ("seq", [n]) => {
            for i in 1..=n.as_number().expect("seq takes a number") {
                call.push(Symbol::number(i))?;
            }
            Ok(())
        }
        ("none", []) => Ok(()),
        ("pair", [a, b]) => {
            let pair = Symbol::tuple(&[*a, *b])?;
            call.push(pair)
        }
        (name, args) => panic!("unexpected call @{name}/{}", args.len()),
    }
}

fn grounded_with(program: &str) -> Control {
    let mut ctl = Control::new().expect("creating a control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground_with(&[Part::base()], functions)
        .expect("the program grounds");
    ctl
}

#[derive(Debug)]
struct UnknownFunction(String);

impl std::fmt::Display for UnknownFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no function @{}", self.0)
    }
}

impl std::error::Error for UnknownFunction {}

#[test]
fn a_ground_callback_computes_a_value() {
    let mut ctl = grounded_with("p(@double(21)).");
    assert_eq!(models(&mut ctl), texts(&[&["p(42)"]]));
}

#[test]
fn a_ground_callback_can_return_several_values() {
    let mut ctl = grounded_with("p(@seq(3)).");
    assert_eq!(models(&mut ctl), texts(&[&["p(1)", "p(2)", "p(3)"]]));
}

#[test]
fn a_ground_callback_without_values_drops_the_rule() {
    let mut ctl = grounded_with("p(@none()). q.");
    assert_eq!(models(&mut ctl), texts(&[&["q"]]));
}

#[test]
fn a_ground_callback_receives_its_arguments() {
    let mut ctl = grounded_with("p(@pair(1,x)).");
    assert_eq!(models(&mut ctl), texts(&[&["p((1,x))"]]));

    let mut seen = Vec::new();
    let mut ctl = Control::new().unwrap();
    ctl.add_base(r#"p(@f(1, "s", g(a))). p(X) :- X = @f(Y), Y = 2..3."#)
        .unwrap();
    ctl.ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
        seen.push(format!("{}/{:?}", call.name(), call.args()));
        call.push(Symbol::number(0))
    })
    .unwrap();
    seen.sort();
    assert_eq!(
        seen,
        vec![
            r#"f/[Symbol(1), Symbol("s"), Symbol(g(a))]"#,
            "f/[Symbol(2)]",
            "f/[Symbol(3)]",
        ]
    );
}

#[test]
fn a_ground_callback_runs_on_the_calling_thread_and_may_borrow() {
    // The closure borrows a local mutably and captures an `Rc`, which is not
    // `Send`: ground callbacks run on the calling thread (DESIGN S10).
    let caller = std::thread::current().id();
    let calls = Rc::new(Cell::new(0_u32));
    let counter = Rc::clone(&calls);
    let mut names = Vec::new();

    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(@double(1)). q(@double(1)).").unwrap();
    ctl.ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
        assert_eq!(std::thread::current().id(), caller);
        counter.set(counter.get() + 1);
        names.push(call.name().to_owned());
        functions(call)
    })
    .unwrap();
    // clingo evaluates each occurrence; it does not cache calls.
    assert_eq!(calls.get(), 2);
    assert_eq!(names, vec!["double", "double"]);
    assert_eq!(models(&mut ctl), texts(&[&["p(2)", "q(2)"]]));
}

#[test]
fn an_error_from_a_ground_callback_is_returned_and_poisons() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(@nosuch(1)).").unwrap();
    ctl.add("next", &[], "q.").unwrap();
    let err = ctl
        .ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
            Err(Error::callback(UnknownFunction(call.name().to_owned())))
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
    let source = std::error::Error::source(&err).expect("the user error is the source");
    let unknown = source
        .downcast_ref::<UnknownFunction>()
        .expect("the source keeps its type");
    assert_eq!(unknown.0, "nosuch");
    // The Display names the operation only; the user's text is reached
    // through `source()`.
    assert_eq!(err.to_string(), "grounding `base`: the callback failed");

    // Grounding reverses this: clingo 5.8.2 keeps the part of `base`
    // it already ground before the failed call and would answer from it
    // silently, so the control is poisoned instead, and only a new `Control`
    // recovers.
    assert!(format!("{ctl:?}").contains("poisoned"));
    let err = ctl.ground(&[Part::new("next", &[]).unwrap()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

#[test]
fn an_error_from_a_ground_callback_is_returned_whatever_its_kind() {
    // A logic error raised by clingo itself already poisoned;
    // one returned by the callback now poisons too, for the same reason
    // as the test above, whatever kind the callback's own error carries.
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(@f()).").unwrap();
    ctl.add("next", &[], "q.").unwrap();
    let err = ctl
        .ground_with(&[Part::base()], |_: &mut FunctionCall<'_>| {
            let _other = Control::with_args(["--no-such-option"])?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Logic);
    assert!(format!("{ctl:?}").contains("poisoned"));
    let err = ctl.ground(&[Part::new("next", &[]).unwrap()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

#[test]
fn a_clingox_error_from_a_ground_callback_is_returned_unchanged() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(@f()).").unwrap();
    let err = ctl
        .ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
            let symbol = Symbol::function("bad\0name", &[])?;
            call.push(symbol)
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
}

#[test]
fn a_panic_in_a_ground_callback_resumes_on_the_caller() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(@f()).").unwrap();
    ctl.add("next", &[], "q.").unwrap();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctl.ground_with(&[Part::base()], |_: &mut FunctionCall<'_>| {
            panic!("stop here")
        })
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"stop here"));

    // A panic mid-grounding poisons too, the same reason as a returned
    // error (clingo keeps a truncated, partially ground part).
    assert!(format!("{ctl:?}").contains("poisoned"));
    let err = ctl.ground(&[Part::new("next", &[]).unwrap()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

/// Pins *why* a failed ground callback poisons: clingo keeps whatever it
/// already ground before the callback failed, and answers from it silently.
/// Checked directly against clingo 5.8.2: a callback
/// failing at `@f(3)` in `p(1..4). q(X) :- p(X), X = @f(X).` (called in order
/// for `X` = 1, 2, 3, 4, so `q(1)` and `q(2)`'s rule instances are already
/// added when it fails on 3) leaves a control that would answer
/// `p(1) p(2) p(3) p(4) q(1) q(2)` if it could be solved: `p`'s facts are
/// unaffected, and `q`'s first two instances are already there, but `q(3)`/
/// `q(4)` never exist. clingox refuses to solve this truncated, silently
/// wrong program at all, rather than return it.
#[test]
fn a_failed_ground_callback_leaves_the_control_refusing_to_solve_the_truncated_program() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(1..4). q(X) :- p(X), X = @f(X).").unwrap();
    let err = ctl
        .ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
            let n = call.args()[0].as_number().expect("f takes a number");
            if n == 3 {
                return Err(Error::callback(UnknownFunction("f".to_owned())));
            }
            call.push(Symbol::number(n))
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
    assert!(format!("{ctl:?}").contains("poisoned"));
    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::Poisoned,
        "solving must not silently return the truncated program's answer"
    );
}

#[test]
fn plain_ground_reports_an_external_function_as_undefined() {
    // Without a callback clingo logs the call and drops the rule instance.
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    let mut ctl = Control::builder()
        .logger(move |code, text: &str| {
            sink.lock().unwrap().push((code, text.to_owned()));
        })
        .build()
        .unwrap();
    ctl.add_base("p(@nosuch(1)). q.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(models(&mut ctl), texts(&[&["q"]]));

    let log = log.lock().unwrap();
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0].0, MessageCode::OperationUndefined);
    assert!(
        log[0].1.contains("function 'nosuch' not found"),
        "{}",
        log[0].1
    );
}

#[test]
fn ground_with_refuses_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let err = ctl.ground_with(&[Part::base()], functions).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

#[test]
fn function_calls_show_their_name_and_arguments_in_debug() {
    let mut shown = String::new();
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(@double(21)).").unwrap();
    ctl.ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
        shown = format!("{call:?}");
        functions(call)
    })
    .unwrap();
    assert!(shown.contains("double"), "{shown}");
    assert!(shown.contains("21"), "{shown}");
}

// ---------------------------------------------------------------------------
// a failed callback keeps clingo's messages

#[test]
fn a_failed_ground_callback_keeps_the_messages_clingo_logged() {
    // Python clingo 5.8.2 logs `<block>:1:9-13: info: atom does not occur in
    // any rule head:\n  r(X)` while grounding this program, before it calls
    // `@fail`.
    let mut ctl = Control::new().unwrap();
    ctl.add_base("q(X) :- r(X). p(Y) :- Y = @fail(1).").unwrap();
    let err = ctl
        .ground_with(&[Part::base()], |_: &mut FunctionCall<'_>| {
            Err(Error::callback(UnknownFunction("fail".to_owned())))
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
    let messages = err.messages();
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].code(), MessageCode::AtomUndefined);
    assert!(
        messages[0].text().contains("r(X)"),
        "{}",
        messages[0].text()
    );
    let location = messages[0].location().expect("the message has a position");
    assert_eq!((location.line(), location.column()), (1, 9));
}

#[test]
fn a_clingox_error_from_a_ground_callback_keeps_the_messages_too() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("q(X) :- r(X). p(Y) :- Y = @f(1).").unwrap();
    let err = ctl
        .ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
            call.push(Symbol::string("a\0b")?)
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    assert_eq!(err.messages().len(), 1, "{:?}", err.messages());
    assert_eq!(err.messages()[0].code(), MessageCode::AtomUndefined);
}

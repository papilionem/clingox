//! A configurable scripting language for the script tests (`api_script.rs`,
//! `script_child.rs`). A test file includes it with
//! `#[path = "common/script.rs"] mod probe;`.
//!
//! Every language that has run a block is asked about every `@` term for the
//! rest of the process, so a probe answers `callable` only for names that
//! start with its own prefix (`calc_`, `err_`, ...), and answers
//! `callable("main")` only while its switch is on. Every callback is recorded
//! as an [`Event`], with the thread that ran it.

#![allow(dead_code, reason = "each test file uses a part of these helpers")]
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "a probe panics on request, and helpers fail loudly"
)]

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, ThreadId};

use clingox::ast::Span;
use clingox::script::Script;
use clingox::{Error, ErrorKind, Result, ScopedControl, Symbol};

/// One callback a probe received. Spans read `file:line:column-line:column`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Exec {
        code: String,
        span: String,
    },
    Callable(String),
    Call {
        name: String,
        args: Vec<String>,
        span: String,
    },
    Main,
}

impl Event {
    pub(crate) fn exec(code: &str, span: &str) -> Event {
        Event::Exec {
            code: code.to_owned(),
            span: span.to_owned(),
        }
    }

    pub(crate) fn callable(name: &str) -> Event {
        Event::Callable(name.to_owned())
    }

    pub(crate) fn call(name: &str, args: &[&str], span: &str) -> Event {
        Event::Call {
            name: name.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            span: span.to_owned(),
        }
    }
}

/// How a probe's callback fails when told to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fail {
    /// `Error::callback` of an `io::Error`, kind `Callback`.
    Callback,
    /// `Error::new(ErrorKind::Runtime, ..)`.
    Runtime,
    /// `Error::new(ErrorKind::Logic, ..)`.
    Logic,
    /// A panic with the text as its `&'static str` payload.
    Panic,
}

impl Fail {
    /// The kind the error must have when it comes back to the caller.
    pub(crate) fn kind(self) -> ErrorKind {
        match self {
            Fail::Callback => ErrorKind::Callback,
            Fail::Runtime => ErrorKind::Runtime,
            Fail::Logic => ErrorKind::Logic,
            Fail::Panic => panic!("a panic has no kind"),
        }
    }

    fn act(self, text: &'static str) -> Error {
        match self {
            Fail::Callback => Error::callback(io::Error::other(text)),
            Fail::Runtime => Error::new(ErrorKind::Runtime, text),
            Fail::Logic => Error::new(ErrorKind::Logic, text),
            Fail::Panic => panic!("{text}"),
        }
    }
}

/// Whether `err` (of a [`Fail`] built with `text`) carries the text: a
/// callback error keeps it in `source()`, the others in their message.
pub(crate) fn carries(err: &Error, text: &str) -> bool {
    use std::error::Error as _;
    match err.kind() {
        ErrorKind::Callback => err.source().is_some_and(|s| s.to_string() == text),
        _ => err.to_string().contains(text),
    }
}

/// A failure to inject: how, and with what text.
pub(crate) type Failure = Option<(Fail, &'static str)>;
/// Runs inside `call`, before its result; an `Err` fails the call.
pub(crate) type Hook = Arc<dyn Fn(&str, &[Symbol]) -> Result<()> + Send + Sync>;
/// A script `main`.
pub(crate) type MainFn = Arc<dyn Fn(&mut ScopedControl<'_>) -> Result<()> + Send + Sync>;

/// The shared, mutable side of a probe: what the test sets and reads.
pub(crate) struct State {
    prefix: &'static str,
    events: Mutex<Vec<Event>>,
    threads: Mutex<Vec<ThreadId>>,
    exec: Mutex<Failure>,
    callable: Mutex<Failure>,
    call: Mutex<Failure>,
    hook: Mutex<Option<Hook>>,
    main: Mutex<Option<MainFn>>,
    want_main: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn span_text(span: &Span) -> String {
    format!(
        "{}:{}:{}-{}:{}",
        span.begin_file(),
        span.begin_line(),
        span.begin_column(),
        span.end_line(),
        span.end_column()
    )
}

impl State {
    /// The events recorded so far, in order; empties the record.
    pub(crate) fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *lock(&self.events))
    }

    /// The threads that ran a callback since the last call; empties the record.
    pub(crate) fn take_threads(&self) -> Vec<ThreadId> {
        std::mem::take(&mut *lock(&self.threads))
    }

    pub(crate) fn fail_exec(&self, failure: Failure) {
        *lock(&self.exec) = failure;
    }

    pub(crate) fn fail_callable(&self, failure: Failure) {
        *lock(&self.callable) = failure;
    }

    pub(crate) fn fail_call(&self, failure: Failure) {
        *lock(&self.call) = failure;
    }

    pub(crate) fn set_hook(&self, hook: Option<Hook>) {
        *lock(&self.hook) = hook;
    }

    /// The script `main`, and whether `callable("main")` says true.
    pub(crate) fn set_main(&self, main: Option<MainFn>) {
        *lock(&self.main) = main;
    }

    pub(crate) fn want_main(&self, on: bool) {
        self.want_main.store(on, Ordering::SeqCst);
    }

    /// Clears every injected failure, the hook and the main switch.
    pub(crate) fn reset(&self) {
        self.fail_exec(None);
        self.fail_callable(None);
        self.fail_call(None);
        self.set_hook(None);
        self.set_main(None);
        self.want_main(false);
    }

    fn note(&self, event: Event) {
        lock(&self.events).push(event);
        lock(&self.threads).push(thread::current().id());
    }

    fn maybe_fail(failure: &Mutex<Failure>) -> Result<()> {
        let failure = *lock(failure);
        match failure {
            Some((fail, text)) => Err(fail.act(text)),
            None => Ok(()),
        }
    }
}

/// The language registered with `clingox::script::register`; its state stays
/// with the test.
pub(crate) struct Probe(pub(crate) Arc<State>);

impl Probe {
    /// A probe answering `callable` for names starting with `prefix`, and the
    /// handle to its state.
    pub(crate) fn new(prefix: &'static str) -> (Probe, Arc<State>) {
        let state = Arc::new(State {
            prefix,
            events: Mutex::new(Vec::new()),
            threads: Mutex::new(Vec::new()),
            exec: Mutex::new(None),
            callable: Mutex::new(None),
            call: Mutex::new(None),
            hook: Mutex::new(None),
            main: Mutex::new(None),
            want_main: AtomicBool::new(false),
        });
        (Probe(Arc::clone(&state)), state)
    }
}

impl Script for Probe {
    fn execute(&self, span: &Span, code: &str) -> Result<()> {
        self.0.note(Event::exec(code, &span_text(span)));
        State::maybe_fail(&self.0.exec)
    }

    fn callable(&self, name: &str) -> Result<bool> {
        self.0.note(Event::callable(name));
        if name == "main" {
            return Ok(self.0.want_main.load(Ordering::SeqCst));
        }
        if !name.starts_with(self.0.prefix) {
            return Ok(false);
        }
        State::maybe_fail(&self.0.callable)?;
        Ok(true)
    }

    fn call(&self, span: &Span, name: &str, arguments: &[Symbol]) -> Result<Vec<Symbol>> {
        self.0.note(Event::Call {
            name: name.to_owned(),
            args: arguments.iter().map(ToString::to_string).collect(),
            span: span_text(span),
        });
        let hook = lock(&self.0.hook).clone();
        if let Some(hook) = hook {
            hook(name, arguments)?;
        }
        State::maybe_fail(&self.0.call)?;
        match name.strip_prefix(self.0.prefix) {
            Some("f" | "g") => Ok(vec![Symbol::number(42)]),
            Some("pool") => Ok((1..=3).map(Symbol::number).collect()),
            Some("none") => Ok(Vec::new()),
            Some("echo") => Ok(arguments.iter().rev().copied().collect()),
            _ => Err(Error::new(
                ErrorKind::InvalidInput,
                format!("the probe has no function `{name}`"),
            )),
        }
    }

    fn main(&self, control: &mut ScopedControl<'_>) -> Result<()> {
        self.0.note(Event::Main);
        let main = lock(&self.0.main).clone();
        match main {
            Some(main) => main(control),
            None => Ok(()),
        }
    }
}

//! Forwarding clingo's messages to the `log` crate (feature `log`, on by
//! default), which `Control`'s rustdoc promises and
//! mutation testing found untested.
//!
//! The `log` crate has one logger per process, so this file installs one for
//! its whole test binary. clingo logs on the thread that calls it during
//! `add` and `ground`, so each test reads the records of its own thread.
//!
//! The messages were checked against the Python module `clingo` 5.8.2: `q :-
//! r.` logs `<block>:1:6-7: info: atom does not occur in any rule head:\n  r`
//! with code `AtomUndefined` while grounding.

#![forbid(unsafe_code)]
#![cfg(feature = "log")]

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;

use clingox::{Control, ErrorKind, Part};

thread_local! {
    static RECORDS: RefCell<Vec<(log::Level, String, String)>> = const { RefCell::new(Vec::new()) };
    static PANIC: Cell<bool> = const { Cell::new(false) };
}

struct Recorder;

impl log::Log for Recorder {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        RECORDS.with_borrow_mut(|records| {
            records.push((
                record.level(),
                record.target().to_owned(),
                record.args().to_string(),
            ));
        });
        assert!(!PANIC.get(), "the log backend failed");
    }

    fn flush(&self) {}
}

static RECORDER: Recorder = Recorder;
static INSTALL: Once = Once::new();

/// Installs the recorder and clears this thread's records.
fn recording() {
    INSTALL.call_once(|| {
        log::set_logger(&RECORDER).expect("no other logger is installed in this binary");
        log::set_max_level(log::LevelFilter::Trace);
    });
    RECORDS.with_borrow_mut(Vec::clear);
    PANIC.set(false);
}

fn records() -> Vec<(log::Level, String, String)> {
    RECORDS.with_borrow(Clone::clone)
}

#[test]
fn warnings_go_to_the_log_crate_at_warn_under_target_clingox() {
    recording();
    let mut ctl = Control::new().unwrap();
    ctl.add_base("q :- r.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let records = records();
    assert_eq!(records.len(), 1, "{records:?}");
    let (level, target, text) = &records[0];
    assert_eq!(*level, log::Level::Warn);
    assert_eq!(target, "clingox");
    assert!(
        text.starts_with("<block>:1:6-7: info: atom does not occur"),
        "{text}"
    );
}

#[test]
fn errors_go_to_the_log_crate_at_error() {
    recording();
    let mut ctl = Control::new().unwrap();
    let err = ctl.add_base("a :- b c.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    let records = records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].0, log::Level::Error);
    assert_eq!(records[0].1, "clingox");
    assert!(records[0].2.contains("syntax error"), "{:?}", records[0]);
    // The error keeps its own copy of the message.
    assert_eq!(err.messages()[0].text(), records[0].2);
}

#[test]
fn a_user_logger_replaces_the_log_crate() {
    recording();
    let mut ctl = Control::builder().logger(|_, _| {}).build().unwrap();
    ctl.add_base("q :- r.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(records().is_empty(), "{:?}", records());
}

#[test]
fn a_panicking_log_backend_resumes_on_the_caller() {
    recording();
    let mut ctl = Control::new().unwrap();
    ctl.add_base("q :- r. s :- t.").unwrap();
    PANIC.set(true);
    let caught = catch_unwind(AssertUnwindSafe(|| ctl.ground(&[Part::base()])));
    PANIC.set(false);
    let payload = caught.expect_err("the panic reaches the caller of ground");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"the log backend failed")
    );
    // Only the first message reached the backend: after a panic nothing more
    // is forwarded until it has resumed (DESIGN S9).
    assert_eq!(records().len(), 1, "{:?}", records());

    // Not poisoned: grounding had finished, and the control goes on, with the
    // backend called again.
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(ctl.solve(&[]).unwrap().is_sat());
    // Python clingo 5.8.2 logs `operation undefined` for this in a later step.
    ctl.add("more", &[], "p(1/0).").unwrap();
    ctl.ground(&[Part::new("more", &[]).unwrap()]).unwrap();
    assert_eq!(records().len(), 2, "{:?}", records());
}

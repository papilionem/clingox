//! Configuring a control with `Control::builder()`: arguments, the user logger
//! and the message limit.
//!
//! Message texts and counts were checked against the Python module `clingo`
//! 5.8.2, which takes the same logger and message limit (`Control(args, logger,
//! limit)`).

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use clingox::{Control, ControlBuilder, ErrorKind, MessageCode, Part};

type Log = Arc<Mutex<Vec<(MessageCode, String)>>>;

/// A builder whose logger records every message, and the record.
fn recording() -> (ControlBuilder, Log) {
    let log: Log = Arc::default();
    let sink = Arc::clone(&log);
    let builder = Control::builder().logger(move |code, text: &str| {
        sink.lock()
            .expect("no test panics while holding the log")
            .push((code, text.to_owned()));
    });
    (builder, log)
}

fn entries(log: &Log) -> Vec<(MessageCode, String)> {
    log.lock()
        .expect("no test panics while holding the log")
        .clone()
}

/// `n` rules whose bodies use atoms that occur in no head: grounding logs one
/// `AtomUndefined` message for each.
fn undefined_atoms(n: usize) -> String {
    (0..n)
        .map(|i| format!("a{i} :- b{i}."))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_builder_with_nothing_set_builds_a_default_control() {
    let mut ctl = Control::builder().build().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn the_logger_receives_clingo_messages() {
    let (builder, log) = recording();
    let mut ctl = builder.build().unwrap();
    ctl.add_base("a :- b.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    // The text is clingo's, with its position prefix, trailing newline removed.
    assert_eq!(
        entries(&log),
        vec![(
            MessageCode::AtomUndefined,
            "<block>:1:6-7: info: atom does not occur in any rule head:\n  b".to_owned()
        )]
    );
}

#[test]
fn the_logger_receives_each_kind_of_message_with_its_code() {
    let (builder, log) = recording();
    let mut ctl = builder.build().unwrap();
    ctl.add_base("p(1/0).").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let entries = entries(&log);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].0, MessageCode::OperationUndefined);
    assert!(
        entries[0].1.contains("operation undefined"),
        "{}",
        entries[0].1
    );
}

#[test]
fn a_parse_error_keeps_its_messages_with_a_user_logger() {
    let (builder, log) = recording();
    let mut ctl = builder.build().unwrap();
    let err = ctl.add_base("a :- b c.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);

    let expected = "<block>:1:8-9: error: syntax error, unexpected <IDENTIFIER>";
    assert_eq!(
        entries(&log),
        vec![(MessageCode::RuntimeError, expected.to_owned())]
    );
    // The logger does not take the messages away from the error.
    let texts: Vec<&str> = err.messages().iter().map(clingox::Message::text).collect();
    assert_eq!(texts, vec![expected]);
}

#[test]
fn the_message_limit_caps_the_messages_passed_to_the_logger() {
    let (builder, log) = recording();
    let mut ctl = builder.message_limit(1).build().unwrap();
    ctl.add_base(&undefined_atoms(3)).unwrap();
    // Too many warnings are dropped, not an error.
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(entries(&log).len(), 1);
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn the_default_message_limit_is_twenty() {
    let (builder, log) = recording();
    let mut ctl = builder.build().unwrap();
    ctl.add_base(&undefined_atoms(25)).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(entries(&log).len(), 20);
}

#[test]
fn a_message_limit_of_zero_silences_warnings_but_not_errors() {
    let (builder, log) = recording();
    let mut ctl = builder.message_limit(0).build().unwrap();
    ctl.add_base(&undefined_atoms(3)).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(entries(&log).is_empty(), "{:?}", entries(&log));

    let (builder, log) = recording();
    let mut ctl = builder.message_limit(0).build().unwrap();
    let err = ctl.add_base("a :- b c.").unwrap_err();
    assert_eq!(entries(&log).len(), 1, "clingo still passes the error on");
    assert_eq!(err.messages().len(), 1, "and it is still captured");
}

#[test]
fn a_panic_in_the_logger_resumes_on_the_caller() {
    let mut ctl = Control::builder()
        .logger(|_, _| panic!("logger failed"))
        .build()
        .unwrap();
    ctl.add_base("a :- b.").unwrap();
    let caught =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.ground(&[Part::base()])));
    let payload = caught.expect_err("the panic reaches the caller of ground");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"logger failed"));

    // Not poisoned: grounding had finished, and the control goes on.
    assert!(!format!("{ctl:?}").contains("poisoned"));
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn the_control_owns_the_logger_and_drops_it_with_itself() {
    let (builder, log) = recording();
    let ctl = builder.build().unwrap();
    assert_eq!(Arc::strong_count(&log), 2, "the control holds the logger");
    drop(ctl);
    assert_eq!(Arc::strong_count(&log), 1, "dropping the control drops it");
}

#[test]
fn builder_arguments_are_checked_like_with_args() {
    let err = Control::builder()
        .args(["--no-such-option"])
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Logic);
    assert!(err.to_string().contains("no-such-option"), "{err}");

    let err = Control::builder()
        .args(["--models=0\0"])
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
}

#[test]
fn builder_arguments_accept_any_string_form() {
    Control::builder()
        .args(vec![String::from("--models=0")])
        .args(["--opt-mode=optN"])
        .build()
        .unwrap();
}

#[test]
fn builders_are_debug_and_default() {
    fn builder_traits<T: Default + std::fmt::Debug>() {}
    builder_traits::<ControlBuilder>();

    let text = format!("{:?}", Control::builder().args(["--models=0"]));
    assert!(text.contains("ControlBuilder"), "{text}");
    assert!(text.contains("--models=0"), "{text}");
    let text = format!("{:?}", Control::builder().logger(|_, _| {}));
    assert!(text.contains("logger"), "{text}");
}

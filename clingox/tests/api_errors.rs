//! The error type: `Error::conversion`,
//! `ErrorKind::InvalidInput`, the `Display` of callback errors, and the
//! message and location details that mutation testing found untested.
//!
//! Every message text and position below was checked against the Python module
//! `clingo` 5.8.2 with a logger that records each message and its code.

#![forbid(unsafe_code)]

use std::error::Error as _;

use clingox::{Control, Error, ErrorKind, Message, MessageCode, Part, Symbol};

#[derive(Debug)]
struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("refused by the user")
    }
}

impl std::error::Error for Refused {}

fn is_poisoned(ctl: &Control) -> bool {
    format!("{ctl:?}").contains("poisoned")
}

// ---------------------------------------------------------------------------
// Error::conversion

#[test]
fn conversion_errors_can_be_built_by_users() {
    let err = Error::conversion("expected a colour, found `p(1)`");
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert_eq!(err.to_string(), "expected a colour, found `p(1)`");
    assert_eq!(err.messages(), []);
    assert!(err.source().is_none(), "a conversion error has no source");
}

#[test]
fn a_conversion_error_reads_as_one_line_without_a_trailing_period() {
    let err = Error::conversion(String::from("first line.\nsecond line."));
    assert_eq!(err.to_string(), "first line.; second line");
}

// ---------------------------------------------------------------------------
// ErrorKind::InvalidInput

#[test]
fn invalid_input_reads_as_invalid_input() {
    assert_eq!(ErrorKind::InvalidInput.to_string(), "invalid input");
    assert_ne!(ErrorKind::InvalidInput, ErrorKind::Runtime);
}

#[test]
fn a_reserved_part_name_is_invalid_input_and_does_not_poison() {
    let mut ctl = Control::new().unwrap();
    let err = ctl.add("__clingox_facts_0", &[], "a.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(err.to_string().contains("reserved"), "{err}");
    assert!(!is_poisoned(&ctl));

    let err = Part::new("__clingox_facts_7", &[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);

    // The control is still usable.
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
fn a_thread_count_outside_1_to_64_is_invalid_input() {
    // clingo accepts 1 to 64 solver threads (`--parallel-mode`); clingox
    // checks the typed shortcut itself, on every build, before calling clingo.
    for n in [0, 65, u32::MAX] {
        let err = Control::builder().threads(n).build().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "threads({n})");
        assert!(err.to_string().contains(&n.to_string()), "{err}");
    }
}

#[test]
fn a_nul_byte_is_still_nul_and_not_invalid_input() {
    let mut ctl = Control::new().unwrap();
    let err = ctl.add("ba\0se", &[], "a.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
}

// ---------------------------------------------------------------------------
// Error::callback: Display shows the context only

#[test]
fn a_callback_error_does_not_repeat_its_source_in_display() {
    let err = Error::callback(Refused);
    assert_eq!(err.kind(), ErrorKind::Callback);
    assert_eq!(err.to_string(), "the callback failed");
    let source = err.source().expect("the user error is the source");
    assert_eq!(source.to_string(), "refused by the user");
    assert!(source.downcast_ref::<Refused>().is_some());
}

#[test]
fn walking_the_chain_of_a_callback_error_shows_the_user_text_once() {
    // How `anyhow` and `eyre` print an error: each level's Display, then its
    // source's, until there is none.
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(@f()).").unwrap();
    let err = ctl
        .ground_with(&[Part::base()], |_| Err(Error::callback(Refused)))
        .unwrap_err();
    let mut chain = Vec::new();
    let mut level: Option<&dyn std::error::Error> = Some(&err);
    while let Some(e) = level {
        chain.push(e.to_string());
        level = e.source();
    }
    assert_eq!(
        chain,
        [
            "grounding `base`: the callback failed",
            "refused by the user"
        ]
    );
}

#[test]
fn a_callback_error_from_a_model_closure_keeps_its_source() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let err = ctl
        .for_each_model(&[], |_| Err(Error::callback(Refused)))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
    assert!(!err.to_string().contains("refused by the user"), "{err}");
    assert!(err.source().unwrap().downcast_ref::<Refused>().is_some());
}

#[test]
fn debug_of_a_callback_error_still_shows_the_source() {
    let text = format!("{:?}", Error::callback(Refused));
    assert!(
        text.contains("Callback") && text.contains("Refused"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// Messages and locations

#[test]
fn a_message_on_a_later_line_reports_that_line() {
    // Python clingo 5.8.2: `<block>:3:8-9: error: syntax error, unexpected
    // <IDENTIFIER>`.
    let mut ctl = Control::new().unwrap();
    let err = ctl.add_base("a.\nb.\nc :- d e.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    let location = err.messages()[0].location().expect("a position");
    assert_eq!(
        (location.file(), location.line(), location.column()),
        ("<block>", 3, 8)
    );
}

#[test]
fn display_shows_the_first_error_not_an_earlier_message() {
    // Python clingo 5.8.2 logs, while grounding this program, an info message
    // for `1/0` on line 1 and then the error for the unsafe `Z` on line 2.
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- 1/0 = 1.\nb(Z).").unwrap();
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
    let codes: Vec<MessageCode> = err.messages().iter().map(Message::code).collect();
    assert_eq!(
        codes,
        [MessageCode::OperationUndefined, MessageCode::RuntimeError]
    );
    let text = err.to_string();
    assert!(text.contains("unsafe variables"), "{text}");
    assert!(!text.contains("operation undefined"), "{text}");
    assert_eq!(
        err.messages()[1].location().map(|l| (l.line(), l.column())),
        Some((2, 1))
    );
}

#[test]
fn a_term_error_clingo_does_not_log_still_has_a_message() {
    // clingo's term parser logs nothing for these; Python clingo 5.8.2 raises
    // `RuntimeError: parsing failed` for `1/0`, and the position-prefixed
    // syntax error as the exception text for `p(` (`<string>:2:2: error:
    // syntax error, unexpected <EOF>, expecting )`: the end of the input counts
    // as a second line).
    let err = "1/0".parse::<Symbol>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    assert_eq!(err.messages().len(), 1);
    assert_eq!(err.messages()[0].text(), "parsing failed");
    assert_eq!(err.messages()[0].code(), MessageCode::RuntimeError);
    assert!(err.to_string().ends_with("parsing failed"), "{err}");

    let err = "p(".parse::<Symbol>().unwrap_err();
    assert_eq!(err.messages().len(), 1);
    let message = &err.messages()[0];
    assert!(message.text().contains("unexpected"), "{}", message.text());
    let location = message.location().expect("the syntax error has a position");
    assert_eq!(
        (location.file(), location.line(), location.column()),
        ("<string>", 2, 2)
    );
}

#[test]
fn messages_and_locations_display_as_clingo_wrote_them() {
    let mut ctl = Control::new().unwrap();
    let err = ctl.add_base("a.\nb :- c d.").unwrap_err();
    let message = &err.messages()[0];
    assert_eq!(
        message.to_string(),
        "<block>:2:8-9: error: syntax error, unexpected <IDENTIFIER>"
    );
    assert_eq!(message.to_string(), message.text());
    let location = message.location().unwrap();
    assert_eq!(location.to_string(), "<block>:2:8");
}

/// File names may contain `:` and `-`, so a prefix that looks like a location
/// followed by `-` is only the start of one if what follows is a valid end.
/// This needs a file, reached with `#include`. Windows does not allow `:` in a
/// file name, so the case cannot be built there.
#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm", windows)))]
fn a_file_name_with_colons_and_dashes_is_read_whole() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("api_errors_g3");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a:1:2-b.lp");
    std::fs::write(&file, "q(1).\n\nq(2) :- a b.\n").unwrap();
    let file = file.to_str().expect("the target directory is UTF-8");

    // Python clingo 5.8.2: `<path>/a:1:2-b.lp:3:11-12: error: syntax error,
    // unexpected <IDENTIFIER>`.
    let mut ctl = Control::new().unwrap();
    let err = ctl.add_base(&format!("#include \"{file}\".")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    let location = err.messages()[0].location().expect("a position");
    assert_eq!(location.file(), file);
    assert_eq!((location.line(), location.column()), (3, 11));
}

// ---------------------------------------------------------------------------
// ErrorKind::Interrupted is gone; `tests/ui/error_kind_interrupted.rs`
// checks that it no longer compiles. Interrupts are results, never errors:

#[test]
fn an_interrupted_search_is_a_result_not_an_error() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let result = ctl
        .for_each_model(&[], |_| Ok(std::ops::ControlFlow::Break(())))
        .unwrap();
    assert!(result.is_interrupted());
}

//! `clingox::ast::parse_string`/`parse_files`: statement
//! order, the parse callback's error/panic handling (S8), the refcount
//! contract on the node the callback receives, and the syntax-error kind.
//!
//! Checked against the Python clingo 5.8.2 oracle (`clingo.ast`, and
//! `clingo._internal._lib`/`_ffi` directly for the C-level error code behind
//! a syntax error and a missing file). clingo delivers files in reverse
//! order, which `parse_files_delivers_files_in_reverse_order` below depends on.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

#[cfg(not(any(target_os = "android", target_family = "wasm")))]
use std::path::Path;

#[cfg(not(any(target_os = "android", target_family = "wasm")))]
use clingox::ast::Attribute;
use clingox::ast::{self, Ast, AstType};
use clingox::{Error, ErrorKind};

/// A directory under the test binary's own scratch space (mirrors
/// `api_control_load.rs::scratch_dir`).
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn write_fixture(dir: &Path, name: &str, contents: &[u8]) -> std::path::PathBuf {
    let file = dir.join(name);
    std::fs::write(&file, contents).unwrap();
    file
}

// ---------------------------------------------------------------------------
// parse_string: ordering, Display
// ---------------------------------------------------------------------------

#[test]
fn parse_string_delivers_every_statement_in_source_order() {
    let mut seen = Vec::new();
    ast::parse_string("a.\nb :- a.\nc.\n", |node| {
        seen.push((node.ast_type(), node.to_string()));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        seen,
        [
            (AstType::Program, "#program base.".to_owned()),
            (AstType::Rule, "a.".to_owned()),
            (AstType::Rule, "b :- a.".to_owned()),
            (AstType::Rule, "c.".to_owned()),
        ]
    );
}

// ---------------------------------------------------------------------------
// The callback's error and panic handling (S8)
// ---------------------------------------------------------------------------

#[test]
fn an_error_from_the_callback_is_returned_with_its_own_kind_and_later_calls_are_skipped() {
    let mut calls = 0;
    let err = ast::parse_string("a.\nb.\nc.\n", |_node| {
        calls += 1;
        Err(Error::new(ErrorKind::Conversion, "stop here"))
    })
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert_eq!(calls, 1, "the first failure stops every later call (S8)");
}

#[test]
fn a_panic_in_the_callback_resumes_on_the_caller() {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ast::parse_string("a.\nb.\n", |_node| panic!("stop here"))
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"stop here"));
}

// ---------------------------------------------------------------------------
// Refcounting: a node stored past the callback's return stays valid (ASan/LSan)
// ---------------------------------------------------------------------------

#[test]
fn nodes_collected_in_the_callback_stay_valid_after_parse_string_returns() {
    // Run under `cargo xtask sanitize` (ASan, LSan): a missing
    // `clingo_ast_acquire` in the trampoline shows up here as a
    // use-after-free (ASan abort) the first time a stored node is touched
    // below, since the parser has already moved on to its next node or
    // finished by the time this runs.
    let mut collected: Vec<Ast> = Vec::new();
    ast::parse_string("a.\nb :- a.\nc.\n", |node| {
        collected.push(node);
        Ok(())
    })
    .unwrap();
    assert_eq!(collected.len(), 4); // the implicit `#program base.` plus 3 rules
    let texts: Vec<String> = collected.iter().map(ToString::to_string).collect();
    assert_eq!(
        texts,
        ["#program base.", "a.", "b :- a.", "c."].map(str::to_owned)
    );
    // Every node is still independently comparable and clonable.
    let clones: Vec<Ast> = collected.clone();
    assert_eq!(clones, collected);
    drop(collected);
    for clone in &clones {
        let _ = clone.to_string();
    }
}

// ---------------------------------------------------------------------------
// Syntax errors: kind, captured messages, location
// ---------------------------------------------------------------------------

#[test]
fn a_syntax_error_is_reported_as_parse_with_its_position() {
    // Checked live at the C level: `clingo_ast_parse_string` on "a :- b c."
    // raises `clingo_error_code() == 1` (`clingo_error_runtime`) with the
    // message "syntax error" and one logged message
    // "<string>:1:8-9: error: syntax error, unexpected <IDENTIFIER>". This
    // is the same runtime code `Control::add`'s own parser failure reports
    // before clingox remaps it (`control.rs:527-530`); `ast::parse_string`
    // performs the identical remap for the identical reason.
    let err = ast::parse_string("a :- b c.\n", |_node| Ok(())).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    assert!(!err.messages().is_empty(), "{err:?}");
    let location = err.messages()[0]
        .location()
        .expect("the message has a position");
    assert_eq!((location.line(), location.column()), (1, 8));
    assert!(err.messages()[0].text().contains("syntax error"), "{err:?}");
}

// ---------------------------------------------------------------------------
// parse_files
// ---------------------------------------------------------------------------

#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn parse_files_delivers_files_in_reverse_order() {
    // Checked live, and surprising: `clingo_ast_parse_files` visits the
    // files it is given from last to first (`Input::NonGroundParser::
    // pushFile` stacks its streams and `parse` pops them, control.cc:1859-
    // 1864), not in the order the caller listed them, and each file gets
    // its own leading `#program base.` node. A test that assumed forward
    // order would be wrong against real clingo, not merely against a guess.
    let dir = scratch_dir("api_ast_parse_files_order");
    let f1 = write_fixture(&dir, "f1.lp", b"a.\n");
    let f2 = write_fixture(&dir, "f2.lp", b"b.\n");

    let mut seen = Vec::new();
    ast::parse_files(&[&f1, &f2], |node| {
        seen.push(node.to_string());
        Ok(())
    })
    .unwrap();
    assert_eq!(
        seen,
        ["#program base.", "b.", "#program base.", "a."].map(str::to_owned),
        "f2 is delivered whole before f1"
    );
}

/// a span whose file name is not UTF-8 is
/// `ErrorKind::Utf8`, never a lossy name that matches no file. An `#include`
/// of `inc_\xff.lp` makes the included statements carry that name.
#[test]
#[cfg(all(
    unix,
    not(any(
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_family = "wasm"
    ))
))]
fn a_span_file_name_that_is_not_utf8_is_a_utf8_error() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let dir = scratch_dir("api_ast_parse_files_non_utf8_include");
    let included = dir.join(OsStr::from_bytes(b"inc_\xff.lp"));
    if std::fs::write(&included, b"inner.\n").is_err() {
        #[allow(clippy::print_stderr, reason = "a visible skip")]
        {
            eprintln!("SKIPPED: this file system refuses a name that is not UTF-8");
        }
        return;
    }
    let main = write_fixture(&dir, "main.lp", b"outer.\n#include \"inc_\xff.lp\".\n");

    let mut inner = Vec::new();
    let mut outer = Vec::new();
    ast::parse_files(&[&main], |node| {
        if node.ast_type() == AstType::Rule {
            let located = node.span(Attribute::Location);
            match node.to_string().as_str() {
                "inner." => inner.push(located),
                _ => outer.push(located),
            }
        }
        Ok(())
    })
    .unwrap();

    assert_eq!(inner.len(), 1, "the included statement is delivered");
    let err = inner.pop().unwrap().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Utf8, "{err}");
    // A file with a UTF-8 name is unaffected.
    assert_eq!(outer.len(), 1);
    let span = outer.pop().unwrap().unwrap();
    assert!(
        span.begin_file().ends_with("main.lp"),
        "{}",
        span.begin_file()
    );
}

#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn parse_files_on_a_missing_file_is_a_parse_error_with_no_location() {
    let missing = scratch_dir("api_ast_parse_files_missing").join("does_not_exist.lp");
    let err = ast::parse_files(&[&missing], |_node| Ok(())).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    assert!(!err.messages().is_empty(), "{err:?}");
    let message = &err.messages()[0];
    assert!(message.text().contains("could not be opened"), "{err:?}");
    // The message's own prefix ("<cmd>: error: ...") is not a
    // `file:line:column` position, so it has none.
    assert!(message.location().is_none(), "{err:?}");
}

#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn a_non_utf8_string_attribute_read_from_a_parsed_file_is_a_utf8_error() {
    // The program text itself is always valid UTF-8 by construction
    // (`parse_files` takes `&[impl AsRef<Path>]`, and clingox never builds
    // an invalid `&str`); what can still be non-UTF-8 is a string clingo
    // reads back out of the file's own raw bytes, which the parser does not
    // validate as UTF-8 (checked live: `clingo_ast_parse_string` accepts
    // this exact byte sequence and clingo_symbol_string/
    // clingo_ast_attribute_get_string both hand back the invalid bytes
    // unchanged). A `#script` block's own body is captured verbatim, with
    // no lexical restriction, so it is the simplest fixture for a `string`
    // (not `symbol`) attribute.
    let dir = scratch_dir("api_ast_parse_non_utf8");
    let file = write_fixture(&dir, "bad_utf8.lp", b"#script (python)\n# a\xffb\n#end.\n");

    let mut script = None;
    ast::parse_files(&[&file], |node| {
        if node.ast_type() == AstType::Script {
            script = Some(node);
        }
        Ok(())
    })
    .unwrap();
    let script = script.expect("the file has a #script block");
    assert_eq!(script.string(Attribute::Name).unwrap(), "python");
    let err = script.string(Attribute::Code).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Utf8);
}

/// Reads a `#script` block whose code holds a stray `\xff` byte through
/// `parse_files`, the only way to get non-UTF-8 text into a node.
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn script_with_invalid_utf8(dir_name: &str) -> Ast {
    let dir = scratch_dir(dir_name);
    let file = write_fixture(&dir, "bad_utf8.lp", b"#script (python)\n# a\xffb\n#end.\n");
    let mut script = None;
    ast::parse_files(&[&file], |node| {
        if node.ast_type() == AstType::Script {
            script = Some(node);
        }
        Ok(())
    })
    .unwrap();
    script.expect("the file has a #script block")
}

#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn display_of_a_node_with_non_utf8_text_does_not_panic_and_says_why() {
    // A panic was reproduced here: `to_string()` on this node used
    // to fail with "a Display implementation returned an error
    // unexpectedly". `Display` must never fail; it
    // writes a placeholder instead.
    let script = script_with_invalid_utf8("api_ast_display_non_utf8");
    let text = script.to_string();
    assert!(text.contains("<error:"), "{text}");
    assert_eq!(format!("{script}"), text);
    // Debug never reads the text, so it is unaffected.
    assert_eq!(format!("{script:?}"), "Ast(Script, 3 attributes)");
}

#[test]
#[cfg(not(any(target_os = "android", target_family = "wasm")))]
fn try_to_string_reports_non_utf8_text_as_utf8_and_matches_display_otherwise() {
    let script = script_with_invalid_utf8("api_ast_try_to_string_non_utf8");
    assert_eq!(script.try_to_string().unwrap_err().kind(), ErrorKind::Utf8);

    // For an ordinary node the fallible form is the same text `Display`
    // prints.
    let mut nodes = Vec::new();
    ast::parse_string("q(X) :- p(X), X > 1.\n", |node| {
        nodes.push(node);
        Ok(())
    })
    .unwrap();
    for node in &nodes {
        assert_eq!(node.try_to_string().unwrap(), node.to_string());
    }
    assert_eq!(nodes[1].try_to_string().unwrap(), "q(X) :- p(X); X > 1.");
}

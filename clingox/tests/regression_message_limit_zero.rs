//! `Application::message_limit(0)`
//! does not silence errors. clingo passes errors on whatever the limit
//! (`Logger::check`, `libgringo/gringo/logger.hh`), as
//! `ControlBuilder::message_limit` documents. This pins that a syntax error
//! still reaches the application's logger with limits of 0, 1 and 20.

#![allow(clippy::unwrap_used, reason = "test")]

use std::sync::Mutex;

use clingox::application::Application;

#[path = "common/child.rs"]
mod child;

fn errors_seen(limit: u32) -> Vec<String> {
    let path = child::fixture(&format!("limit{limit}.lp"), "a :- b c.");
    let seen = Mutex::new(Vec::new());
    let code = Application::new()
        .message_limit(limit)
        .logger(|_code, text| seen.lock().unwrap().push(text.to_owned()))
        .run([child::arg(&path).as_str(), "--outf=3"])
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_ne!(code, 0, "limit {limit}: a syntax error is not a success");
    seen.into_inner().unwrap()
}

#[test]
fn a_limit_of_zero_still_delivers_errors() {
    for limit in [20, 1, 0] {
        let seen = errors_seen(limit);
        assert!(
            seen.iter().any(|m| m.contains("syntax error")),
            "limit {limit}: the syntax error must reach the logger: {seen:?}"
        );
    }
}

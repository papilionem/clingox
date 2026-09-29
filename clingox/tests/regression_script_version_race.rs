//! `script::version` read clingo's
//! script registry without the lock `script::register` holds, so a
//! registration on another thread could reallocate the vector while `version`
//! walked it (SIGSEGV within 0.2 s, `ThreadSanitizer` confirms). `version` now
//! takes the registry lock. The race runs in a child process because a
//! regression would end the process, with two reader threads against 8 000
//! registrations (bounded by the child's time limit). The child never creates
//! a `Control`, which would freeze the registry.

#![allow(clippy::unwrap_used, clippy::print_stdout, reason = "test")]

#[path = "common/child.rs"]
mod child;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use clingox::ast::Span;
use clingox::script::{self, Script};

const ENTRY: &str = "child_entry";

struct Quiet;

impl Script for Quiet {
    fn execute(&self, _span: &Span, _code: &str) -> clingox::Result<()> {
        Ok(())
    }
}

#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    let readers_count = match case.as_str() {
        "race" => 2,
        "alone" => 0,
        _ => unreachable!(),
    };
    let registered = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let wrong = Arc::new(AtomicU64::new(0));
    let readers: Vec<_> = (0..readers_count)
        .map(|_| {
            let (registered, stop, wrong) = (registered.clone(), stop.clone(), wrong.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    // A hit at index 0 dereferences the element's script, a
                    // miss walks the whole vector.
                    let first = script::version("s0");
                    if registered.load(Ordering::Acquire) && first.as_deref() != Some("v0") {
                        wrong.fetch_add(1, Ordering::Relaxed);
                    }
                    let _ = script::version("missing");
                }
            })
        })
        .collect();
    script::register("s0", "v0", Quiet).unwrap();
    registered.store(true, Ordering::Release);
    for i in 1..8_000 {
        script::register(&format!("s{i}"), "v", Quiet).unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for reader in readers {
        reader.join().unwrap();
    }
    println!("CHILD-DONE wrong={}", wrong.load(Ordering::Relaxed));
}

/// Control: the same registrations with no concurrent reader never crash.
#[test]
fn registration_alone_is_fine() {
    let Some(outcome) = child::run_child_with(ENTRY, "alone", Duration::from_secs(120), None)
    else {
        return;
    };
    assert_eq!(outcome.signal, None, "{outcome:?}");
    assert!(
        outcome.stdout.contains("CHILD-DONE wrong=0"),
        "{}",
        outcome.stdout
    );
}

#[test]
fn version_while_another_thread_registers() {
    let Some(outcome) = child::run_child_with(ENTRY, "race", Duration::from_secs(120), None) else {
        return;
    };
    println!("{outcome:?}");
    assert_eq!(outcome.signal, None, "the child was killed by a signal");
    assert!(
        outcome.stdout.contains("CHILD-DONE wrong=0"),
        "{}",
        outcome.stdout
    );
}

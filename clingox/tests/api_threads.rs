//! Threads: `Control` is `Send` and never `Sync` (item 3),
//! thread support comes from how clingo was built, and
//! `clingox` forwards the features `threads` and `vendored`.
//!
//! The moves below follow an experiment that ran clean under the
//! thread sanitizer with 1, 4 and 8 solver threads. `cargo xtask sanitize`
//! runs this file under the thread sanitizer.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail loudly on unexpected errors"
)]

use std::ops::ControlFlow;
use std::thread;

use clingox::prelude::*;
use clingox::{AsyncSolveHandle, InterruptHandle, SolveOptions};

fn assert_send<T: Send>() {}

#[test]
fn a_control_and_its_handles_are_send() {
    assert_send::<Control>();
    assert_send::<SolveHandle<'static>>();
    assert_send::<AsyncSolveHandle<'static>>();
    assert_send::<InterruptHandle>();
    // `tests/ui/control_is_not_sync.rs` and `tests/ui/model_is_not_send.rs`
    // check the other half: no `Sync` control, and no model leaves its thread.
}

/// Whether this build of clingo has threads.
fn has_threads() -> bool {
    clingox_sys::HAS_THREADS
}

/// Each model's shown symbols as one text, in the order of the models.
fn sorted_texts(models: &[OwnedModel]) -> Vec<String> {
    models
        .iter()
        .map(|m| {
            let texts: Vec<String> = m.symbols().iter().map(ToString::to_string).collect();
            texts.join(" ")
        })
        .collect()
}

/// One step of that experiment: every phase of a control's life on a
/// thread of its own.
fn moved_through_threads(solver_threads: u32) {
    let program = "{a;b;c}. :- a, b.";
    let ctl = thread::spawn(move || {
        Control::builder()
            .threads(solver_threads)
            .args(["--models=0"])
            .build()
            .unwrap()
    })
    .join()
    .unwrap();
    let ctl = thread::spawn(move || {
        let mut ctl = ctl;
        ctl.add_base(program).unwrap();
        ctl
    })
    .join()
    .unwrap();
    let mut ctl = thread::spawn(move || {
        let mut ctl = ctl;
        ctl.ground(&[Part::base()]).unwrap();
        ctl
    })
    .join()
    .unwrap();
    let (result, reference) = ctl.solve_all().unwrap();
    assert!(result.is_exhausted());
    assert_eq!(reference.len(), 6);

    // A yield search whose models are read on alternating threads, and which
    // is closed on yet another one.
    thread::scope(|scope| {
        let mut handle = ctl.solve_yield(&[]).unwrap();
        let mut seen = Vec::new();
        loop {
            let (next, back) = scope
                .spawn(move || {
                    let model = handle.next_model().unwrap().map(|m| m.snapshot().unwrap());
                    (model, handle)
                })
                .join()
                .unwrap();
            handle = back;
            match next {
                Some(model) => seen.push(model),
                None => break,
            }
        }
        let result = scope.spawn(move || handle.close().unwrap()).join().unwrap();
        assert!(result.is_exhausted(), "{result:?}");
        seen.sort_by(|a, b| a.symbols().cmp(b.symbols()));
        assert_eq!(sorted_texts(&seen), sorted_texts(&reference));
    });

    // An async search waited for on one thread and read on another.
    if has_threads() {
        thread::scope(|scope| {
            let handle = ctl.solve_async(&[]).unwrap();
            let handle = scope
                .spawn(move || {
                    let mut handle = handle;
                    assert!(handle.wait(std::time::Duration::from_secs(60)));
                    handle
                })
                .join()
                .unwrap();
            let result = scope
                .spawn(move || {
                    let mut handle = handle;
                    handle.get().unwrap()
                })
                .join()
                .unwrap();
            assert!(result.is_sat());
        });
    }

    // The control is dropped on another thread.
    thread::spawn(move || drop(ctl)).join().unwrap();
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build cannot start threads"
)]
fn a_control_moves_between_threads_in_every_phase() {
    moved_through_threads(1);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build cannot start threads"
)]
fn a_control_with_several_solver_threads_moves_between_threads() {
    if !has_threads() {
        return;
    }
    moved_through_threads(4);
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build cannot start threads"
)]
fn a_control_moved_in_the_middle_of_a_model_loop_goes_on() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut count = 0;
    let stopped = ctl
        .for_each_model(&[], |_| {
            count += 1;
            Ok(ControlFlow::Break(()))
        })
        .unwrap();
    assert!(stopped.is_interrupted(), "Break ends the search early");
    let mut ctl = thread::spawn(move || {
        let mut ctl = ctl;
        let (result, models) = ctl.solve_all().unwrap();
        assert!(result.is_exhausted());
        assert_eq!(models.len(), 4);
        ctl
    })
    .join()
    .unwrap();
    assert_eq!(count, 1);
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

#[test]
#[cfg_attr(
    all(target_family = "wasm", not(target_feature = "atomics")),
    ignore = "the default WASM build cannot start threads"
)]
fn controls_on_many_threads_match_a_single_threaded_reference() {
    // A second experiment: facts in heads and bodies, choice rules and
    // constraints, so that clasp's shared `trueAtom_g` is written by every
    // control. The writes race (a ThreadSanitizer suppression covers exactly
    // that global, DESIGN S12), but they must never change a result.
    const PROGRAMS: [&str; 4] = [
        "a. b :- a. {c}. :- c, not b.",
        "p(1..3). {q(X)} :- p(X). :- q(1), q(2).",
        "n(1..4). e(1,2). e(2,3). e(3,4). col(r;g). 1 {c(N,C) : col(C)} 1 :- n(N). :- e(X,Y), c(X,C), c(Y,C).",
        "x. y :- x, not z. z :- x, not y.",
    ];
    let reference: Vec<Vec<String>> = PROGRAMS
        .iter()
        .map(|program| {
            let mut ctl = Control::new().unwrap();
            ctl.add_base(program).unwrap();
            ctl.ground(&[Part::base()]).unwrap();
            sorted_texts(&ctl.solve_all().unwrap().1)
        })
        .collect();
    thread::scope(|scope| {
        for worker in 0..8 {
            let reference = &reference;
            scope.spawn(move || {
                for round in 0..10 {
                    let index = (worker + round) % PROGRAMS.len();
                    let mut ctl = Control::new().unwrap();
                    ctl.add_base(PROGRAMS[index]).unwrap();
                    ctl.ground(&[Part::base()]).unwrap();
                    let models = sorted_texts(&ctl.solve_all().unwrap().1);
                    assert_eq!(models, reference[index], "program {index}");
                }
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Thread support from the real build, and the forwarded features

#[test]
fn thread_support_is_what_clingo_was_built_with() {
    // The vendored build gives clasp threads when the feature `threads` is on
    // and the target can run threads: everywhere except WebAssembly without
    // the `atomics` target feature (DESIGN 9).
    if clingox_sys::VENDORED {
        let target_can = !cfg!(all(target_family = "wasm", not(target_feature = "atomics")));
        assert_eq!(
            clingox_sys::HAS_THREADS,
            cfg!(feature = "threads") && target_can
        );
    }
}

#[test]
fn the_threads_and_vendored_features_are_forwarded_and_on_by_default() {
    // Cargo features are additive, so clingox must depend on clingox-sys
    // without its defaults and forward both features, or a user could never
    // turn them off.
    let manifest = include_str!("../Cargo.toml");
    let line = |start: &str| {
        manifest
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(start))
            .unwrap_or_else(|| panic!("clingox/Cargo.toml has no line `{start} ...`"))
    };
    let dependency = line("clingox-sys = ");
    assert!(
        dependency.contains("default-features = false"),
        "{dependency}"
    );
    let threads = line("threads = ");
    assert!(threads.contains("\"clingox-sys/threads\""), "{threads}");
    let vendored = line("vendored = ");
    assert!(vendored.contains("\"clingox-sys/vendored\""), "{vendored}");
    let default = line("default = ");
    for feature in ["\"threads\"", "\"vendored\"", "\"log\"", "\"derive\""] {
        assert!(default.contains(feature), "{default}");
    }
}

#[test]
fn several_solver_threads_work_exactly_when_clingo_has_threads() {
    let built = Control::builder().threads(2).build();
    if has_threads() {
        let mut ctl = built.unwrap();
        ctl.add_base("{a;b}.").unwrap();
        ctl.ground(&[Part::base()]).unwrap();
        assert_eq!(ctl.solve_all().unwrap().1.len(), 4);
    } else {
        assert_eq!(built.unwrap_err().kind(), clingox::ErrorKind::Unsupported);
    }
}

#[test]
fn a_timeout_works_exactly_when_clingo_has_threads() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let options = SolveOptions::new().timeout(std::time::Duration::from_secs(60));
    match ctl.solve_with(options) {
        Ok(result) => {
            assert!(has_threads());
            assert!(result.is_sat());
        }
        Err(err) => {
            assert!(!has_threads(), "{err}");
            assert_eq!(err.kind(), clingox::ErrorKind::Unsupported);
        }
    }
}

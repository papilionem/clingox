//! Regression test: an `on_model` that fails during a multi-threaded async
//! search (also the blocking `solve_with_events`, which runs in async mode
//! under clingo whenever `-t` is more than 1) returned `false` to clingo for
//! the model event, clingo's own safe error path for every *other* trampoline
//! (S9) but not for a parallel search: it left clasp's internal state
//! inconsistent, and the next update (here, `release_external`) read out of
//! bounds inside `ClingoControl::cleanup` (`ASan`: heap-buffer-overflow, 3 runs
//! of 3, upstream issue U26). The race is a genuine data/memory-safety bug
//! inside clasp's own C++, which a plain debug build does not reliably crash on
//! -- the sanitizer evidence comes from running this file under `ASan` (`cargo
//! xtask sanitize`; the race cases are `TSan` cases, in their own files). This
//! file itself pins the functional contract: 200 multi-shot steps with a
//! handler that fails every second model, at `-t 4`, must never leave a step,
//! or the loop overall, anything but cleanly `Ok` or a reported `Err` -- never
//! a crash, and never a poisoned control from a handler's own failure
//! (unaffected by this fix).
//!
//! a model-event failure takes the `goon = false` path, like the other three
//! events; the trampoline never returns `false` to clingo for any event, so it
//! can never see clasp go inconsistent this way.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::ops::ControlFlow;

use clingox::{
    Control, ErrorKind, ExtendableModel, Part, Result, SolveEventHandler, SolveOptions, Symbol,
};

fn poisoned(ctl: &Control) -> bool {
    format!("{ctl:?}").contains("poisoned")
}

/// Fails on the second model it sees, in every step (a fresh handler is
/// built for each `solve_with_events` call, so "second" resets every step).
#[derive(Default)]
struct FailOnSecondModel(u32);
impl SolveEventHandler for FailOnSecondModel {
    fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
        self.0 += 1;
        if self.0 == 2 {
            return Err(clingox::Error::new(
                ErrorKind::InvalidInput,
                "fails on the second model",
            ));
        }
        Ok(ControlFlow::Continue(()))
    }
}

/// A pigeonhole-shaped step body (`{p(k,1..4)} :- go(k).` with a spread
/// constraint) that reliably has more than one model per step, so
/// `FailOnSecondModel` actually gets to fail rather than the search ending
/// after its first model.
const STEP_PROGRAM: &str =
    "#external go(k). { p(k,1..4) } :- go(k). :- p(k,X), p(k,Y), X < Y, Y - X > 2.";

#[test]
fn two_hundred_multishot_steps_with_a_failing_handler_at_four_threads_never_crash() {
    // Parallel solving needs a build of clingo with threads.
    if !clingox_sys::HAS_THREADS {
        return;
    }
    let mut ctl = Control::with_args(["-t", "4", "0"]).unwrap();
    ctl.add("step", &["k"], STEP_PROGRAM).unwrap();

    for i in 1_i32..=200 {
        let k = Symbol::number(i);
        let go = Symbol::function("go", &[k]).unwrap();
        ctl.ground(&[Part::new("step", &[k]).unwrap()]).unwrap();
        ctl.assign_external(go, clingox::TruthValue::True).unwrap();

        let result = ctl.solve_with_events(SolveOptions::new(), FailOnSecondModel::default());
        match &result {
            Ok(_) => {}
            Err(err) => {
                assert_eq!(
                    err.kind(),
                    ErrorKind::InvalidInput,
                    "step {i}: a handler failure must report the handler's own error kind, not \
                     a crash-shaped one: {err:?}"
                );
            }
        }
        assert!(
            !poisoned(&ctl),
            "step {i}: a handler's own failure must never poison the control: \
             {ctl:?}"
        );

        // The very next update after the failing
        // search is what read out of bounds under ASan.
        ctl.release_external(go).unwrap();
        ctl.cleanup().unwrap();
        assert!(
            !poisoned(&ctl),
            "step {i}: cleanup must not poison either: {ctl:?}"
        );
    }

    // The control is still fully usable after 200 steps of a failing
    // handler: a corrupted clasp state would show up here even if every
    // individual step above happened not to crash.
    assert!(ctl.solve(&[]).unwrap().is_sat() || ctl.solve(&[]).unwrap().is_unsat());
}

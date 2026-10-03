//! A timeout shorter than the time a call needs to start its search must still
//! stop the search. The deadline is kept by a timer thread that calls
//! `InterruptHandle::interrupt`, which acts only on a running search (DESIGN
//! S13), so a deadline that passed before the search started was lost and the
//! search ran to its end: 1 to 7 of 5 000 zero-timeout `solve_with_events`
//! calls in a 5 000-round run. The timer now retries until the interrupt is
//! accepted or the call returns.
//!
//! The program is 11 pigeons in 10 holes, unsatisfiable and slow to prove, so a
//! lost deadline shows as an exhausted result, never as a false pass. The loss
//! is a race, so each test runs many rounds; `CLINGOX_TEST_ROUNDS` changes how
//! many.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test helpers fail loudly, and say why they skip"
)]

use std::ops::ControlFlow;
use std::time::Duration;

use clingox::{
    Control, ExtendableModel, Outcome, Part, SolveEventHandler, SolveOptions, SolveResult,
};

const PIGEONS: &str =
    "p(1..11). h(1..10). 1 { at(P,H) : h(H) } 1 :- p(P). :- at(P,H), at(Q,H), P < Q.";

fn rounds() -> usize {
    std::env::var("CLINGOX_TEST_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5000)
}

fn grounded() -> Control {
    let mut ctl = Control::new().unwrap();
    ctl.add_base(PIGEONS).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl
}

fn check(result: SolveResult, call: &str, round: usize) {
    assert!(
        result.is_interrupted() && !result.is_exhausted(),
        "{call} with a zero timeout, round {round}: the deadline was lost ({result:?})"
    );
}

struct Ignore;

impl SolveEventHandler for Ignore {
    fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> clingox::Result<ControlFlow<()>> {
        Ok(ControlFlow::Continue(()))
    }
}

#[test]
fn a_zero_timeout_stops_solve_with_events() {
    if !clingox_sys::HAS_THREADS {
        eprintln!("SKIPPED: timeouts need threads");
        return;
    }
    for round in 0..rounds() {
        let mut ctl = grounded();
        let result = ctl
            .solve_with_events(SolveOptions::new().timeout(Duration::ZERO), Ignore)
            .unwrap();
        check(result, "solve_with_events", round);
    }
}

#[test]
fn a_zero_timeout_stops_a_yielding_search() {
    if !clingox_sys::HAS_THREADS {
        eprintln!("SKIPPED: timeouts need threads");
        return;
    }
    for round in 0..rounds() {
        let mut ctl = grounded();
        match ctl
            .solve_first_with(SolveOptions::new().timeout(Duration::ZERO))
            .unwrap()
        {
            Outcome::Unknown(result) => check(result, "solve_first_with", round),
            other => panic!(
                "solve_first_with with a zero timeout, round {round}: the deadline was lost ({other:?})"
            ),
        }
    }
}

#[test]
fn a_zero_timeout_stops_solve_with() {
    if !clingox_sys::HAS_THREADS {
        eprintln!("SKIPPED: timeouts need threads");
        return;
    }
    for round in 0..rounds() {
        let mut ctl = grounded();
        let result = ctl
            .solve_with(SolveOptions::new().timeout(Duration::ZERO))
            .unwrap();
        check(result, "solve_with", round);
    }
}

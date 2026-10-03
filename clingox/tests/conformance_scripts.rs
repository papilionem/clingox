//! Ported from potassco/clingo v5.8.2
//! Source: `app/clingo/tests/python/` and `app/clingo/tests/lua/`.
//!
//! Each fixture's `#script (python)` / `#script (lua)` block is stripped
//! (`conformance::strip_script`) and its logic reimplemented in Rust, driving
//! a clingox `Control` directly. `icolor.lp` and `incshow.lp` have no script
//! at all: they are plain `#include <incmode>` fixtures, ported with the same
//! incmode driver as the `lp/` fixtures.
//!
//! `conformance::ScriptRun` reproduces `run.py`'s `normalize()`: one `Step`
//! block per `prg.solve(...)` call in the upstream script (see the comment
//! above `ScriptRun` in `conformance/mod.rs` for how that was established),
//! holding the atoms of every model that call actually returns, with the
//! overall result taken from the last solve call.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each handler or helper is defined next to the test that uses it"
)]

mod conformance;

use std::cell::RefCell;
use std::ops::ControlFlow;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clingox::{
    Control, ExtendableModel, FunctionCall, MessageCode, Part, ProgramLiteral,
    Result as ClingoxResult, ShowType, Signature, SolveEventHandler, SolveResult, Symbol,
    TheoryTerm, TruthValue,
};
use conformance::{ScriptRun, assert_fixture_matches, assert_script_matches, strip_script};

/// Whether this build of clingo has threads, which `Control::solve_async`
/// needs.
fn has_threads() -> bool {
    clingox_sys::HAS_THREADS
}

/// A 0-ary atom, for assumptions and external assignments.
fn atom(name: &str) -> Symbol {
    Symbol::function(name, &[]).expect("a plain name is a valid symbol")
}

/// Include a Python fixture from the submodule.
macro_rules! py {
    ($name:literal) => {
        include_str!(concat!(
            "../../clingox-sys/clingo/app/clingo/tests/python/",
            $name
        ))
    };
}

/// Include a Lua fixture from the submodule.
macro_rules! lua {
    ($name:literal) => {
        include_str!(concat!(
            "../../clingox-sys/clingo/app/clingo/tests/lua/",
            $name
        ))
    };
}

// ---------------------------------------------------------------------------
// assumptions1.lp -- ground base, solve once under one positive assumption.
// ---------------------------------------------------------------------------

#[test]
fn script_assumptions1() {
    let program = strip_script(py!("assumptions1.lp"));
    let expected = py!("assumptions1.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[(atom("a"), true).into()]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// assumptions2.lp -- as assumptions1.lp, but the assumption is on `b`, which
// is only derived from `a`: assuming it true forces `a` true too.
// ---------------------------------------------------------------------------

#[test]
fn script_assumptions2() {
    let program = strip_script(py!("assumptions2.lp"));
    let expected = py!("assumptions2.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[(atom("b"), true).into()]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// assumptions3.lp -- two solves with different assumptions on the same
// grounding, including one on an external (`d`) that was never assigned.
// ---------------------------------------------------------------------------

#[test]
fn script_assumptions3() {
    let program = strip_script(py!("assumptions3.lp"));
    let expected = py!("assumptions3.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let (a, b, c, d) = (atom("a"), atom("b"), atom("c"), atom("d"));
    let mut run = ScriptRun::new();
    let _ = run
        .solve(
            &mut ctl,
            &[(a, true).into(), (b, false).into(), (d, true).into()],
        )
        .unwrap();
    let _ = run
        .solve(&mut ctl, &[(a, false).into(), (c, true).into()])
        .unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// assumptions4.lp -- the same solve repeated through three of pyclingo's
// APIs (plain, async, yield). None of the three is cancelled, so the search
// runs to completion every time and the real clingo app prints the same
// model regardless of which API asked for it; the port uses three ordinary
// solves (see the note above `ScriptRun` in `conformance/mod.rs`).
// ---------------------------------------------------------------------------

#[test]
fn script_assumptions4() {
    let program = strip_script(py!("assumptions4.lp"));
    let expected = py!("assumptions4.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let a = atom("a");
    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[(a, true).into()]).unwrap();
    let _ = run.solve(&mut ctl, &[(a, true).into()]).unwrap();
    let _ = run.solve(&mut ctl, &[(a, true).into()]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// blocksworld1.lp -- five incremental ground+solve rounds, each on a
// different combination of program parts. Ported verbatim from the script's
// five blocks; the module-level command comment is not code.
// ---------------------------------------------------------------------------

/// A `Part` built from a name and plain numeric arguments.
fn numeric_part(name: &str, args: &[i32]) -> Part {
    let symbols: Vec<Symbol> = args.iter().map(|&n| Symbol::number(n)).collect();
    Part::new(name, &symbols).expect("a plain numeric part is valid")
}

#[test]
fn script_blocksworld1() {
    let program = strip_script(py!("blocksworld1.lp"));
    let expected = py!("blocksworld1.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();

    ctl.ground(&[
        numeric_part("init", &[1, 0]),
        numeric_part("state", &[1, 0]),
    ])
    .unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    let mut parts = vec![
        numeric_part("base_2", &[]),
        numeric_part("init", &[2, 0]),
        numeric_part("state", &[2, 0]),
    ];
    for i in 1..3 {
        parts.push(numeric_part("table", &[i]));
        for j in 1..3 {
            parts.push(numeric_part("state", &[j, i]));
            parts.push(numeric_part("move", &[j, i]));
        }
    }
    ctl.ground(&parts).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    let mut parts = vec![
        numeric_part("base_3", &[]),
        numeric_part("init", &[3, 0]),
        numeric_part("state", &[3, 0]),
    ];
    for i in 1..4 {
        parts.push(numeric_part("state", &[3, i]));
        parts.push(numeric_part("move", &[3, i]));
    }
    for j in 1..3 {
        parts.push(numeric_part("state", &[j, 3]));
        parts.push(numeric_part("move", &[j, 3]));
    }
    parts.push(numeric_part("table", &[3]));
    ctl.ground(&parts).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    let mut parts = vec![
        numeric_part("base_4", &[]),
        numeric_part("init", &[4, 0]),
        numeric_part("state", &[4, 0]),
    ];
    for i in 1..5 {
        parts.push(numeric_part("state", &[4, i]));
        parts.push(numeric_part("move", &[4, i]));
    }
    for j in 1..4 {
        parts.push(numeric_part("state", &[j, 4]));
        parts.push(numeric_part("move", &[j, 4]));
    }
    parts.push(numeric_part("table", &[4]));
    ctl.ground(&parts).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    let mut parts = vec![numeric_part("base_5", &[]), numeric_part("init", &[5, 1])];
    for i in 1..8 {
        parts.push(numeric_part("state", &[5, i]));
        parts.push(numeric_part("move", &[5, i]));
    }
    for i in 5..8 {
        parts.push(numeric_part("table", &[i]));
        for j in 1..5 {
            parts.push(numeric_part("state", &[j, i]));
            parts.push(numeric_part("move", &[j, i]));
        }
    }
    ctl.ground(&parts).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// cancel.lp -- solves normally, then starts an async search and cancels it
// shortly after. `AsyncSolveHandle` gives no access to models (by design:
// see its rustdoc), which matches the upstream fixture's own expectation
// that a cancelled step shows no models. Needs threads for the async half.
// ---------------------------------------------------------------------------

#[test]
fn script_cancel() {
    if !has_threads() {
        return;
    }
    let program = strip_script(py!("cancel.lp"));
    let expected = py!("cancel.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::new("pigeon", &[]).unwrap()]).unwrap();

    let p = atom("p");
    let mut run = ScriptRun::new();
    for _ in 0..2 {
        ctl.assign_external(p, TruthValue::True).unwrap();
        let _ = run.solve(&mut ctl, &[]).unwrap();
        ctl.assign_external(p, TruthValue::False).unwrap();
        let mut handle = ctl.solve_async(&[]).unwrap();
        handle.wait(Duration::from_millis(10));
        handle.cancel().unwrap();
        let result = handle.close().unwrap();
        run.record_result(result);
    }

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// conflicting.lp -- `Control::is_conflicting` on four small, throwaway
// controls, one of them solved first. Needs only `is_conflicting` plus
// `ground_with`/`FunctionCall`.
// The Lua `conflicting.lp` is a duplicate with the same script logic
// (`conformance/NOT_PORTED.md`); only the Python fixture is ported, matching
// the existing convention for the other duplicated fixtures in this suite.
// ---------------------------------------------------------------------------

/// `str(bool)` as Python spells it, matching `conflicting.sol` exactly.
fn py_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

/// Grounds `program` in a fresh, throwaway control, optionally solving it,
/// and reports whether it is conflicting afterward. Mirrors the upstream
/// script's own `ground` helper, which builds a fresh `clingo.Control()` for
/// each of the four checks.
fn conflicting_after(program: &str, solve: bool) -> bool {
    let mut ctl = Control::new().unwrap();
    ctl.add_base(program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    if solve {
        let _ = ctl.solve(&[]).unwrap();
    }
    ctl.is_conflicting()
}

fn conflicting_call(call: &mut FunctionCall<'_>) -> ClingoxResult<()> {
    match call.name() {
        "get" => {
            let tuple = Symbol::tuple(&[
                Symbol::string(py_bool(conflicting_after("a.", false)))?,
                Symbol::string(py_bool(conflicting_after(":-.", false)))?,
                Symbol::string(py_bool(conflicting_after("2 { a; b }. :- a, b.", false)))?,
                Symbol::string(py_bool(conflicting_after("2 { a; b }. :- a, b.", true)))?,
            ])?;
            call.push(tuple)
        }
        _ => Ok(()),
    }
}

#[test]
fn script_conflicting() {
    let program = strip_script(py!("conflicting.lp"));
    let expected = py!("conflicting.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    ctl.ground_with(&[Part::base()], conflicting_call).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// core1.lp, core2.lp (`NOT_PORTED.md`) -- `on_core` is not a real clingo solve
// event: pyclingo's `Control.solve` calls the user's `on_core` itself, in
// Python, only for a blocking call whose result is unsatisfiable
// (`libpyclingo/clingo/control.py:1076-1084`, `if on_core is not None and
// ret.unsatisfiable: on_core(handle.core())`). So both fixtures port as an
// ordinary `SolveHandle::core` read after a blocking-shaped search (already
// covered on its own in `api_solve_core.rs`), with no dependency on
// `SolveEventHandler` at all.
//
// Both scripts solve, use the core to build a backend constraint that blocks
// the same assumption forever, then solve again with no assumptions to show the
// constraint alone now reproduces the same unsatisfiability. Neither `.sol`
// file shows any model (both solves are UNSAT), so `ScriptRun::record_result`
// is the right accumulator (as `cancel.lp`/`interrupt.lp` already use it for a
// different reason); the core values themselves are asserted directly,
// oracle-checked against clingo 5.8.2 in the same way `api_solve_core.rs`
// already checks `SolveHandle::core` for the *same* `{a;b}. :- a, b.` program
// (`core_contains_both_assumptions_in_the_order_given_when_both_are_needed`).
// ---------------------------------------------------------------------------

/// Adds `:- not <lit's atom>.` for a positive `lit`, or `:- <lit's atom>.`
/// for a negative one, matching the upstream script's own
/// `backend.add_rule([], [-lit])`: negating each core literal in the body
/// of a fresh integrity constraint, so the constraint alone forever
/// reproduces the same unsatisfiability without needing the assumption
/// again.
fn block_core(ctl: &mut Control, core: &[ProgramLiteral]) {
    ctl.with_backend(|backend| {
        for &lit in core {
            backend.add_rule(clingox::backend::Head::Constraint, &[lit.negate()])?;
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn script_core1() {
    let program = strip_script(py!("core1.lp"));
    let expected = py!("core1.sol");

    let mut ctl = Control::new().unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (a, b) = (atom("a"), atom("b"));
    let la = ctl
        .symbolic_atoms()
        .unwrap()
        .find(a)
        .unwrap()
        .unwrap()
        .literal();
    let lb = ctl
        .symbolic_atoms()
        .unwrap()
        .find(b)
        .unwrap()
        .unwrap()
        .literal();

    let mut run = ScriptRun::new();

    // First solve, under both assumptions true: unsatisfiable (`:- a, b.`),
    // core is both assumption literals, in the order given (the same
    // program and oracle finding `api_solve_core.rs` already establishes).
    let mut handle = ctl
        .solve_yield(&[clingox::Assumption::from(la), clingox::Assumption::from(lb)])
        .unwrap();
    let result = handle.get().unwrap();
    assert!(result.is_unsat());
    let core = handle.core().unwrap();
    assert_eq!(core, [la, lb]);
    run.record_result(handle.close().unwrap());

    block_core(&mut ctl, &core);

    // Second solve, no assumptions: the fresh constraints alone
    // (`:- not a.`, `:- not b.`) force both `a` and `b` true, which
    // `:- a, b.` then forbids outright, so the program is unsatisfiable
    // unconditionally. The core is empty: it never contains anything but
    // given assumption literals
    // (`core_is_empty_when_unsat_does_not_come_from_assumptions`).
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let result = handle.get().unwrap();
    assert!(result.is_unsat());
    assert_eq!(handle.core().unwrap(), []);
    run.record_result(result);

    assert_script_matches(run, expected);
}

#[test]
fn script_core2() {
    let program = strip_script(py!("core2.lp"));
    let expected = py!("core2.sol");

    let mut ctl = Control::new().unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let a = atom("a");
    let la = ctl
        .symbolic_atoms()
        .unwrap()
        .find(a)
        .unwrap()
        .unwrap()
        .literal();

    let mut run = ScriptRun::new();

    // First solve, assuming `a` false: contradicts the fact `a.`, so the
    // core is the single negative assumption literal itself.
    let mut handle = ctl
        .solve_yield(&[clingox::Assumption::from(la.negate())])
        .unwrap();
    let result = handle.get().unwrap();
    assert!(result.is_unsat());
    let core = handle.core().unwrap();
    assert_eq!(core, [la.negate()]);
    run.record_result(handle.close().unwrap());

    block_core(&mut ctl, &core);

    // Second solve, no assumptions: the fresh constraint (`:- a.`)
    // directly contradicts the fact `a.`, unconditionally unsatisfiable,
    // core empty again.
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let result = handle.get().unwrap();
    assert!(result.is_unsat());
    assert_eq!(handle.core().unwrap(), []);
    run.record_result(result);

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// domain.lp -- the script never solves; it prints `Solving...`, `Answer: 1`
// and a computed line of its own, which `normalize()` parses exactly like
// one model of one step (see `ScriptRun::record_fake_model`). The atoms come
// from `SymbolicAtoms`, clingox's equivalent of `prg.symbolic_atoms`.
// ---------------------------------------------------------------------------

#[test]
fn script_domain() {
    let program = strip_script(py!("domain.lp"));
    let expected = py!("domain.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let mut fields = Vec::new();
    {
        let atoms = ctl.symbolic_atoms().unwrap();
        fields.push(format!("universe({})", atoms.len().unwrap()));
        for x in &atoms {
            let x = x.unwrap();
            fields.push(format!(
                "domain({},{},{})",
                x.symbol(),
                x.is_fact(),
                x.is_external()
            ));
        }
        let p2 = Symbol::function("p", &[Symbol::number(2)]).unwrap();
        let p4 = Symbol::function("p", &[Symbol::number(4)]).unwrap();
        fields.push(format!(
            "in_domain(p(2),{})",
            atoms.find(p2).unwrap().is_some()
        ));
        fields.push(format!(
            "in_domain(p(4),{})",
            atoms.find(p4).unwrap().is_some()
        ));
        let sig_p1 = clingox::Signature::new("p", 1).unwrap();
        for x in atoms.by_signature(sig_p1) {
            let x = x.unwrap();
            fields.push(format!(
                "domain_of_p({},{},{})",
                x.symbol(),
                x.is_fact(),
                x.is_external()
            ));
        }
        for sig in atoms.signatures().unwrap() {
            fields.push(format!("sig({},{})", sig.name(), sig.arity()));
        }
    }

    let mut run = ScriptRun::new();
    run.record_fake_model(fields);

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// external-lookup.lp -- ground callbacks `bar`, `foo` (which calls `bar`'s
// logic itself) and `foobar`, across two incremental ground+solve rounds.
// ---------------------------------------------------------------------------

fn external_lookup_call(call: &mut FunctionCall<'_>) -> ClingoxResult<()> {
    let x = call.args()[0].as_number().unwrap_or(0);
    match call.name() {
        "bar" => call.push(Symbol::number(42 + x)),
        "foo" => call.push(Symbol::number(42 + x + 1)),
        "foobar" => call.push(Symbol::number(x)),
        _ => Ok(()),
    }
}

#[test]
fn script_external_lookup() {
    let program = strip_script(py!("external-lookup.lp"));
    let expected = py!("external-lookup.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    ctl.ground_with(&[Part::base()], external_lookup_call)
        .unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    ctl.ground_with(&[Part::new("test", &[]).unwrap()], external_lookup_call)
        .unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// externals.lp -- assign, reassign, free and release one external atom.
// ---------------------------------------------------------------------------

#[test]
fn script_externals() {
    let program = strip_script(py!("externals.lp"));
    let expected = py!("externals.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let a = atom("a");
    let mut run = ScriptRun::new();
    ctl.assign_external(a, TruthValue::True).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    ctl.assign_external(a, TruthValue::False).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    ctl.assign_external(a, TruthValue::Free).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    ctl.release_external(a).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// extend-model.lp (`NOT_PORTED.md`) -- `Model::extend` (`ExtendableModel`
// in clingox, S6): every model of `1{a;b;c}1.` is extended with the number
// `17` inside `on_model`, and the extension is read back inside the same
// callback, since `clingo_model_extend`'s own doc comment says the symbols
// "are only meaningful if there is an underlying clingo application" (there
// is none here); `run.py` (the real `clingo` binary, unbounded models)
// still shows the extension because it *is* such an application. clingox
// has no such application layer, so this test reads the extension back
// through `ExtendableModel::symbols(ShowType::SHOWN | ShowType::THEORY)`
// from inside `on_model` itself, which needs no application and is exactly
// what pyclingo's own `m.symbols(theory=True)` selects
// (`libpyclingo/clingo/solving.py`'s `Model.symbols` doc: "theory: Select
// atoms added with `Model.extend`").
//
// Driven through `solve_yield_with_events` (not `for_each_model`, which has
// no event handler to extend from): `on_model` extends and captures each
// model's shown-plus-theory symbols into a shared `Vec`: the yielding form
// takes only `'static` handlers.
// ---------------------------------------------------------------------------

#[test]
#[allow(
    clippy::items_after_statements,
    reason = "the handler is defined next to its only use"
)]
fn script_extend_model() {
    let program = strip_script(py!("extend-model.lp"));
    let expected = py!("extend-model.sol");

    // Unbounded models, matching every other script fixture's harness
    // (`app/clingo/tests/run.py`'s own invocation passes clingo the
    // positional argument `0` unless a `.cmd` file overrides it, and
    // `extend-model.lp` has none): `1{a;b;c}1.` has three answer sets.
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    struct ExtendWithSeventeen {
        models: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    }

    impl SolveEventHandler for ExtendWithSeventeen {
        fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> ClingoxResult<ControlFlow<()>> {
            model.extend([Symbol::number(17)])?;
            let mut atoms: Vec<String> = model
                .symbols(ShowType::SHOWN | ShowType::THEORY)?
                .iter()
                .map(ToString::to_string)
                .collect();
            atoms.sort();
            self.models.lock().unwrap().push(atoms);
            Ok(ControlFlow::Continue(()))
        }
    }

    let models = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let handler = ExtendWithSeventeen {
        models: std::sync::Arc::clone(&models),
    };
    let mut handle = ctl.solve_yield_with_events(&[], handler).unwrap();
    while handle.next_model().unwrap().is_some() {}
    let result = handle.close().unwrap();

    let mut run = ScriptRun::new();
    let models = std::mem::take(&mut *models.lock().unwrap());
    run.push_step(models, result);
    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// iclingo.lp -- incremental grounding of `step(k)`, releasing the previous
// step's `vol` external, stopping once the first model is found.
// ---------------------------------------------------------------------------

#[test]
fn script_iclingo() {
    let program = strip_script(py!("iclingo.lp"));
    let expected = py!("iclingo.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    let mut step: i32 = 0;
    loop {
        if step > 0 {
            let old = Symbol::function("vol", &[Symbol::number(step - 1)]).unwrap();
            ctl.release_external(old).unwrap();
        }
        ctl.ground(&[numeric_part("step", &[step])]).unwrap();
        let vol = Symbol::function("vol", &[Symbol::number(step)]).unwrap();
        ctl.assign_external(vol, TruthValue::True).unwrap();
        let result = run.solve(&mut ctl, &[]).unwrap();
        if result.is_sat() {
            break;
        }
        step += 1;
    }

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// infsup.lp -- no `main`, so the default applies (ground base, solve once).
// Ground callbacks `inf`/`sup` return the infimum/supremum symbols.
// ---------------------------------------------------------------------------

fn infsup_call(call: &mut FunctionCall<'_>) -> ClingoxResult<()> {
    match call.name() {
        "inf" => call.push(Symbol::infimum()),
        "sup" => call.push(Symbol::supremum()),
        _ => Ok(()),
    }
}

#[test]
fn script_infsup() {
    let program = strip_script(py!("infsup.lp"));
    let expected = py!("infsup.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    ctl.ground_with(&[Part::base()], infsup_call).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// interrupt.lp -- as cancel.lp, but stops the search with `InterruptHandle`
// instead of `AsyncSolveHandle::cancel`. Needs threads for the async half.
// ---------------------------------------------------------------------------

#[test]
fn script_interrupt() {
    if !has_threads() {
        return;
    }
    let program = strip_script(py!("interrupt.lp"));
    let expected = py!("interrupt.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::new("pigeon", &[]).unwrap()]).unwrap();

    let p = atom("p");
    let mut run = ScriptRun::new();
    for _ in 0..2 {
        ctl.assign_external(p, TruthValue::True).unwrap();
        let _ = run.solve(&mut ctl, &[]).unwrap();
        ctl.assign_external(p, TruthValue::False).unwrap();
        let stop = ctl.interrupt_handle();
        let mut handle = ctl.solve_async(&[]).unwrap();
        handle.wait(Duration::from_millis(10));
        stop.interrupt();
        let result = handle.close().unwrap();
        run.record_result(result);
    }

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// logger.lp -- a nested control with a custom logger and message limit,
// whose captured messages feed a ground callback of the real control.
//
// `Message::text()` trims trailing whitespace, so the
// captured "atom does not occur..." message is one trailing `\n` shorter
// than the raw text pyclingo hands to its logger.
// ---------------------------------------------------------------------------

#[test]
fn script_logger() {
    let program = strip_script(py!("logger.lp"));
    // `Message::text` drops the newline clingo ends each message with (see its
    // documentation), and pyclingo keeps it, so the upstream text is adjusted
    // by exactly that: the `\n` before each closing quote.
    let expected = py!("logger.sol").replace("\\n\")", "\")");

    let messages: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&messages);
    let mut nested = Control::builder()
        .message_limit(2)
        .logger(move |code: MessageCode, text: &str| {
            sink.lock()
                .unwrap()
                .push((format!("{code:?}"), text.replace(' ', "_")));
        })
        .build()
        .unwrap();
    nested.add("base", &[], ":- a. :- b. :- c.").unwrap();
    nested.ground(&[Part::base()]).unwrap();
    drop(nested);

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    ctl.ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
        if call.name() == "msg" {
            for (code, text) in messages.lock().unwrap().iter() {
                let tuple = Symbol::tuple(&[Symbol::string(code)?, Symbol::string(text)?])?;
                call.push(tuple)?;
            }
        }
        Ok(())
    })
    .unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, &expected);
}

// ---------------------------------------------------------------------------
// multi.lp -- two program parts, each grounded and solved in turn.
// ---------------------------------------------------------------------------

#[test]
fn script_multi() {
    let program = strip_script(py!("multi.lp"));
    let expected = py!("multi.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    ctl.ground(&[Part::new("a", &[]).unwrap()]).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    ctl.ground(&[Part::new("b", &[]).unwrap()]).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// parse-term.lp -- no `main`. Ground callback `get` returns terms parsed
// with clingo's own term syntax (`Symbol::from_str`, backed by
// `clingo_parse_term`, matching `clingo.parse_term`). The script's own
// top-level `c = clingo.Control(); c.ground(...); c.solve()` builds an
// unrelated, throwaway control whose result is never part of the observable
// output, so it is not reproduced.
// ---------------------------------------------------------------------------

fn parse_term_call(call: &mut FunctionCall<'_>) -> ClingoxResult<()> {
    if call.name() == "get" {
        for text in ["1", "p(1+2)", "-p", "-p(1)"] {
            let symbol: Symbol = text.parse()?;
            call.push(symbol)?;
        }
    }
    Ok(())
}

#[test]
fn script_parse_term() {
    let program = strip_script(py!("parse-term.lp"));
    let expected = py!("parse-term.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    ctl.ground_with(&[Part::base()], parse_term_call).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// queens.lp -- incremental grounding of `cumulative(k)`, releasing the
// previous step's `volatile` external, for a fixed 10 steps regardless of
// satisfiability.
// ---------------------------------------------------------------------------

#[test]
fn script_queens() {
    let program = strip_script(py!("queens.lp"));
    let expected = py!("queens.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    let last_step = 10;
    let mut step = 1;
    loop {
        if step > 1 {
            let old = Symbol::function("volatile", &[Symbol::number(step - 1)]).unwrap();
            ctl.release_external(old).unwrap();
        }
        ctl.ground(&[numeric_part("cumulative", &[step])]).unwrap();
        let v = Symbol::function("volatile", &[Symbol::number(step)]).unwrap();
        ctl.assign_external(v, TruthValue::True).unwrap();
        let _ = run.solve(&mut ctl, &[]).unwrap();
        if step == last_step {
            break;
        }
        step += 1;
    }

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// ret.lp -- no `main`. Ground callbacks returning a tuple, a list and an
// iterator: `a` pushes one tuple symbol, `b` and `c` push three symbols
// each, matching how clingox's `FunctionCall::push` models several values.
// The script's top-level throwaway `ctl` is not reproduced (see parse-term).
// ---------------------------------------------------------------------------

fn ret_call(call: &mut FunctionCall<'_>) -> ClingoxResult<()> {
    match call.name() {
        "a" => {
            let tuple = Symbol::tuple(&[Symbol::number(1), Symbol::number(2), Symbol::number(3)])?;
            call.push(tuple)
        }
        "b" | "c" => {
            for n in 1..=3 {
                call.push(Symbol::number(n))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[test]
fn script_ret() {
    let program = strip_script(py!("ret.lp"));
    let expected = py!("ret.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    ctl.ground_with(&[Part::base()], ret_call).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// setconfig.lp -- changes `solve.models` between two incremental rounds.
// `ScriptRun::solve` is built on `for_each_model`, which honours the
// configured model limit, so the first round's limit of 1 is respected.
// ---------------------------------------------------------------------------

#[test]
fn script_setconfig() {
    let program = strip_script(py!("setconfig.lp"));
    let expected = py!("setconfig.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let f = atom("f");
    let mut run = ScriptRun::new();
    ctl.ground(&[Part::new("step1", &[]).unwrap()]).unwrap();
    ctl.assign_external(f, TruthValue::True).unwrap();
    ctl.configuration().set("solve.models", "1").unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    ctl.ground(&[Part::new("step2", &[]).unwrap()]).unwrap();
    ctl.assign_external(f, TruthValue::False).unwrap();
    ctl.configuration().set("solve.models", "0").unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// show.lp -- an `on_model` handler that records four summaries per model
// (its shown text, and its shown/atoms/terms symbols), consumed by a ground
// callback of a second, incrementally-grounded part.
// ---------------------------------------------------------------------------

#[test]
fn script_show() {
    let program = strip_script(py!("show.lp"));
    let expected = py!("show.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let accumulator: Rc<RefCell<Vec<Symbol>>> = Rc::new(RefCell::new(Vec::new()));
    let mut run = ScriptRun::new();

    let mut model_texts = Vec::new();
    let result = ctl
        .for_each_model(&[], |model| {
            let mut shown = model.symbols(ShowType::SHOWN)?;
            shown.sort();
            let mut shown_text: Vec<String> = shown.iter().map(ToString::to_string).collect();
            shown_text.sort();
            model_texts.push(shown_text.clone());

            let model_args: Vec<Symbol> = shown_text
                .iter()
                .map(|s| Symbol::string(s))
                .collect::<ClingoxResult<Vec<_>>>()?;
            let model_entry = Symbol::function("model", &model_args)?;
            let shown_entry = Symbol::function("shown", &shown)?;

            let mut atoms = model.symbols(ShowType::ATOMS)?;
            atoms.sort();
            let atoms_entry = Symbol::function("atoms", &atoms)?;

            let mut terms = model.symbols(ShowType::TERMS)?;
            terms.sort();
            let terms_entry = Symbol::function("terms", &terms)?;

            accumulator
                .borrow_mut()
                .extend([model_entry, shown_entry, atoms_entry, terms_entry]);
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    run.push_step(model_texts, result);

    ctl.ground_with(
        &[Part::new("result", &[]).unwrap()],
        |call: &mut FunctionCall<'_>| {
            if call.name() == "getModels" {
                for symbol in accumulator.borrow().iter() {
                    call.push(*symbol)?;
                }
            }
            Ok(())
        },
    )
    .unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// test-numeric.lp -- assumptions and external assignments given as raw program
// literals, including one (`bot = 100`) with no symbol behind it. Ported now
// that both literal forms exist. One difference from upstream: step 4's
// `prg.assign_external(-lit(p(1)), True)` passes a *negative* literal to
// `assign_external`, which clingox's `assign_external_literal` rejects by
// design (`ErrorKind::InvalidInput`, tested in `api_literals.rs`). `clingo.h`
// documents a negative literal with `TruthValue::True` as exactly equivalent to
// the same positive literal with `TruthValue::False`
// (`clingo_control_assign_external`'s own doc comment), so the port calls
// `assign_external_literal(lit_p1, TruthValue::False)` there instead; the
// observable result (`Step: 4` is an empty answer set) is unchanged and matches
// `test-numeric.sol` exactly. Every other step is a direct, unmodified
// translation.
// ---------------------------------------------------------------------------

fn p(n: i32) -> Symbol {
    Symbol::function("p", &[Symbol::number(n)]).unwrap()
}

/// The program literal of a symbol that is a current atom.
fn literal_of(ctl: &Control, symbol: Symbol) -> ProgramLiteral {
    ctl.symbolic_atoms()
        .unwrap()
        .find(symbol)
        .unwrap()
        .expect("the symbol is a current atom")
        .literal()
}

#[test]
fn script_test_numeric() {
    let program = strip_script(py!("test-numeric.lp"));
    let expected = py!("test-numeric.sol");

    let mut ctl = Control::new().unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let lit_p3 = literal_of(&ctl, p(3));
    let lit_p4 = literal_of(&ctl, p(4));

    let mut run = ScriptRun::new();
    let _ = run
        .solve(&mut ctl, &[(p(3), false).into(), lit_p4.into()])
        .unwrap();
    let _ = run
        .solve(&mut ctl, &[(p(3), true).into(), (-lit_p4).into()])
        .unwrap();

    ctl.assign_external(p(1), TruthValue::True).unwrap();
    let _ = run
        .solve(&mut ctl, &[(-lit_p3).into(), (-lit_p4).into()])
        .unwrap();

    // Upstream: `prg.assign_external(-lit(p(1)), True)`; see the comment
    // above this test for why the port uses the positive-literal form.
    let lit_p1 = literal_of(&ctl, p(1));
    ctl.assign_external_literal(lit_p1, TruthValue::False)
        .unwrap();
    let _ = run
        .solve(&mut ctl, &[(-lit_p3).into(), (-lit_p4).into()])
        .unwrap();

    ctl.assign_external_literal(lit_p1, TruthValue::True)
        .unwrap();
    let _ = run
        .solve(&mut ctl, &[(-lit_p3).into(), (-lit_p4).into()])
        .unwrap();

    // A literal with no symbol behind it at all.
    let bot = ProgramLiteral::from_raw(100).unwrap();
    let _ = run
        .solve(&mut ctl, &[(-lit_p3).into(), (-lit_p4).into(), bot.into()])
        .unwrap();
    let _ = run
        .solve(
            &mut ctl,
            &[(-lit_p3).into(), (-lit_p4).into(), (-bot).into()],
        )
        .unwrap();

    // Upstream's own comment: "this actually creates the external, hence
    // the subsequent call with the assumption is unsatisfiable."
    ctl.assign_external_literal(bot, TruthValue::True).unwrap();
    let _ = run
        .solve(
            &mut ctl,
            &[(-lit_p3).into(), (-lit_p4).into(), (-bot).into()],
        )
        .unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// icolor.lp -- no script: a plain `#include <incmode>` fixture, like the
// `lp/` fixtures. Ported with the same incmode driver.
// ---------------------------------------------------------------------------

#[test]
fn script_icolor() {
    let program = lua!("icolor.lp");
    let expected = lua!("icolor.sol");
    assert_fixture_matches(&[], program, expected);
}

// ---------------------------------------------------------------------------
// incshow.lp -- no script: a plain `#include <incmode>` fixture with
// `#const imin=4.`, parsed by the incmode driver like `istop.lp`.
// ---------------------------------------------------------------------------

#[test]
fn script_incshow() {
    let program = lua!("incshow.lp");
    let expected = lua!("incshow.sol");
    assert_fixture_matches(&[], program, expected);
}

// ---------------------------------------------------------------------------
// mutex-bug.lp (lua) -- ground base plus a fixed part, then three
// incremental rounds of a stepped part.
// ---------------------------------------------------------------------------

#[test]
fn script_mutex_bug() {
    let program = strip_script(lua!("mutex-bug.lp"));
    let expected = lua!("mutex-bug.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base(), Part::new("plan_graph_base", &[]).unwrap()])
        .unwrap();

    let mut run = ScriptRun::new();
    for step in 1..=3 {
        ctl.ground(&[numeric_part("plan_graph_step", &[step])])
            .unwrap();
        let _ = run.solve(&mut ctl, &[]).unwrap();
    }

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// project_bug.lp (lua) -- ground base, add and ground a second part built
// from a string at run time.
// ---------------------------------------------------------------------------

#[test]
fn script_project_bug() {
    let program = strip_script(lua!("project_bug.lp"));
    let expected = lua!("project_bug.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.add("rules", &[], "r :- q(_), p.").unwrap();
    ctl.ground(&[Part::new("rules", &[]).unwrap()]).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// robots.lp, solitaire_para.lp, solitaire_sort.lp, toh.lp (lua) -- the same
// incremental loop: ground `base` once, then `cumulative(step)` each round,
// releasing the previous round's `volatile` external, stopping once
// satisfiable.
// ---------------------------------------------------------------------------

/// The loop shared by `robots.lp`, `solitaire_para.lp`, `solitaire_sort.lp`
/// and `toh.lp`.
fn run_volatile_cumulative(ctl: &mut Control, run: &mut ScriptRun) {
    let mut step: i32 = 1;
    loop {
        let mut parts = Vec::new();
        if step > 1 {
            let old = Symbol::function("volatile", &[Symbol::number(step - 1)]).unwrap();
            ctl.release_external(old).unwrap();
        } else {
            parts.push(Part::base());
        }
        parts.push(numeric_part("cumulative", &[step]));
        ctl.ground(&parts).unwrap();
        let v = Symbol::function("volatile", &[Symbol::number(step)]).unwrap();
        ctl.assign_external(v, TruthValue::True).unwrap();
        let result = run.solve(ctl, &[]).unwrap();
        if result.is_sat() {
            break;
        }
        step += 1;
    }
}

#[test]
fn script_robots() {
    let program = strip_script(lua!("robots.lp"));
    let expected = lua!("robots.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    run_volatile_cumulative(&mut ctl, &mut run);

    assert_script_matches(run, expected);
}

#[test]
fn script_solitaire_para() {
    let program = strip_script(lua!("solitaire_para.lp"));
    let expected = lua!("solitaire_para.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    run_volatile_cumulative(&mut ctl, &mut run);

    assert_script_matches(run, expected);
}

#[test]
fn script_solitaire_sort() {
    let program = strip_script(lua!("solitaire_sort.lp"));
    let expected = lua!("solitaire_sort.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    run_volatile_cumulative(&mut ctl, &mut run);

    assert_script_matches(run, expected);
}

#[test]
fn script_toh() {
    let program = strip_script(lua!("toh.lp"));
    let expected = lua!("toh.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    run_volatile_cumulative(&mut ctl, &mut run);

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// theory-term-types.lp (lua) -- a ground callback returning the fixed set of
// theory term type names. The script does not create or inspect any real
// theory atom; it only stringifies a fixed enum, so no theory atoms API is
// needed to reproduce it.
// ---------------------------------------------------------------------------

fn theory_term_types_call(call: &mut FunctionCall<'_>) -> ClingoxResult<()> {
    if call.name() == "types" {
        for name in ["Tuple", "List", "Set", "Function", "Symbol", "Number"] {
            call.push(Symbol::string(name)?)?;
        }
    }
    Ok(())
}

#[test]
fn script_theory_term_types() {
    let program = strip_script(lua!("theory-term-types.lp"));
    let expected = lua!("theory-term-types.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    ctl.ground_with(&[Part::base()], theory_term_types_call)
        .unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// unsat-sync.lp (lua) -- an always-false constraint, solved twice.
// ---------------------------------------------------------------------------

#[test]
fn script_unsat_sync() {
    let program = strip_script(lua!("unsat-sync.lp"));
    let expected = lua!("unsat-sync.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ===========================================================================
// Further fixtures: the ones needing the backend (`add_atom`, `backend_*`) and
// the Lua-only `theory.lp`/`project_bug2.lp`.
// ===========================================================================

// ---------------------------------------------------------------------------
// add_atom.lp (python; the Lua original is a byte-for-byte duplicate, listed
// as such in NOT_PORTED.md, matching the existing convention) -- two backend
// atoms as choice facts, ground, add a backend fact after grounding, solve,
// ground an undeclared "multi" part (a no-op) and solve again.
// ---------------------------------------------------------------------------

#[test]
fn script_add_atom() {
    use clingox::backend::Head;

    let program = strip_script(py!("add_atom.lp"));
    let expected = py!("add_atom.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.with_backend(|backend| {
        let a2 = backend.add_atom(Some("a(2)".parse()?))?;
        let b1 = backend.add_atom(Some("b(1)".parse()?))?;
        backend.add_rule(Head::Choice(&[a2]), &[])?;
        backend.add_rule(Head::Choice(&[b1]), &[])
    })
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    ctl.with_backend(|backend| {
        let e3 = backend.add_atom(Some("e(3)".parse()?))?;
        backend.add_rule(Head::Normal(&[e3]), &[])
    })
    .unwrap();
    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    ctl.ground(&[Part::new("multi", &[]).unwrap()]).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// backend_acyc.lp (python) -- `{a;b}.`, then an acyclicity constraint on the
// same two (named) atoms forbids the model where both hold.
// ---------------------------------------------------------------------------

#[test]
fn script_backend_acyc() {
    let program = strip_script(py!("backend_acyc.lp"));
    let expected = py!("backend_acyc.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.with_backend(|backend| {
        let a = backend.add_atom(Some(atom("a")))?;
        let b = backend.add_atom(Some(atom("b")))?;
        backend.add_edge(1, 2, &[a.pos()])?;
        backend.add_edge(2, 1, &[b.pos()])
    })
    .unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// backend_assume.lp (python) -- a one-shot backend assumption directive:
// `-a, b` forces the next solve alone.
// ---------------------------------------------------------------------------

#[test]
fn script_backend_assume() {
    let program = strip_script(py!("backend_assume.lp"));
    let expected = py!("backend_assume.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.with_backend(|backend| {
        let a = backend.add_atom(Some(atom("a")))?;
        let b = backend.add_atom(Some(atom("b")))?;
        backend.add_assumptions([a.neg(), b.pos()])
    })
    .unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// backend_heuristic.lp (python) -- the script narrows the model limit back
// to 1 (`configuration.solve.models = "1"`) and steers the domain heuristic
// so `a` wins over `b`.
// ---------------------------------------------------------------------------

#[test]
fn script_backend_heuristic() {
    use clingox::backend::HeuristicKind;

    let program = strip_script(py!("backend_heuristic.lp"));
    let expected = py!("backend_heuristic.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.configuration().set("solve.models", "1").unwrap();
    // `solve.parallel_mode` is not a registered configuration key at all on
    // a clasp build without threads (a WASM/emscripten build without
    // threading support, checked directly): setting it there is a runtime
    // error ("invalid key"), not a clingox bug, and the fixture's own value
    // ("1", meaning one thread) is a no-op on a single-threaded build
    // anyway.
    if has_threads() {
        ctl.configuration().set("solve.parallel_mode", "1").unwrap();
    }
    ctl.configuration()
        .set("solver.heuristic", "domain")
        .unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.with_backend(|backend| {
        let a = backend.add_atom(Some(atom("a")))?;
        let b = backend.add_atom(Some(atom("b")))?;
        backend.add_heuristic(a, HeuristicKind::True, 1, 1, &[])?;
        backend.add_heuristic(b, HeuristicKind::False, 1, 1, &[])
    })
    .unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// backend_project.lp (python; `.cmd`: `--project=project`) -- `#show.`
// suppresses every shown atom; only the model *count* (after projecting
// onto `a`) is visible in the normalised output.
// ---------------------------------------------------------------------------

#[test]
fn script_backend_project() {
    let program = strip_script(py!("backend_project.lp"));
    let expected = py!("backend_project.sol");

    let mut ctl = Control::with_args(["--models=0", "--project=project"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.with_backend(|backend| {
        let a = backend.add_atom(Some(atom("a")))?;
        backend.add_project([a])
    })
    .unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// project_bug2.lp (lua) -- two program parts, a backend fact added between
// them.
// ---------------------------------------------------------------------------

#[test]
fn script_project_bug2() {
    use clingox::backend::Head;

    let program = strip_script(lua!("project_bug2.lp"));
    let expected = lua!("project_bug2.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::new("a", &[]).unwrap()]).unwrap();
    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    ctl.with_backend(|backend| {
        let q0 = backend.add_atom(Some("q(0)".parse()?))?;
        backend.add_rule(Head::Normal(&[q0]), &[])
    })
    .unwrap();
    ctl.ground(&[Part::new("b", &[]).unwrap()]).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// theory.lp (lua; no Python duplicate) -- every theory atom of "base" is
// reified into a ground term through a `@get()` pool callback, read back with
// the `theory` program part's own `p(@get())`.
//
// `convert_term` mirrors the Lua script's own recursive conversion:
// `TheoryTerm::Compound { kind: Function, .. }` becomes a 2-tuple of (name,
// arguments) like the script's `clingo.Tuple({t.name, clingo.Tuple(a)})`,
// `TheoryTerm::Number` stays a number, `TheoryTerm::Symbol` becomes a string
// (matching the script's `t.name`, a Lua string, auto-converted to a
// `clingo.String` by the Lua binding when placed inside a `Tuple`; clingox's
// Rust port makes that conversion explicit with `Symbol::string`). Checked
// directly against clingo 5.8.2's own element sort order for the two-element
// theory atom `&a`.
// ---------------------------------------------------------------------------

fn convert_theory_term(term: &TheoryTerm<'_>) -> Symbol {
    use clingox::TheoryTermKind;
    match term {
        TheoryTerm::Number(n) => Symbol::number(*n),
        TheoryTerm::Symbol(s) => Symbol::string(s.name().unwrap_or_default()).unwrap(),
        TheoryTerm::Compound {
            kind: TheoryTermKind::Function,
            name,
            arguments,
        } => {
            let args: Vec<Symbol> = arguments.iter().map(convert_theory_term).collect();
            Symbol::tuple(&[
                Symbol::string(name.unwrap_or_default()).unwrap(),
                Symbol::tuple(&args).unwrap(),
            ])
            .unwrap()
        }
        TheoryTerm::Compound { .. } => Symbol::function("unimplemented", &[]).unwrap(),
    }
}

fn theory_get_call(pool: &[Symbol], call: &mut FunctionCall<'_>) -> ClingoxResult<()> {
    if call.name() == "get" {
        for &symbol in pool {
            call.push(symbol)?;
        }
    }
    Ok(())
}

#[test]
fn script_theory() {
    let program = strip_script(lua!("theory.lp"));
    let expected = lua!("theory.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let pool: Vec<Symbol> = {
        let atoms = ctl.theory_atoms().unwrap();
        let mut pool = Vec::new();
        for atom in atoms.iter() {
            let atom = atom.unwrap();
            let mut r = vec![convert_theory_term(&atom.term().unwrap())];
            let mut elements: Vec<Symbol> = atom
                .elements()
                .unwrap()
                .iter()
                .map(|element| {
                    let terms: Vec<Symbol> = element
                        .tuple()
                        .unwrap()
                        .iter()
                        .map(|&id| convert_theory_term(&atoms.term(id).unwrap()))
                        .collect();
                    let condition: Vec<Symbol> = element
                        .condition()
                        .unwrap()
                        .iter()
                        .map(|_| Symbol::string("n/a").unwrap())
                        .collect();
                    Symbol::tuple(&[
                        Symbol::tuple(&terms).unwrap(),
                        Symbol::tuple(&condition).unwrap(),
                    ])
                    .unwrap()
                })
                .collect();
            elements.sort();
            r.push(Symbol::tuple(&elements).unwrap());
            if let Some((operator, rhs)) = atom.guard().unwrap() {
                r.push(
                    Symbol::tuple(&[Symbol::string(operator).unwrap(), convert_theory_term(&rhs)])
                        .unwrap(),
                );
            }
            pool.push(Symbol::tuple(&r).unwrap());
        }
        pool
    };

    let mut run = ScriptRun::new();
    ctl.ground_with(&[Part::new("theory", &[]).unwrap()], |call| {
        theory_get_call(&pool, call)
    })
    .unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ===========================================================================
// A second group of fixtures that were first deferred. Size is not a
// reason to defer; every one below is ported in full.
// ===========================================================================

// ---------------------------------------------------------------------------
// statistics.lp (python) -- ported from MutableStatistics primitives
// (set_value/push_array/add_map_key), not pyclingo's dict-assignment
// sugar, which MutableStatistics has no equivalent for.
// The *observable* tree pyclingo's script builds is reproduced directly:
// checked against the Python oracle, which traces the sugar's own
// merge-not-replace semantics (`accu["x"] = [99, 9]` overwrites the first
// two elements of the existing `[1,2,3]` in place, leaving index 2 (`3`)
// untouched; `.extend` then appends, giving `[99,9,3,4,5,6]`, not
// `[99,9,4,5,6]`; the same merge applies to `y.b`). This port skips
// reproducing the sugar's intermediate steps and builds the final tree
// directly, exactly as `api_statistics_writing.rs` already does for
// `test_conf.py::test_user_stats`.
// ---------------------------------------------------------------------------

/// Reads one statistics entry back into the same ground-term shape
/// `statistics.lp`'s own `tosymbol` builds: a value becomes a `Number`
/// (truncated, matching `int(x)`), an array becomes `list(...)` and a map
/// becomes `dict(...)`, both with one argument per entry (an array's
/// entries in order, a map's `(key, value)` tuples sorted by key, matching
/// `tosymbol`'s own `sorted(x)`). Array-vs-map is told apart the same way
/// `isinstance(x, list)` does: an array's keys are exactly `"0".."n-1"` in
/// order.
fn read_statistics_symbol(stats: &clingox::Statistics<'_>, path: &str) -> Symbol {
    if let Ok(value) = stats.value(path) {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "matching Python's int(x) truncation"
        )]
        return Symbol::number(value as i32);
    }
    let keys = stats.keys(path).unwrap();
    let is_array = keys.iter().enumerate().all(|(i, k)| *k == i.to_string());
    let child_path = |key: &str| -> String {
        if path.is_empty() {
            key.to_owned()
        } else {
            format!("{path}.{key}")
        }
    };
    if is_array {
        let children: Vec<Symbol> = keys
            .iter()
            .map(|k| read_statistics_symbol(stats, &child_path(k)))
            .collect();
        Symbol::function("list", &children).unwrap()
    } else {
        let mut pairs: Vec<(&String, Symbol)> = keys
            .iter()
            .map(|k| (k, read_statistics_symbol(stats, &child_path(k))))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        let tuples: Vec<Symbol> = pairs
            .iter()
            .map(|(k, v)| Symbol::tuple(&[Symbol::string(k).unwrap(), *v]).unwrap())
            .collect();
        Symbol::function("dict", &tuples).unwrap()
    }
}

#[test]
fn script_statistics() {
    use clingox::{MutableStatistics, SolveEventHandler, SolveOptions, StatKind};

    let expected = py!("statistics.sol");

    struct BuildUserAccu;
    impl SolveEventHandler for BuildUserAccu {
        fn on_statistics(
            &mut self,
            _step: &mut MutableStatistics<'_>,
            accu: &mut MutableStatistics<'_>,
        ) -> clingox::Result<std::ops::ControlFlow<()>> {
            accu.add_map_key("", "test", StatKind::Value)?;
            accu.set_value("test", 65.0)?;
            accu.add_map_key("", "x", StatKind::Array)?;
            for v in [99.0, 9.0, 3.0, 4.0, 5.0, 6.0] {
                let i = accu.push_array("x", StatKind::Value)?;
                accu.set_value(&format!("x.{i}"), v)?;
            }
            accu.add_map_key("", "y", StatKind::Map)?;
            accu.add_map_key("y", "a", StatKind::Value)?;
            accu.set_value("y.a", 42.0)?;
            accu.add_map_key("y", "b", StatKind::Array)?;
            for v in [0.0, 2.0, 3.0] {
                let i = accu.push_array("y.b", StatKind::Value)?;
                accu.set_value(&format!("y.b.{i}"), v)?;
            }
            accu.add_map_key("y", "c", StatKind::Value)?;
            accu.set_value("y.c", 99.0)?;
            Ok(std::ops::ControlFlow::Continue(()))
        }
    }

    let program = strip_script(py!("statistics.lp"));
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let mut run = ScriptRun::new();
    let result = ctl
        .solve_with_events(SolveOptions::new(), BuildUserAccu)
        .unwrap();
    run.push_step(vec![Vec::new()], result);

    let symbol = {
        let stats = ctl.statistics().unwrap();
        read_statistics_symbol(&stats, "user_accu")
    };

    ctl.ground_with(&[Part::new("two", &[]).unwrap()], |call| {
        if call.name() == "tomodel" {
            call.push(symbol)?;
        }
        Ok(())
    })
    .unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// cover-py.lp (python; cover-lua.lp is a duplicate, same algorithm, checked
// directly: byte-identical `.sol`, same nested-control incremental search
// and the same final vertex-cover encoding, only the scripting language
// differs) -- a nested, throwaway control `pre` incrementally extends a
// path in a fixed graph (one more "leaf" vertex per step, hard-fixing each
// earlier choice through a `@fix(k)` ground callback) until no further
// extension is possible; the last satisfiable step's model then seeds a
// small vertex-cover subproblem on the outer control `prg` through three
// 0-ary ground callbacks (`@vertex()`, `@edge()`, `@cover()`), each
// returning a pool of results (clingox's ordinary `FunctionCall::push`
// mechanism, called once per element, exactly as a single `@f(...)` call
// already does elsewhere in this file).
//
// Traced against the Python oracle directly (2026-09-27): the incremental
// search stops at step 3 (UNSAT), using step 2's model (the last one
// `on_model` actually saw; the failed step 3 solve never calls it, so the
// recorded atoms are unchanged, exactly like the Python original's own
// `self.last`); `prepare_instance` then reads `edge(2, X, Y)` (`k - 1`),
// giving the 3-vertex, 6-directed-edge remaining subgraph the four
// `cover(...)` models below cover.
// ---------------------------------------------------------------------------

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the incremental search and the final vertex-cover encoding are one fixture"
)]
fn script_cover_py() {
    use std::collections::{BTreeSet, HashMap};

    let expected = py!("cover-py.sol");

    /// Runs `pre`'s current grounding to its one model (the default model
    /// limit is 1, matching upstream's plain `Control()`/`pre.solve(...)`),
    /// records its atoms into `last` (unchanged if the solve is UNSAT, since
    /// `on_model` then never fires, matching upstream), and folds every
    /// `leaf(K, X)` atom seen into `leaves`. Returns whether it was
    /// satisfiable.
    fn solve_and_record(
        pre: &mut Control,
        leaves: &mut HashMap<i32, Symbol>,
        last: &mut Vec<Symbol>,
    ) -> bool {
        let result = pre
            .for_each_model(&[], |model| {
                *last = model.symbols(ShowType::ATOMS)?;
                Ok(ControlFlow::Continue(()))
            })
            .unwrap();
        for atom in last.iter() {
            if atom.name() == Some("leaf") {
                let args = atom.arguments().unwrap();
                leaves.insert(args[0].as_number().unwrap(), args[1]);
            }
        }
        result.is_sat()
    }

    let mut leaves: HashMap<i32, Symbol> = HashMap::new();
    let mut last: Vec<Symbol> = Vec::new();

    let mut pre = Control::new().unwrap();
    pre.add(
        "base",
        &[],
        "edge(a,(b;c)).\n\
         edge(b,(c;d)).\n\
         edge(d,e).\n\
         edge(e,f).\n\
         edge(x,(a;c)).\n\
         edge(X,Y) :- edge(Y,X).\n\
         vertex(X) :- edge(X,Y;Y,X).\n\
         edge(0,X,Y) :- edge(X,Y).\n",
    )
    .unwrap();
    pre.add(
        "step",
        &["k"],
        ":- not leaf(k-1,@fix(k)).\n\
         1 { leaf(k,X) : vertex(X) } 1.\n\
         cover(k,X) :- edge(k-1,X,Y), leaf(k,Y).\n\
         edge(k,X,Y) :- edge(k-1,X,Y), not cover(k,X), not cover(k,Y).\n\
         :- edge(k-1,X,Y), leaf(k,Y), edge(k-1,Y,Z), X < Z.\n\
         :- leaf(k,Y), not edge(k-1,_,Y).\n",
    )
    .unwrap();
    pre.ground(&[Part::base()]).unwrap();

    let mut sat = solve_and_record(&mut pre, &mut leaves, &mut last);
    let mut k: i32 = 0;
    while sat {
        k += 1;
        pre.cleanup().unwrap();
        pre.ground_with(
            &[Part::new("step", &[Symbol::number(k)]).unwrap()],
            |call| {
                if call.name() == "fix" {
                    let arg = call.args()[0].as_number().unwrap();
                    if let Some(&v) = leaves.get(&(arg - 1)) {
                        call.push(v)?;
                    }
                }
                Ok(())
            },
        )
        .unwrap();
        sat = solve_and_record(&mut pre, &mut leaves, &mut last);
    }

    let mut edges: Vec<Symbol> = Vec::new();
    let mut cover: Vec<Symbol> = Vec::new();
    let mut vertices: BTreeSet<Symbol> = BTreeSet::new();
    for atom in &last {
        if atom.name() == Some("edge") && atom.arguments().unwrap().len() == 3 {
            let args = atom.arguments().unwrap();
            if args[0].as_number() == Some(k - 1) {
                edges.push(Symbol::tuple(&[args[1], args[2]]).unwrap());
                vertices.insert(args[1]);
                vertices.insert(args[2]);
            }
        }
        if atom.name() == Some("cover") {
            cover.push(atom.arguments().unwrap()[1]);
        }
    }
    let vertices: Vec<Symbol> = vertices.into_iter().collect();

    let mut prg = Control::with_args(["--models=0"]).unwrap();
    prg.add(
        "base",
        &[],
        "vertex(X) :- X = @vertex().\n\
         edge(X,Y) :- (X,Y) = @edge().\n\
         cover(X)  :- X = @cover().\n\
         { cover(X) : vertex(X) }.\n\
         :- edge(X,Y), not cover(X), not cover(Y).\n\
         #show cover/1.\n",
    )
    .unwrap();
    prg.ground_with(&[Part::base()], |call| {
        match call.name() {
            "vertex" => {
                for &v in &vertices {
                    call.push(v)?;
                }
            }
            "edge" => {
                for &e in &edges {
                    call.push(e)?;
                }
            }
            "cover" => {
                for &c in &cover {
                    call.push(c)?;
                }
            }
            _ => {}
        }
        Ok(())
    })
    .unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut prg, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// project.lp (python) -- an embedded literal event sequence, assembled into
// `seq/2` facts and an `nbs` constant, then a 4-step incremental sequential
// pattern mining search (`base`, then `incr(2)` through `incr(4)`; the
// script's own `Control::get_const("ml")` always finds no such constant in
// this program, so the incremental depth is the fixture's own hard-coded
// default, 4). Ported unchanged: the primed variables (`P'`, `Q'`) are
// ordinary clingo identifiers, not a scripting artifact.
//
// The script's `on_model`/`patterns` bookkeeping has no effect on the ASP
// program or its shown output (`#show pattern/2.` alone decides that); it
// is dropped as dead weight, matching this suite's own convention of
// reproducing the *observable* behaviour, not incidental script-side state
// nothing reads back (as for `domain.lp`).
// ---------------------------------------------------------------------------

#[test]
fn script_project() {
    let program = strip_script(py!("project.lp"));
    let expected = py!("project.sol");

    // The event sequence `readsequence()` embeds as a literal string
    // (positions 1-20 are already strictly increasing, so its duplicate
    // check never triggers; only the second column, the event value, is
    // kept).
    let seq: [i32; 20] = [5, 2, 5, 2, 5, 2, 1, 2, 4, 2, 1, 6, 1, 2, 5, 3, 3, 1, 3, 2];
    let nbs = seq.iter().copied().max().unwrap();

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.configuration().set("solve.project", "auto").unwrap();
    ctl.add_base(&program).unwrap();
    let mut facts = format!("#const nbs = {nbs}.\n");
    for (i, value) in seq.iter().enumerate() {
        use std::fmt::Write as _;
        let _ = writeln!(facts, "seq({},{value}).", i + 1);
    }
    ctl.add_base(&facts).unwrap();

    // `Control::get_const` finding nothing for "ml" (never defined in this
    // program) is exactly what the script's own `if d is None: d = 4`
    // fallback handles; called here for faithfulness, though its result is
    // always `None`.
    assert_eq!(ctl.get_const("ml").unwrap(), None);
    let depth = 4;

    let mut run = ScriptRun::new();
    ctl.ground(&[Part::base()]).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    for k in 2..=depth {
        ctl.ground(&[Part::new("incr", &[Symbol::number(k)]).unwrap()])
            .unwrap();
        let _ = run.solve(&mut ctl, &[]).unwrap();
    }

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// sokoban.lp, sokoban_back.lp, sokoban_para.lp (Lua) -- one shared
// incremental-deepening driver (byte-identical script logic in all three
// fixtures, checked directly: only the ASP program itself, and which
// program part it grounds each step under ("cumulative"), differ), solving
// a Sokoban instance encoded as forward pushes, backward pushes and a
// parallel-push variant respectively. Each step grounds one more
// `cumulative(k)`, moves the `volatile(k)` external forward (releasing the
// previous one and cleaning up first), and stops at the first satisfiable
// step, whose *every* model (there can be more than one plan of the same
// length) becomes that step's own line in the normalised output.
// ---------------------------------------------------------------------------

fn run_sokoban(program: &str) -> ScriptRun {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(program).unwrap();

    let mut run = ScriptRun::new();
    let mut step: i32 = 1;
    loop {
        let mut parts = Vec::new();
        if step > 1 {
            let old = Symbol::function("volatile", &[Symbol::number(step - 1)]).unwrap();
            ctl.release_external(old).unwrap();
            ctl.cleanup().unwrap();
        } else {
            parts.push(Part::base());
        }
        parts.push(Part::new("cumulative", &[Symbol::number(step)]).unwrap());
        ctl.ground(&parts).unwrap();
        let new_volatile = Symbol::function("volatile", &[Symbol::number(step)]).unwrap();
        ctl.assign_external(new_volatile, TruthValue::True).unwrap();
        let result = run.solve(&mut ctl, &[]).unwrap();
        if result.is_sat() {
            break;
        }
        step += 1;
    }
    run
}

#[test]
fn script_sokoban() {
    let program = strip_script(lua!("sokoban.lp"));
    let expected = lua!("sokoban.sol");
    assert_script_matches(run_sokoban(&program), expected);
}

#[test]
fn script_sokoban_back() {
    let program = strip_script(lua!("sokoban_back.lp"));
    let expected = lua!("sokoban_back.sol");
    assert_script_matches(run_sokoban(&program), expected);
}

#[test]
fn script_sokoban_para() {
    let program = strip_script(lua!("sokoban_para.lp"));
    let expected = lua!("sokoban_para.sol");
    assert_script_matches(run_sokoban(&program), expected);
}

// ---------------------------------------------------------------------------
// sokoban.lp (python; `.cmd`: `-q1,2`, print verbosity only) -- a different,
// harder Sokoban variant from the Lua family above: optimising
// (`--opt-mode=optN`, two `#minimize` statements ordered by priority) over
// an incremental `state(t)`/`trans(t)` split, and the driving loop keeps
// going for one extra step past the first satisfiable one (`e` counts down
// from 2, not 1) before stopping, plus a `t > 20` unsatisfiable escape
// hatch. Ported literally: releasing `volatile(t-1)` on the very first
// iteration (`t=0`, so `volatile(-1)`) is a safe no-op, checked directly,
// since that atom was never even part of any grounding.
// ---------------------------------------------------------------------------

/// Solves the current grounding under `optN`, keeping only the models the
/// real application's `-q1` would print.
///
/// Traced directly against the vendored `clasp` source:
/// `Output::onModel` (`clasp/src/
/// clasp_output.cpp`) computes `type = (m.opt == 1 && !consequences) ||
/// m.def ? print_best : print_all`, and only prints when `modelQ() <=
/// type`; `-q1` sets `modelQ() == print_best (1)`, so a model prints
/// exactly when `type == print_best`, i.e. exactly when clasp's own
/// `Model::opt` bit is set. `enumerator.h`'s own doc comment on that field
/// is exact: "whether the model is optimal w.r.t costs (0: unknown)" --
/// clasp sets it only once a model is *known* optimal, not merely tied
/// with the best cost seen so far, which an improving model reported
/// before optimality is proven can also be. `Model::optimality_proven`
/// (`clingo_model_optimality_proven`) mirrors this bit exactly, so
/// filtering on it, not on cost equality with the last model, is the
/// direct port: checked against pyclingo 5.8.2 on the sequence-mining
/// fixture below, where a same-cost-but-not-yet-optimal model and a
/// genuinely optimal one happen to share the same cost value, which a
/// cost-equality filter cannot tell apart but `optimality_proven` can.
fn solve_quiet_except_optimum(ctl: &mut Control) -> (Vec<Vec<String>>, SolveResult) {
    let mut final_models: Vec<Vec<String>> = Vec::new();
    let result = ctl
        .for_each_model(&[], |model| {
            if model.optimality_proven()? {
                let mut atoms: Vec<String> = model
                    .symbols(ShowType::SHOWN)?
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                atoms.sort();
                final_models.push(atoms);
            }
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    (final_models, result)
}

#[test]
fn script_sokoban_optimizing() {
    let program = strip_script(py!("sokoban.lp"));
    let expected = py!("sokoban.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.configuration().set("solve.opt_mode", "optN").unwrap();
    ctl.add_base(&program).unwrap();

    let mut run = ScriptRun::new();
    let mut t: i32 = 0;
    let mut e = 2;
    let mut parts = vec![
        Part::base(),
        Part::new("state", &[Symbol::number(t)]).unwrap(),
    ];
    loop {
        ctl.ground(&parts).unwrap();
        let old = Symbol::function("volatile", &[Symbol::number(t - 1)]).unwrap();
        ctl.release_external(old).unwrap();
        ctl.cleanup().unwrap();
        let new_volatile = Symbol::function("volatile", &[Symbol::number(t)]).unwrap();
        ctl.assign_external(new_volatile, TruthValue::True).unwrap();

        let (final_models, result) = solve_quiet_except_optimum(&mut ctl);
        let sat = result.is_sat();
        run.push_optimizing_step(final_models, result);

        if !sat && t > 20 {
            break;
        }
        if sat {
            e -= 1;
            if e == 0 {
                break;
            }
        }
        t += 1;
        parts = vec![
            Part::new("trans", &[Symbol::number(t)]).unwrap(),
            Part::new("state", &[Symbol::number(t)]).unwrap(),
        ];
    }

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// conformant1.lp, conformant2.lp, conformant3.lp (Lua) -- one shared driver
// (byte-identical script logic in all three, checked directly), solving a
// conformant-planning instance: at each step, ground the newest `step(t)`/
// `state(t)` parts, solve, and once a candidate plan is found, ground and
// solve `check(t)` once (verifying it against every admissible initial
// state under uncertainty) before accepting it. `Control::get_const
// ("nocheck")` never finds such a constant in any of the three fixtures
// (checked directly: grepping the submodule), so the check step always
// runs on the first candidate. Ported literally, including the fixture's
// own quirk once a check has been attempted: `check` stays `true`
// afterward regardless of whether that check passed, so a later step's
// first candidate is accepted without a further check.
// ---------------------------------------------------------------------------

fn run_conformant(program: &str) -> ScriptRun {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(program).unwrap();

    let mut run = ScriptRun::new();
    let mut step: i32 = 0;
    let mut check = false;
    loop {
        let mut parts = Vec::new();
        if step > 0 {
            parts.push(Part::new("step", &[Symbol::number(step)]).unwrap());
        } else {
            parts.push(Part::base());
        }
        parts.push(Part::new("state", &[Symbol::number(step)]).unwrap());
        if check {
            parts.push(Part::new("check", &[Symbol::number(step)]).unwrap());
        }
        let old = Symbol::function("vol", &[Symbol::number(step - 1)]).unwrap();
        ctl.release_external(old).unwrap();
        ctl.cleanup().unwrap();
        ctl.ground(&parts).unwrap();
        let new_vol = Symbol::function("vol", &[Symbol::number(step)]).unwrap();
        ctl.assign_external(new_vol, TruthValue::True).unwrap();
        let result = run.solve(&mut ctl, &[]).unwrap();
        if result.is_sat() {
            if !check && ctl.get_const("nocheck").unwrap().is_none() {
                check = true;
                ctl.cleanup().unwrap();
                ctl.ground(&[Part::new("check", &[Symbol::number(step)]).unwrap()])
                    .unwrap();
                let check_result = run.solve(&mut ctl, &[]).unwrap();
                if check_result.is_sat() {
                    break;
                }
            } else {
                break;
            }
        }
        step += 1;
    }
    run
}

#[test]
fn script_conformant1() {
    let program = strip_script(lua!("conformant1.lp"));
    let expected = lua!("conformant1.sol");
    assert_script_matches(run_conformant(&program), expected);
}

#[test]
fn script_conformant2() {
    let program = strip_script(lua!("conformant2.lp"));
    let expected = lua!("conformant2.sol");
    assert_script_matches(run_conformant(&program), expected);
}

#[test]
fn script_conformant3() {
    let program = strip_script(lua!("conformant3.lp"));
    let expected = lua!("conformant3.sol");
    assert_script_matches(run_conformant(&program), expected);
}

// ---------------------------------------------------------------------------
// test.lp (python and lua) -- the script's own `print`/`writeln` calls are
// interleaved with the real clingo application's own model output in the
// upstream `.sol`, which the app produces regardless of whether the script
// also registers a model callback (`run.py`'s `normalize()` treats every
// printed block the same way: text after an "Answer: N" marker becomes one
// line, and every line within a step is sorted together, matching this
// fixture's own `Step: 6` block, whose observed order -- `SAT`, then the
// real atom line, then the script's own `hasA(...)` line, then `on_finish`
// -- is exactly ASCII order, `S` < `a` < `h` < `o`).
//
// `ScriptRun` needs no new API for this: `normalise_fixture` already sorts
// and joins each "model" (`Vec<String>`) independently before sorting the
// step's own lines, so a script-printed line, wrapped as its own
// single-element `vec![line]`, joins back to exactly that line unchanged
// and takes part in the same per-step sort as a real model's atom list.
// `push_step` accepts that directly; no harness change was needed once the
// representation was seen this way.
// ---------------------------------------------------------------------------

/// `hasA(...)`/`hasVolatile(...)`/`model(...)`, matching both scripts'
/// `on_model` exactly (Lua's `tostring(bool)` and Python's
/// `str(bool).lower()` both give `true`/`false`).
fn test_fixture_on_model_line(model: &clingox::Model) -> String {
    let a = Symbol::function("a", &[]).unwrap();
    let volatile9 = Symbol::function("volatile", &[Symbol::number(9)]).unwrap();
    let has_a = model.contains(a).unwrap();
    let has_volatile = model.contains(volatile9).unwrap();
    let mut shown: Vec<String> = model
        .symbols(ShowType::SHOWN)
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    shown.sort();
    // Both scripts' own literal ends in a trailing space after the closing
    // paren (`"model(" .. ... .. ") "`); `run.py`'s own line handling does
    // not keep it, checked directly against the fixture's `.sol`.
    format!(
        "hasA({has_a}) hasVolatile({has_volatile}) model({})",
        shown.join(",")
    )
}

fn test_fixture_base_and_test_part(
    ctl: &mut Control,
    run: &mut ScriptRun,
    program: &str,
) -> Symbol {
    ctl.add_base(program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let _ = run.solve(ctl, &[]).unwrap();
    ctl.add("test", &["x"], "test(x).").unwrap();
    Symbol::function("f", &[Symbol::number(1), Symbol::number(2)]).unwrap()
}

#[test]
fn script_test_python() {
    use clingox::{ExtendableModel, SolveEventHandler, SolveOptions};

    let program = strip_script(py!("test.lp"));
    let expected = py!("test.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    let mut run = ScriptRun::new();
    let f12 = test_fixture_base_and_test_part(&mut ctl, &mut run, &program);
    let n = ctl.get_const("n").unwrap().unwrap().as_number().unwrap();

    // `solve_with_events` always closes its search and drops the handler
    // before returning, so it is the one solve form that may borrow the
    // caller's own locals for the search's whole lifetime (see
    // the documentation on the three event-taking solve methods); borrowing
    // this step's own `lines` directly is simpler than owning a copy and
    // reading it back out afterward.
    struct Handler<'a> {
        lines: &'a mut Vec<Vec<String>>,
    }
    impl SolveEventHandler for Handler<'_> {
        fn on_model(
            &mut self,
            model: &mut ExtendableModel<'_>,
        ) -> clingox::Result<ControlFlow<()>> {
            let mut atoms: Vec<String> = model
                .symbols(ShowType::SHOWN)?
                .iter()
                .map(ToString::to_string)
                .collect();
            atoms.sort();
            self.lines.push(atoms);
            self.lines.push(vec![test_fixture_on_model_line(model)]);
            Ok(ControlFlow::Continue(()))
        }
        fn on_finish(&mut self, _result: clingox::SolveResult) -> clingox::Result<ControlFlow<()>> {
            self.lines.push(vec!["on_finish".to_owned()]);
            Ok(ControlFlow::Continue(()))
        }
    }

    let mut parts = vec![Part::new("test", &[f12]).unwrap()];
    for i in 1..=n {
        parts.push(Part::new("cumulative", &[Symbol::number(i)]).unwrap());
        ctl.ground(&parts).unwrap();
        parts = Vec::new();

        let mut lines: Vec<Vec<String>> = Vec::new();
        let result = ctl
            .solve_with_events(SolveOptions::new(), Handler { lines: &mut lines })
            .unwrap();
        lines.push(vec![if result.is_sat() {
            "SAT".to_owned()
        } else if result.is_unsat() {
            "UNSAT".to_owned()
        } else {
            "UNKNOWN".to_owned()
        }]);
        run.push_step(lines, result);
    }

    assert_script_matches(run, expected);
}

#[test]
fn script_test_lua() {
    let program = strip_script(lua!("test.lp"));
    let expected = lua!("test.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    let mut run = ScriptRun::new();
    let f12 = test_fixture_base_and_test_part(&mut ctl, &mut run, &program);
    let n = ctl.get_const("n").unwrap().unwrap().as_number().unwrap();

    let mut parts = vec![Part::new("test", &[f12]).unwrap()];
    for i in 1..=n {
        parts.push(Part::new("cumulative", &[Symbol::number(i)]).unwrap());
        ctl.ground(&parts).unwrap();
        parts = Vec::new();

        let mut lines: Vec<Vec<String>> = Vec::new();
        let result = ctl
            .for_each_model(&[], |model| {
                let mut atoms: Vec<String> = model
                    .symbols(ShowType::SHOWN)?
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                atoms.sort();
                lines.push(atoms);
                lines.push(vec![test_fixture_on_model_line(model)]);
                Ok(ControlFlow::Continue(()))
            })
            .unwrap();
        lines.push(vec![if result.is_sat() {
            "SAT".to_owned()
        } else if result.is_unsat() {
            "UNSAT".to_owned()
        } else {
            "UNKNOWN".to_owned()
        }]);
        run.push_step(lines, result);
    }

    assert_script_matches(run, expected);
}

// ===========================================================================
// the propagator fixtures. `check-py.lp`/`tag.lp` and
// `test-numeric.lp`-style single-purpose fixtures aside, every remaining
// `app/clingo/tests/{python,lua}/` propagator row is
// ported below, through the same `ScriptRun` machinery as the earlier fixtures.
//
// Every Lua fixture among them turned
// out to be either a duplicate of its Python original (byte-identical ASP
// program and `.sol`, only the scripting idiom differing, checked directly
// by diffing both) or a duplicate of `check-py.lp` (`check-lua.lp`); see
// `clingox/tests/conformance/NOT_PORTED.md` for the
// per-fixture diff notes. None gets its own Rust port here, matching the
// existing duplicate convention.
// ===========================================================================

/// A 0-ary atom's own solver literal, for a propagator's `init`.
fn m3_solver_literal(
    init: &clingox::propagate::PropagateInit<'_>,
    name: &str,
) -> ClingoxResult<clingox::propagate::SolverLiteral> {
    let sig = clingox::Signature::new(name, 0)?;
    let plit = init
        .symbolic_atoms()?
        .by_signature(sig)
        .next()
        .unwrap_or_else(|| panic!("{name} is an atom"))?
        .literal();
    init.solver_literal(plit)
}

// ---------------------------------------------------------------------------
// add-clause-py.lp (python), add-clause-lua.lp (Lua duplicate: byte-
// identical program and script logic, checked directly, `.sol` identical
// too -- ported through this test only) -- `PropagateInit::add_clause`/
// `propagate` across three solving steps: a biconditional chain linking `a`
// to `b`/`c` through a fresh literal (step 0), contradictory-looking but
// harmless repeated unit clauses (step 1), and an empty clause -- an
// immediate, unconditional conflict -- followed by more calls that can no
// longer matter (step 2).
// ---------------------------------------------------------------------------

#[test]
fn script_add_clause_py() {
    use clingox::propagate::{PropagateInit, Propagator};

    let program = strip_script(py!("add-clause-py.lp"));
    let expected = py!("add-clause-py.sol");

    struct AddClauseStepper {
        step: Mutex<u32>,
    }

    impl Propagator for AddClauseStepper {
        fn init(&self, init: &mut PropagateInit<'_>) -> ClingoxResult<()> {
            let a = m3_solver_literal(init, "a")?;
            let b = m3_solver_literal(init, "b")?;
            let c = m3_solver_literal(init, "c")?;
            let mut step = self.step.lock().unwrap();
            match *step {
                0 => {
                    let l = init.add_literal(true)?;
                    init.add_clause(&[-b, l])?;
                    init.propagate()?;
                    init.add_clause(&[-l, b])?;
                    init.propagate()?;
                    init.add_clause(&[-c, l])?;
                    init.propagate()?;
                    init.add_clause(&[-l, c])?;
                    init.propagate()?;
                    init.add_clause(&[-a, -b])?;
                    init.propagate()?;
                    init.add_clause(&[-a, b])?;
                    init.propagate()?;
                    init.add_clause(&[-a, -a, -b])?;
                    init.propagate()?;
                    init.add_clause(&[-a, a])?;
                    init.propagate()?;
                }
                1 => {
                    init.add_clause(&[b])?;
                    init.propagate()?;
                    init.add_clause(&[b])?;
                    init.propagate()?;
                }
                _ => {
                    // The empty clause is an unconditional conflict: clingo
                    // reports `Flow::Stop` for it (or, failing that, for the
                    // `propagate()` right after), and clingox then refuses any
                    // further call on this `init` with
                    // `ErrorKind::InvalidInput` (`api_propagator_init.rs::
                    // a_further_call_after_stop_is_refused_by_clingox_itself`).
                    // Upstream's pyclingo keeps calling regardless and
                    // tolerates it, returning `False` each time; this port
                    // stops issuing further calls once `Stop` is seen instead,
                    // since the fixture's own outcome (`UNSAT`) is already
                    // settled by the empty clause alone.
                    let mut ok = !init.add_clause(&[])?.is_stop();
                    if ok {
                        ok = !init.propagate()?.is_stop();
                    }
                    if ok {
                        ok = !init.add_clause(&[a])?.is_stop();
                    }
                    if ok {
                        ok = !init.propagate()?.is_stop();
                    }
                    if ok {
                        ok = !init.add_clause(&[b])?.is_stop();
                    }
                    if ok {
                        ok = !init.propagate()?.is_stop();
                    }
                    if ok {
                        ok = !init.add_clause(&[-a])?.is_stop();
                    }
                    if ok {
                        ok = !init.propagate()?.is_stop();
                    }
                    if ok {
                        ok = !init.add_clause(&[-b])?.is_stop();
                    }
                    if ok {
                        let _ = init.propagate()?;
                    }
                }
            }
            *step += 1;
            Ok(())
        }
    }

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(AddClauseStepper {
        step: Mutex::new(0),
    })
    .unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// add_watch.lp (python and lua: byte-identical program and `.sol`; Lua's
// own script watches thread 1/asserts `thread_id == 1` where Python watches
// thread 0/asserts `thread_id == 0`, the Lua binding's own 1-based
// convention for an otherwise identical single-threaded test -- ported
// through this test only) -- `PropagateInit::add_watch` plus `Assignment`
// reads at `init` time (`a` free, `b` forced false, `c` a fact), and every
// `propagate` change is the watched literal.
// ---------------------------------------------------------------------------

#[test]
fn script_add_watch() {
    use clingox::propagate::{PropagateControl, PropagateInit, Propagator, SolverLiteral};

    let program = strip_script(py!("add_watch.lp"));
    let expected = py!("add_watch.sol");

    struct WatchesA {
        watched: Mutex<Option<SolverLiteral>>,
    }

    impl Propagator for WatchesA {
        fn init(&self, init: &mut PropagateInit<'_>) -> ClingoxResult<()> {
            let a = m3_solver_literal(init, "a")?;
            let b = m3_solver_literal(init, "b")?;
            let c = m3_solver_literal(init, "c")?;
            init.add_watch(a)?;
            let assignment = init.assignment();
            assert_eq!(assignment.truth_value(a)?, None);
            assert!(assignment.is_false(b)?);
            assert!(assignment.is_true(c)?);
            *self.watched.lock().unwrap() = Some(a);
            Ok(())
        }

        fn propagate(
            &self,
            control: &mut PropagateControl<'_>,
            changes: &[SolverLiteral],
        ) -> ClingoxResult<()> {
            assert_eq!(control.thread_id(), 0);
            let a = self.watched.lock().unwrap().expect("init ran first");
            for &lit in changes {
                assert_eq!(lit, a);
            }
            Ok(())
        }
    }

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(WatchesA {
        watched: Mutex::new(None),
    })
    .unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// add_weight.lp (python and lua: byte-identical program, script logic and
// `.sol`, checked directly -- ported through this test only) --
// `PropagateInit::add_weight_constraint`'s default kind (pyclingo's own
// `type_=0`, `WeightConstraintKind::Equivalence`) linking a fresh literal
// both ways to two two-element weight-2 constraints.
// ---------------------------------------------------------------------------

#[test]
fn script_add_weight() {
    use clingox::propagate::{PropagateInit, Propagator, WeightConstraintKind};

    let program = strip_script(py!("add_weight.lp"));
    let expected = py!("add_weight.sol");

    struct AddsWeight;
    impl Propagator for AddsWeight {
        fn init(&self, init: &mut PropagateInit<'_>) -> ClingoxResult<()> {
            let a = m3_solver_literal(init, "a")?;
            let b = m3_solver_literal(init, "b")?;
            let c = m3_solver_literal(init, "c")?;
            let l = init.add_literal(true)?;
            init.add_weight_constraint(
                l,
                &[(a, 1), (b, 1)],
                2,
                WeightConstraintKind::Equivalence,
                false,
            )?;
            init.add_weight_constraint(
                -l,
                &[(b, 1), (c, 1)],
                2,
                WeightConstraintKind::Equivalence,
                false,
            )?;
            Ok(())
        }
    }

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(AddsWeight).unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// add_minimize.lp (python and lua: byte-identical program, script logic and
// `.sol`, checked directly -- ported through this test only) --
// `PropagateInit::add_minimize` extends the solver's own minimize
// constraint with four weighted literals at the default priority.
//
// `.cmd`: `--opt-mode=optN -q1,1`. `-q1` is `run.py`'s own application-level
// quiet flag, with no libclingo equivalent: `solve_quiet_except_optimum`
// (established by `script_sokoban_optimizing`) reproduces its observed
// semantics, keeping only the models tied at the final proven-optimal cost.
// ---------------------------------------------------------------------------

#[test]
fn script_add_minimize() {
    use clingox::propagate::{PropagateInit, Propagator};

    let program = strip_script(py!("add_minimize.lp"));
    let expected = py!("add_minimize.sol");

    struct AddsMinimize;
    impl Propagator for AddsMinimize {
        fn init(&self, init: &mut PropagateInit<'_>) -> ClingoxResult<()> {
            let a = m3_solver_literal(init, "a")?;
            let b = m3_solver_literal(init, "b")?;
            let c = m3_solver_literal(init, "c")?;
            let d = m3_solver_literal(init, "d")?;
            init.add_minimize(a, 1, 0)?;
            init.add_minimize(b, 1, 0)?;
            init.add_minimize(c, 1, 0)?;
            init.add_minimize(d, 1, 0)?;
            Ok(())
        }
    }

    let mut ctl = Control::with_args(["--models=0", "--opt-mode=optN"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(AddsMinimize).unwrap();

    let mut run = ScriptRun::new();
    let (models, result) = solve_quiet_except_optimum(&mut ctl);
    run.push_optimizing_step(models, result);

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// assignment.lp (python and lua) -- deep `Assignment`/`Trail` consistency
// checks, run once from `init` (decision level 0) and once per `check`
// (`CheckMode::Fixpoint`): every level's own trail slice is entirely true,
// the trail and the assignment agree on their sizes, and every assigned
// literal appears on the trail exactly once under its current sign.
//
// Lua's own script additionally indexes `assignment[i]`/`trail[i]` through
// `__pairs`/`__ipairs` metamethods; those are pure Lua indexing sugar over
// the same values `Assignment::at`/`Trail::at` already read here (the plain
// `at`, used throughout this file's ported fixtures), not a distinct
// invariant, so Lua's fixture is a duplicate of this port, not a second one.
// ---------------------------------------------------------------------------

#[test]
fn script_assignment() {
    use clingox::propagate::{Assignment, CheckMode, PropagateControl, PropagateInit, Propagator};

    let program = strip_script(py!("assignment.lp"));
    let expected = py!("assignment.sol");

    fn check_lit(
        assignment: &Assignment<'_>,
        l: clingox::propagate::SolverLiteral,
    ) -> ClingoxResult<()> {
        if let Some(truth) = assignment.truth_value(l)? {
            let signed = if truth { l } else { -l };
            let mut n = 0;
            for k in &assignment.trail() {
                if k? == signed {
                    n += 1;
                }
            }
            assert_eq!(n, 1);
        }
        Ok(())
    }

    fn check_trail(assignment: &Assignment<'_>) -> ClingoxResult<()> {
        let trail = assignment.trail();
        let mut n = 0u32;
        for level in 0..=assignment.decision_level() {
            let level_lits = trail.level(level)?;
            assert!(
                !level_lits.is_empty(),
                "every decision level up to the current one has at least one literal"
            );
            for &lit in &level_lits {
                assert!(assignment.is_true(lit)?);
            }
            n += u32::try_from(level_lits.len()).unwrap();
        }
        assert_eq!(n, trail.size()?);

        let mut n = 0usize;
        for offset in 0..assignment.size() {
            let l = assignment.at(offset)?;
            n += 1;
            check_lit(assignment, l)?;
        }
        assert_eq!(n, assignment.size());
        Ok(())
    }

    struct ChecksAssignment;
    impl Propagator for ChecksAssignment {
        fn init(&self, init: &mut PropagateInit<'_>) -> ClingoxResult<()> {
            check_trail(&init.assignment())?;
            init.set_check_mode(CheckMode::Fixpoint);
            Ok(())
        }

        fn check(&self, control: &mut PropagateControl<'_>) -> ClingoxResult<()> {
            check_trail(&control.assignment())
        }
    }

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(ChecksAssignment).unwrap();

    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();

    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// wc1.lp to wc6.lp (python and lua: byte-identical program, script logic
// and `.sol` for every one of the six, checked directly) -- every
// combination of `WeightConstraintKind` (the C level's `-1`/`0`/`1`, wc2/
// wc3/wc1 and wc5/wc6/wc4) and `compare_equal` (`false` for wc1-3, `true`
// for wc4-6) on `add_weight_constraint(a, [(b,1),(c,1)], 1, kind,
// compare_equal)`, enumerating every model of `{a;b;c}.` under each: the
// six own `.sol` files differ from each other (unlike the plainer coverage
// in `api_propagator_init.rs`'s own `weight_constraint_*` tests, which only
// check one bound and `compare_equal = false`), so all six are ported.
// ---------------------------------------------------------------------------

fn run_wc(
    program: &str,
    kind: clingox::propagate::WeightConstraintKind,
    compare_equal: bool,
) -> ScriptRun {
    use clingox::propagate::{PropagateInit, Propagator, WeightConstraintKind};

    struct Wc {
        kind: WeightConstraintKind,
        compare_equal: bool,
    }
    impl Propagator for Wc {
        fn init(&self, init: &mut PropagateInit<'_>) -> ClingoxResult<()> {
            let a = m3_solver_literal(init, "a")?;
            let b = m3_solver_literal(init, "b")?;
            let c = m3_solver_literal(init, "c")?;
            init.add_weight_constraint(a, &[(b, 1), (c, 1)], 1, self.kind, self.compare_equal)?;
            Ok(())
        }
    }

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base(program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(Wc {
        kind,
        compare_equal,
    })
    .unwrap();
    let mut run = ScriptRun::new();
    let _ = run.solve(&mut ctl, &[]).unwrap();
    run
}

#[test]
fn script_wc1() {
    use clingox::propagate::WeightConstraintKind;
    let program = strip_script(py!("wc1.lp"));
    let expected = py!("wc1.sol");
    let run = run_wc(&program, WeightConstraintKind::ImplicationRight, false);
    assert_script_matches(run, expected);
}

#[test]
fn script_wc2() {
    use clingox::propagate::WeightConstraintKind;
    let program = strip_script(py!("wc2.lp"));
    let expected = py!("wc2.sol");
    let run = run_wc(&program, WeightConstraintKind::ImplicationLeft, false);
    assert_script_matches(run, expected);
}

#[test]
fn script_wc3() {
    use clingox::propagate::WeightConstraintKind;
    let program = strip_script(py!("wc3.lp"));
    let expected = py!("wc3.sol");
    let run = run_wc(&program, WeightConstraintKind::Equivalence, false);
    assert_script_matches(run, expected);
}

#[test]
fn script_wc4() {
    use clingox::propagate::WeightConstraintKind;
    let program = strip_script(py!("wc4.lp"));
    let expected = py!("wc4.sol");
    let run = run_wc(&program, WeightConstraintKind::ImplicationRight, true);
    assert_script_matches(run, expected);
}

#[test]
fn script_wc5() {
    use clingox::propagate::WeightConstraintKind;
    let program = strip_script(py!("wc5.lp"));
    let expected = py!("wc5.sol");
    let run = run_wc(&program, WeightConstraintKind::ImplicationLeft, true);
    assert_script_matches(run, expected);
}

#[test]
fn script_wc6() {
    use clingox::propagate::WeightConstraintKind;
    let program = strip_script(py!("wc6.lp"));
    let expected = py!("wc6.sol");
    let run = run_wc(&program, WeightConstraintKind::Equivalence, true);
    assert_script_matches(run, expected);
}

// ---------------------------------------------------------------------------
// propagator.lp (python and lua: identical embedded ASP program, the same
// sequence-mining algorithm reimplemented in Lua, checked directly -- ported
// through this test only) -- a propagator over `&seq/1` (body) and `&pat/0`
// (directive) theory atoms that finds every length-`n` subsequence common to
// three sequences, matching `libclingo/tests/propagator.cc`'s
// `SequenceMiningPropagator`/`sequence_mining_encoding` line for line (that
// C++ `propgator-sequence-mining` TEST_CASE is counted ported through this
// test too: same
// algorithm, same embedded ASP program, checked directly by diffing both
// against this port's own literal transcription).
//
// `.cmd`: `-q1,1`, handled the same way as `add_minimize.lp` above
// (`solve_quiet_except_optimum`). The script's own `if prg.get_const("n")
// .number == 0` backend-pruning branch is dead for this fixture (`#const n
// = 5.` is in the embedded program itself, so `get_const("n")` is always
// `Some(5)`, checked directly); it is not ported.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SeqState {
    seq_active: Vec<bool>,
    // A pattern index (`>= 0`) or `-(sequence index) - 1` (`< 0`), matching
    // the upstream `StackItem` encoding.
    stack: Vec<i64>,
    trail: Vec<(u32, usize)>,
    pat: Vec<Option<(clingox::propagate::SolverLiteral, usize)>>,
    pat_assigned: usize,
}

impl SeqState {
    fn new(pat_len: usize, seq_len: usize) -> SeqState {
        SeqState {
            seq_active: vec![true; seq_len],
            stack: Vec::new(),
            trail: Vec::new(),
            pat: vec![None; pat_len],
            pat_assigned: 0,
        }
    }
}

#[derive(Default)]
struct SeqInner {
    seq_atoms: Vec<Option<(clingox::propagate::SolverLiteral, Vec<usize>)>>,
    pat_atoms: std::collections::HashMap<clingox::propagate::SolverLiteral, Vec<(usize, usize)>>,
    occurrence_list: Vec<Vec<usize>>,
    item_map: std::collections::HashMap<String, usize>,
    pattern_length: usize,
    states: Vec<SeqState>,
}

impl SeqInner {
    fn map_item(&mut self, item: String) -> usize {
        let next = self.item_map.len();
        *self.item_map.entry(item).or_insert(next)
    }
}

#[derive(Default)]
struct SequenceMining {
    inner: Mutex<SeqInner>,
}

fn seq_add_clause_and_propagate(
    control: &mut clingox::propagate::PropagateControl<'_>,
    clause: &[clingox::propagate::SolverLiteral],
) -> ClingoxResult<bool> {
    if control
        .add_clause(clause, clingox::propagate::ClauseType::Learnt)?
        .is_stop()
    {
        return Ok(false);
    }
    Ok(!control.propagate()?.is_stop())
}

fn seq_propagate_sequence_literal(
    inner: &mut SeqInner,
    control: &mut clingox::propagate::PropagateControl<'_>,
    thread: usize,
    sid: usize,
    lit: clingox::propagate::SolverLiteral,
) -> ClingoxResult<bool> {
    {
        let state = &mut inner.states[thread];
        state.seq_active[sid] = false;
        state.stack.push(-(i64::try_from(sid).unwrap()) - 1);
    }
    if !control.assignment().is_true(lit)? {
        let mut clause = vec![lit];
        for (plit, _) in inner.states[thread].pat.iter().flatten() {
            clause.push(-*plit);
        }
        return seq_add_clause_and_propagate(control, &clause);
    }
    Ok(true)
}

fn seq_propagate_sequence(
    inner: &mut SeqInner,
    control: &mut clingox::propagate::PropagateControl<'_>,
    thread: usize,
    sid: usize,
) -> ClingoxResult<bool> {
    let pattern_length = inner.pattern_length;
    let (lit, items) = {
        let seq = inner.seq_atoms[sid]
            .as_ref()
            .expect("this sequence's own atom is known");
        (seq.0, seq.1.clone())
    };
    let pat_assigned = inner.states[thread].pat_assigned;
    if pat_assigned < pattern_length && control.assignment().is_false(lit)? {
        return Ok(true);
    }
    let pat_items: Vec<Option<usize>> = inner.states[thread]
        .pat
        .iter()
        .map(|p| p.map(|(_, item)| item))
        .collect();
    let mut iid = 0usize;
    for pat_item in &pat_items {
        loop {
            if iid == items.len() {
                return seq_propagate_sequence_literal(inner, control, thread, sid, -lit);
            }
            iid += 1;
            if pat_item.is_none() || items[iid - 1] == pat_item.unwrap() {
                break;
            }
        }
    }
    if pat_assigned < pattern_length {
        Ok(true)
    } else {
        seq_propagate_sequence_literal(inner, control, thread, sid, lit)
    }
}

struct SeqEntry {
    sid: usize,
    plit: ProgramLiteral,
    items: Vec<(i32, String)>,
}
struct PatEntry {
    plit: ProgramLiteral,
    index: i32,
    item: String,
}

/// Reads every `&seq(U){...}`/`&pat{...}` theory atom of the current
/// grounding, resolved into plain data (no `init`/`PropagateInit` borrow
/// held past this call, so the caller is free to call `solver_literal`/
/// `add_watch` afterward).
fn seq_collect_theory_atoms(
    init: &clingox::propagate::PropagateInit<'_>,
) -> ClingoxResult<(Vec<SeqEntry>, Vec<PatEntry>)> {
    let mut seq_entries: Vec<SeqEntry> = Vec::new();
    let mut pat_entries: Vec<PatEntry> = Vec::new();
    let atoms = init.theory_atoms()?;
    for atom in atoms.iter() {
        let atom = atom?;
        let term = atom.term()?;
        // A 0-arity theory function (`&pat{...}`'s own designating
        // term, `pat/0`) comes back as `TheoryTerm::Symbol`, not an
        // empty-argument `Compound` (checked directly: `&seq(U){...}`
        // with its one argument is a `Compound`, but `&pat{...}` is
        // not); both are handled uniformly here.
        let empty: Vec<TheoryTerm<'_>> = Vec::new();
        let (name, arguments): (&str, &[TheoryTerm<'_>]) = match &term {
            TheoryTerm::Compound {
                name: Some(name),
                arguments,
                ..
            } => (name, arguments.as_slice()),
            TheoryTerm::Symbol(sym) => match sym.name() {
                Some(name) => (name, &empty),
                None => continue,
            },
            _ => continue,
        };
        if name == "seq" && arguments.len() == 1 {
            let TheoryTerm::Number(sid) = arguments[0] else {
                continue;
            };
            let plit = atom.literal()?.expect("a seq theory atom has a literal");
            let mut items = Vec::new();
            for elem in atom.elements()? {
                let tuple = elem.tuple()?;
                let TheoryTerm::Number(index) = atoms.term(tuple[0])? else {
                    panic!("a seq element's first term is a number")
                };
                let item_name = atoms.term(tuple[1])?.to_string();
                items.push((index, item_name));
            }
            seq_entries.push(SeqEntry {
                sid: usize::try_from(sid).unwrap(),
                plit,
                items,
            });
        } else if name == "pat" && arguments.is_empty() {
            for elem in atom.elements()? {
                let plit = elem
                    .condition_id()?
                    .expect("a pat element has a condition literal");
                let tuple = elem.tuple()?;
                let TheoryTerm::Number(index) = atoms.term(tuple[0])? else {
                    panic!("a pat element's first term is a number")
                };
                let item_name = atoms.term(tuple[1])?.to_string();
                pat_entries.push(PatEntry {
                    plit,
                    index,
                    item: item_name,
                });
            }
        }
    }
    Ok((seq_entries, pat_entries))
}

impl clingox::propagate::Propagator for SequenceMining {
    fn init(&self, init: &mut clingox::propagate::PropagateInit<'_>) -> ClingoxResult<()> {
        let mut inner = self.inner.lock().unwrap();
        let (seq_entries, pat_entries) = seq_collect_theory_atoms(init)?;

        for entry in seq_entries {
            let slit = init.solver_literal(entry.plit)?;
            if inner.seq_atoms.len() <= entry.sid {
                inner.seq_atoms.resize(entry.sid + 1, None);
            }
            let max_index = entry.items.iter().map(|(i, _)| *i).max().unwrap_or(-1);
            let mut items = vec![0usize; usize::try_from(max_index + 1).unwrap_or(0)];
            for (index, item_name) in entry.items {
                let item_id = inner.map_item(item_name);
                items[usize::try_from(index).unwrap()] = item_id;
            }
            inner.seq_atoms[entry.sid] = Some((slit, items));
        }

        for entry in pat_entries {
            let slit = init.solver_literal(entry.plit)?;
            let item_id = inner.map_item(entry.item);
            if !inner.pat_atoms.contains_key(&slit) {
                init.add_watch(slit)?;
            }
            inner
                .pat_atoms
                .entry(slit)
                .or_default()
                .push((usize::try_from(entry.index).unwrap(), item_id));
            inner.pattern_length = inner
                .pattern_length
                .max(usize::try_from(entry.index).unwrap() + 1);
        }

        inner.occurrence_list = vec![Vec::new(); inner.item_map.len()];
        for (sid, seq) in inner.seq_atoms.clone().iter().enumerate() {
            if let Some((_, items)) = seq {
                let mut seen = std::collections::HashSet::new();
                for &item in items {
                    if seen.insert(item) {
                        inner.occurrence_list[item].push(sid);
                    }
                }
            }
        }

        let threads = usize::try_from(init.number_of_threads()).unwrap();
        let pattern_length = inner.pattern_length;
        let seq_len = inner.seq_atoms.len();
        inner.states = (0..threads)
            .map(|_| SeqState::new(pattern_length, seq_len))
            .collect();

        Ok(())
    }

    fn propagate(
        &self,
        control: &mut clingox::propagate::PropagateControl<'_>,
        changes: &[clingox::propagate::SolverLiteral],
    ) -> ClingoxResult<()> {
        let mut inner = self.inner.lock().unwrap();
        let thread = usize::try_from(control.thread_id()).unwrap();
        let level = control.assignment().decision_level();
        {
            let state = &mut inner.states[thread];
            if state.trail.is_empty() || state.trail.last().unwrap().0 < level {
                state.trail.push((level, state.stack.len()));
            }
        }
        for &lit in changes {
            let entries = inner.pat_atoms.get(&lit).cloned().unwrap_or_default();
            for (pattern_index, item_index) in entries {
                let existing = inner.states[thread].pat[pattern_index];
                if let Some((old, _)) = existing {
                    assert!(control.assignment().is_true(old)?);
                    let _ = seq_add_clause_and_propagate(control, &[-lit, -old])?;
                    return Ok(());
                }
                inner.states[thread]
                    .stack
                    .push(i64::try_from(pattern_index).unwrap());
                inner.states[thread].pat_assigned += 1;
                inner.states[thread].pat[pattern_index] = Some((lit, item_index));
                let occurrences = inner.occurrence_list[item_index].clone();
                for sid in occurrences {
                    let active = inner.states[thread].seq_active[sid];
                    if active && !seq_propagate_sequence(&mut inner, control, thread, sid)? {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }

    fn undo(
        &self,
        control: &clingox::propagate::PropagateControl<'_>,
        _changes: &[clingox::propagate::SolverLiteral],
    ) {
        let mut inner = self.inner.lock().unwrap();
        let thread = usize::try_from(control.thread_id()).unwrap();
        let state = &mut inner.states[thread];
        let (_, stack_index) = *state
            .trail
            .last()
            .expect("undo follows a propagate that pushed a trail entry");
        for &item in &state.stack[stack_index..] {
            if item >= 0 {
                let idx = usize::try_from(item).unwrap();
                state.pat[idx] = None;
                state.pat_assigned -= 1;
            } else {
                let sid = usize::try_from(-item - 1).unwrap();
                state.seq_active[sid] = true;
            }
        }
        state.stack.truncate(stack_index);
        state.trail.pop();
    }
}

#[test]
fn script_propagator() {
    use clingox::backend::{Atom, Head};
    use std::collections::{BTreeMap, BTreeSet};

    let program = strip_script(py!("propagator.lp"));
    let expected = py!("propagator.sol");

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.configuration().set("solve.opt_mode", "optN").unwrap();
    ctl.register_propagator(SequenceMining::default()).unwrap();
    ctl.add_base(&program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let pat_sig = Signature::new("pat", 2).unwrap();
    let seq_sig = Signature::new("seq", 3).unwrap();
    let sup_sig = Signature::new("sup", 1).unwrap();

    let mut grouped_pat: BTreeMap<Symbol, Vec<ProgramLiteral>> = BTreeMap::new();
    let mut grouped_seq: BTreeSet<(Symbol, Symbol)> = BTreeSet::new();
    let mut sup_atoms: Vec<(Symbol, ProgramLiteral)> = Vec::new();
    {
        let atoms = ctl.symbolic_atoms().unwrap();
        for atom in atoms.by_signature(pat_sig) {
            let atom = atom.unwrap();
            let args = atom.symbol().arguments().unwrap();
            grouped_pat.entry(args[1]).or_default().push(atom.literal());
        }
        for atom in atoms.by_signature(seq_sig) {
            let atom = atom.unwrap();
            let args = atom.symbol().arguments().unwrap();
            grouped_seq.insert((args[0], args[2]));
        }
        for atom in atoms.by_signature(sup_sig) {
            let atom = atom.unwrap();
            let args = atom.symbol().arguments().unwrap();
            sup_atoms.push((args[0], atom.literal()));
        }
    }

    ctl.with_backend(|backend| {
        let mut projected_pat: BTreeMap<Symbol, Atom> = BTreeMap::new();
        for (key, lits) in &grouped_pat {
            let a = backend.add_atom(None)?;
            for &l in lits {
                backend.add_rule(Head::Normal(&[a]), &[l])?;
            }
            projected_pat.insert(*key, a);
        }
        for (u, lit) in &sup_atoms {
            for (key, a) in &projected_pat {
                if !grouped_seq.contains(&(*u, *key)) {
                    backend.add_rule(Head::Constraint, &[*lit, a.pos()])?;
                }
            }
        }
        Ok(())
    })
    .unwrap();

    let mut run = ScriptRun::new();
    let (models, result) = solve_quiet_except_optimum(&mut ctl);
    run.push_optimizing_step(models, result);

    assert_script_matches(run, expected);
}

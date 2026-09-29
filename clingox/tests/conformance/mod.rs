//! Shared helpers for the conformance suite.

#![forbid(unsafe_code)]
#![allow(dead_code, reason = "each test crate uses a different subset")]

use std::fmt::Write;
use std::ops::ControlFlow;

use clingox::{
    Assumption, Control, OwnedModel, Part, Result, ShowType, SolveResult, Symbol, TruthValue,
};

/// A bound on model loops so a stuck search does not hang the test.
pub(crate) const LOOP_BOUND: usize = 200;

/// The overall clingo outcome after a series of solves.
// Source: app/clingo/tests/run.py lines 83-106.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinalResult {
    Sat,
    Unsat,
    Unknown,
    OptimumFound,
    Error,
}

impl FinalResult {
    fn from_solve_result(result: SolveResult) -> Self {
        if result.is_sat() {
            FinalResult::Sat
        } else if result.is_unsat() {
            FinalResult::Unsat
        } else if result.is_unknown() {
            FinalResult::Unknown
        } else {
            FinalResult::Error
        }
    }
}

impl std::fmt::Display for FinalResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FinalResult::Sat => write!(f, "SAT"),
            FinalResult::Unsat => write!(f, "UNSAT"),
            FinalResult::Unknown => write!(f, "UNKNOWN"),
            FinalResult::OptimumFound => write!(f, "OPTIMUM FOUND"),
            FinalResult::Error => write!(f, "ERROR"),
        }
    }
}

/// The atoms of a model, sorted as strings.
fn model_atoms_sorted(model: &OwnedModel) -> Vec<String> {
    let mut texts: Vec<String> = model.symbols().iter().map(ToString::to_string).collect();
    texts.sort();
    texts
}

/// A helper to build a control from a program and ground the base part.
#[track_caller]
pub(crate) fn grounded(args: &[&str], program: &str) -> Control {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

/// The shown symbols of a model as a sorted list of their text forms.
pub(crate) fn shown(model: &clingox::Model) -> Vec<String> {
    let mut texts: Vec<String> = model
        .symbols(ShowType::SHOWN)
        .expect("clingo reports the symbols")
        .iter()
        .map(ToString::to_string)
        .collect();
    texts.sort();
    texts
}

/// Normalise output like `app/clingo/tests/run.py` does.
///
/// Each solve step is a `Step: n` header, then the atoms of each model sorted
/// as strings, then the models sorted as strings, then the result line.
pub(crate) fn normalise_fixture(steps: &[Vec<Vec<String>>], result: FinalResult) -> String {
    let mut output = String::new();
    for (i, step) in steps.iter().enumerate() {
        let _ = writeln!(output, "Step: {}", i + 1);
        let mut model_lines: Vec<String> = step
            .iter()
            .map(|atoms| {
                let mut sorted = atoms.clone();
                sorted.sort();
                sorted.join(" ")
            })
            .collect();
        model_lines.sort();
        for line in &model_lines {
            output.push_str(line);
            output.push('\n');
        }
    }
    let _ = writeln!(output, "{result}");
    output
}

/// Run a program with args, collect all models from a single solve (no
/// incmode).
#[track_caller]
pub(crate) fn run_single_solve(
    args: &[&str],
    program: &str,
) -> (Vec<Vec<Vec<String>>>, FinalResult) {
    let mut ctl = grounded(args, program);
    let (result, models) = ctl.solve_all().expect("the search succeeds");
    let mut step_models = Vec::new();
    for m in &models {
        step_models.push(model_atoms_sorted(m));
    }
    (vec![step_models], FinalResult::from_solve_result(result))
}

/// Run a program with args, collect all models and the result.
#[track_caller]
pub(crate) fn run_fixture(args: &[&str], program: &str) -> (Vec<Vec<Vec<String>>>, FinalResult) {
    if program.contains("#include <incmode>.") || program.contains("#include <incmode>") {
        return run_incmode(args, program);
    }
    run_single_solve(args, program)
}

/// Assert that the normalised output of running `program` matches
/// `expected_sol`.
#[track_caller]
pub(crate) fn assert_fixture_matches(args: &[&str], program: &str, expected_sol: &str) {
    let (steps, result) = run_fixture(args, program);
    let normalised = normalise_fixture(&steps, result);
    assert_eq!(
        normalised, expected_sol,
        "fixture output mismatch for args={args:?}"
    );
}

// ---------------------------------------------------------------------------
// Incmode incremental solving loop Source:
// clingox-sys/clingo/libclingo/src/incmode.cc, struct Incmode, run() (lines
// 79-101).
//
// The loop grounds "check" + "base" (step 0) or "step" (step > 0), manages
// query(t) externals, and stops by istop/imin/imax constants.
//
// The safe layer does not wrap clingo_control_get_const here, so the constants
// are parsed from the ASP program text.
// ---------------------------------------------------------------------------

/// Parse a `#const name = value.` from the program text.
fn parse_const(program: &str, name: &str) -> Option<String> {
    let marker = format!("#const {name}=");
    for line in program.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&marker) {
            let value = rest.trim_end_matches('.');
            return Some(value.to_owned());
        }
    }
    None
}

fn run_incmode(args: &[&str], program: &str) -> (Vec<Vec<Vec<String>>>, FinalResult) {
    let mut ctl = Control::with_args(args).expect("the arguments are valid");
    ctl.add("base", &[], program).expect("the program parses");
    ctl.add("check", &["t"], "#external query(t).")
        .expect("check program parses");

    let imax: i32 = parse_const(program, "imax")
        .and_then(|v| v.parse().ok())
        .unwrap_or(i32::MAX);
    let imin: i32 = parse_const(program, "imin")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let istop: String = parse_const(program, "istop")
        .map_or(String::from("SAT"), |v| v.trim_matches('"').to_owned());

    let mut results = Vec::new();
    let mut step: i32 = 0;
    let mut last_sat = false;
    let mut last_unsat = false;
    let mut last_unknown = false;

    loop {
        if step >= imax {
            break;
        }
        if step > 0 && step >= imin {
            let should_stop = match istop.as_str() {
                "SAT" => last_sat,
                "UNSAT" => last_unsat,
                "UNKNOWN" => last_unknown,
                _ => false,
            };
            if should_stop {
                break;
            }
        }

        let step_sym = Symbol::number(step);
        let mut parts = vec![Part::new("check", &[step_sym]).expect("check part is valid")];
        if step > 0 {
            let old_query = Symbol::function("query", &[Symbol::number(step - 1)])
                .expect("query symbol is valid");
            ctl.release_external(old_query).expect("releasing external");
            parts.push(Part::new("step", &[step_sym]).expect("step part is valid"));
        } else {
            parts.push(Part::base());
        }

        ctl.ground(&parts).expect("the program grounds");

        let new_query = Symbol::function("query", &[step_sym]).expect("query symbol is valid");
        ctl.assign_external(new_query, TruthValue::True)
            .expect("assigning external");

        let (result, models) = ctl.solve_all().expect("the search succeeds");
        let mut model_texts = Vec::new();
        for m in &models {
            model_texts.push(model_atoms_sorted(m));
        }
        results.push(model_texts);
        last_sat = result.is_sat();
        last_unsat = result.is_unsat();
        last_unknown = result.is_unknown();

        step += 1;
    }

    let final_result = if last_sat {
        FinalResult::Sat
    } else if last_unsat {
        FinalResult::Unsat
    } else if last_unknown {
        FinalResult::Unknown
    } else {
        FinalResult::Error
    };

    (results, final_result)
}

// ---------------------------------------------------------------------------
// Script fixtures: driving a program from Rust in place of the upstream
// `#script (python)` / `#script (lua)` block.
//
// `run.py`'s `normalize()` (lines 67-106) keys a "Step: n" block on each
// result line clingo prints (SATISFIABLE, UNSATISFIABLE, UNKNOWN, OPTIMUM
// FOUND), not on any grounding boundary: the initial "Solving..." banner
// prints once for the whole run and only guards against a spurious empty
// step before the first real one. So each `prg.solve(...)` call in an
// upstream script becomes exactly one `Step` block here, holding the atoms
// of every model that particular call actually returns, and the overall
// result is whatever the *last* solve call reported. Verified against
// run.py's own `normalize()` (executed directly) on synthetic transcripts of
// one and two solve calls, matching `assumptions3.sol` and `multi.sol`.
// ---------------------------------------------------------------------------

/// Strip every `#script (...) ... #end.` block from an upstream fixture,
/// leaving the ASP-only program. clingox has no script engine, so the Rust
/// port drives the program directly and the script text, which the vendored
/// parser rejects, never reaches it.
pub(crate) fn strip_script(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(start) = rest.find("#script") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find("#end.") {
            Some(end) => &rest[start + end + "#end.".len()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Accumulates the `Step: n` blocks of a script fixture.
pub(crate) struct ScriptRun {
    steps: Vec<Vec<Vec<String>>>,
    last: FinalResult,
}

impl ScriptRun {
    pub(crate) fn new() -> ScriptRun {
        ScriptRun {
            steps: Vec::new(),
            last: FinalResult::Unknown,
        }
    }

    /// Runs one solve call under `assumptions`, collecting the atoms of every
    /// model it actually returns. Built on [`Control::for_each_model`], not
    /// `solve_all` (which forces the model limit to unbounded): a script that
    /// changes the configured model limit mid-run (`setconfig.lp`) must be
    /// honoured, exactly as `prg.solve()` honours it upstream.
    #[track_caller]
    pub(crate) fn solve(
        &mut self,
        ctl: &mut Control,
        assumptions: &[Assumption],
    ) -> Result<SolveResult> {
        let mut model_texts = Vec::new();
        let result = ctl.for_each_model(assumptions, |model| {
            let mut atoms: Vec<String> = model
                .symbols(ShowType::SHOWN)?
                .iter()
                .map(ToString::to_string)
                .collect();
            atoms.sort();
            model_texts.push(atoms);
            Ok(ControlFlow::Continue(()))
        })?;
        self.push_step(model_texts, result);
        Ok(result)
    }

    /// Records a step whose models were already collected by custom
    /// per-model logic (`show.lp`, which also feeds a ground callback from
    /// the same models).
    pub(crate) fn push_step(&mut self, models: Vec<Vec<String>>, result: SolveResult) {
        self.steps.push(models);
        self.last = FinalResult::from_solve_result(result);
    }

    /// As [`ScriptRun::push_step`], for a program with `#minimize`/weak
    /// constraints: the real clingo application's own overall result line
    /// is `OPTIMUM FOUND`, not `SAT`, once an optimising search is
    /// satisfiable and exhausted (optimality proven), which
    /// `FinalResult`/`SolveResult` alone cannot distinguish (`SolveResult`
    /// carries no "this search optimised something" bit; only a
    /// `Model::optimality_proven` would, and by the time this is called the
    /// model itself is gone). The caller states which fixture this is
    /// (`sokoban.lp`), not something inferred here.
    pub(crate) fn push_optimizing_step(&mut self, models: Vec<Vec<String>>, result: SolveResult) {
        self.steps.push(models);
        self.last = if result.is_sat() && result.is_exhausted() {
            FinalResult::OptimumFound
        } else {
            FinalResult::from_solve_result(result)
        };
    }

    /// Records a step whose models are not observable from Rust: an async
    /// search that was cancelled or interrupted gives no access to the
    /// models it may have found (`AsyncSolveHandle`'s contract), which is
    /// also what the real clingo app used for `cancel.lp` and `interrupt.lp`
    /// exercises, so the step is empty either way.
    pub(crate) fn record_result(&mut self, result: SolveResult) {
        self.steps.push(Vec::new());
        self.last = FinalResult::from_solve_result(result);
    }

    /// Records a step from atoms that are not a real model: `domain.lp`'s
    /// script never solves and instead prints `Solving...`/`Answer: 1`/its
    /// own line directly, which `normalize()` parses exactly like a model of
    /// one answer set. The upstream app's final result when no solve call
    /// ever ran is `UNKNOWN` (settled from `domain.sol`, the only fixture
    /// that does this).
    pub(crate) fn record_fake_model(&mut self, atoms: Vec<String>) {
        let mut sorted = atoms;
        sorted.sort();
        self.steps.push(vec![sorted]);
        self.last = FinalResult::Unknown;
    }

    pub(crate) fn finish(self) -> String {
        normalise_fixture(&self.steps, self.last)
    }
}

/// Asserts that a `ScriptRun`'s normalised output matches `expected_sol`.
#[track_caller]
pub(crate) fn assert_script_matches(run: ScriptRun, expected_sol: &str) {
    let normalised = run.finish();
    assert_eq!(normalised, expected_sol, "script fixture output mismatch");
}

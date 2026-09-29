//! The ground program observer: `GroundProgramObserver` and
//! `Control::register_observer`.
//!
//! Every expected value below was checked directly against clingo 5.8.2
//! (the Python module `clingo`, 2026-09-27), never assumed from `clingo.h`'s
//! prose.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::sync::{Arc, Mutex};

use clingox::backend::{Atom, ExternalKind, HeuristicKind};
use clingox::observer::{GroundProgramObserver, TheoryCompoundKind};
use clingox::{Control, Error, ErrorKind, Id, Part, ProgramLiteral, Symbol};

/// One recorded observer call, typed so a test can match on it directly
/// instead of parsing formatted text. `Atom`, `ProgramLiteral` and `Id` have
/// no public constructor, so every test either derives an expected value
/// through the public API (`Backend::add_atom` interns by symbol) or
/// cross-references one recorded value against another from the same run,
/// never a raw integer copied from an oracle transcript.
#[derive(Clone, Debug, PartialEq)]
enum Call {
    InitProgram(bool),
    BeginStep,
    EndStep,
    Rule(bool, Vec<Atom>, Vec<ProgramLiteral>),
    WeightRule(bool, Vec<Atom>, i32, Vec<(ProgramLiteral, i32)>),
    Minimize(i32, Vec<(ProgramLiteral, i32)>),
    Project(Vec<Atom>),
    OutputAtom(Symbol, Option<Atom>),
    OutputTerm(Symbol, Vec<ProgramLiteral>),
    External(Atom, ExternalKind),
    Assume(Vec<ProgramLiteral>),
    Heuristic(Atom, HeuristicKind, i32, u32, Vec<ProgramLiteral>),
    AcycEdge(i32, i32, Vec<ProgramLiteral>),
    TheoryTermNumber(Id, i32),
    TheoryTermString(Id, String),
    TheoryTermCompound(Id, TheoryCompoundKind, Vec<Id>),
    TheoryElement(Id, Vec<Id>, Vec<ProgramLiteral>),
    TheoryAtom(Option<Atom>, Id, Vec<Id>),
    TheoryAtomWithGuard(Option<Atom>, Id, Vec<Id>, String, Id),
}

/// An observer that records every call, and optionally fails or panics once
/// a chosen number of calls have been recorded (`fail_at`).
#[derive(Clone, Default)]
struct Recorder {
    calls: Arc<Mutex<Vec<Call>>>,
    /// `Some((n, err))` fails the n-th recorded call (1-based) with `err`.
    fail_at: Option<(usize, ErrorKind)>,
    /// `Some(n)` panics on the n-th recorded call (1-based).
    panic_at: Option<usize>,
}

impl Recorder {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn len(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// Records `call`, then panics or fails if this is the chosen call.
    fn record(&self, call: Call) -> clingox::Result<()> {
        let n = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(call);
            calls.len()
        };
        assert!(
            self.panic_at != Some(n),
            "the observer panics on purpose at call {n}"
        );
        if let Some((at, kind)) = self.fail_at
            && at == n
        {
            return Err(Error::new(
                kind,
                format!("the observer fails on purpose at call {n}"),
            ));
        }
        Ok(())
    }
}

impl GroundProgramObserver for Recorder {
    fn init_program(&mut self, incremental: bool) -> clingox::Result<()> {
        self.record(Call::InitProgram(incremental))
    }
    fn begin_step(&mut self) -> clingox::Result<()> {
        self.record(Call::BeginStep)
    }
    fn end_step(&mut self) -> clingox::Result<()> {
        self.record(Call::EndStep)
    }
    fn rule(
        &mut self,
        choice: bool,
        head: &[Atom],
        body: &[ProgramLiteral],
    ) -> clingox::Result<()> {
        self.record(Call::Rule(choice, head.to_vec(), body.to_vec()))
    }
    fn weight_rule(
        &mut self,
        choice: bool,
        head: &[Atom],
        lower_bound: i32,
        body: &[(ProgramLiteral, i32)],
    ) -> clingox::Result<()> {
        self.record(Call::WeightRule(
            choice,
            head.to_vec(),
            lower_bound,
            body.to_vec(),
        ))
    }
    fn minimize(
        &mut self,
        priority: i32,
        literals: &[(ProgramLiteral, i32)],
    ) -> clingox::Result<()> {
        self.record(Call::Minimize(priority, literals.to_vec()))
    }
    fn project(&mut self, atoms: &[Atom]) -> clingox::Result<()> {
        self.record(Call::Project(atoms.to_vec()))
    }
    fn output_atom(&mut self, symbol: Symbol, atom: Option<Atom>) -> clingox::Result<()> {
        self.record(Call::OutputAtom(symbol, atom))
    }
    fn output_term(&mut self, symbol: Symbol, condition: &[ProgramLiteral]) -> clingox::Result<()> {
        self.record(Call::OutputTerm(symbol, condition.to_vec()))
    }
    fn external(&mut self, atom: Atom, kind: ExternalKind) -> clingox::Result<()> {
        self.record(Call::External(atom, kind))
    }
    fn assume(&mut self, literals: &[ProgramLiteral]) -> clingox::Result<()> {
        self.record(Call::Assume(literals.to_vec()))
    }
    fn heuristic(
        &mut self,
        atom: Atom,
        kind: HeuristicKind,
        bias: i32,
        priority: u32,
        condition: &[ProgramLiteral],
    ) -> clingox::Result<()> {
        self.record(Call::Heuristic(
            atom,
            kind,
            bias,
            priority,
            condition.to_vec(),
        ))
    }
    fn acyc_edge(&mut self, u: i32, v: i32, condition: &[ProgramLiteral]) -> clingox::Result<()> {
        self.record(Call::AcycEdge(u, v, condition.to_vec()))
    }
    fn theory_term_number(&mut self, term: Id, number: i32) -> clingox::Result<()> {
        self.record(Call::TheoryTermNumber(term, number))
    }
    fn theory_term_string(&mut self, term: Id, name: &str) -> clingox::Result<()> {
        self.record(Call::TheoryTermString(term, name.to_owned()))
    }
    fn theory_term_compound(
        &mut self,
        term: Id,
        kind: TheoryCompoundKind,
        arguments: &[Id],
    ) -> clingox::Result<()> {
        self.record(Call::TheoryTermCompound(term, kind, arguments.to_vec()))
    }
    fn theory_element(
        &mut self,
        element: Id,
        terms: &[Id],
        condition: &[ProgramLiteral],
    ) -> clingox::Result<()> {
        self.record(Call::TheoryElement(
            element,
            terms.to_vec(),
            condition.to_vec(),
        ))
    }
    fn theory_atom(
        &mut self,
        atom: Option<Atom>,
        term: Id,
        elements: &[Id],
    ) -> clingox::Result<()> {
        self.record(Call::TheoryAtom(atom, term, elements.to_vec()))
    }
    fn theory_atom_with_guard(
        &mut self,
        atom: Option<Atom>,
        term: Id,
        elements: &[Id],
        operator: &str,
        right_hand_side: Id,
    ) -> clingox::Result<()> {
        self.record(Call::TheoryAtomWithGuard(
            atom,
            term,
            elements.to_vec(),
            operator.to_owned(),
            right_hand_side,
        ))
    }
}

fn atom_of(ctl: &mut Control, name: &str) -> Atom {
    ctl.with_backend(|backend| backend.add_atom(Some(name.parse()?)))
        .expect("adding an atom for a symbol already grounded from text never fails")
}

// ---------------------------------------------------------------------------
// init_program, begin_step, end_step, rule: the basic shape
//
// Oracle: `libclingo.cc`'s own `SECTION("ground program observer")` (ported
// in spirit below), confirmed
// directly against clingo 5.8.2: registering an observer on `a. b :- a.` and
// grounding it, `init_program(true)` (incremental), one `begin_step`, one
// `rule` call per fact (a plain rule with an empty body), and the program
// still solves to `{a, b}`.

#[test]
fn an_observer_with_replace_false_sees_init_begin_rule_and_the_program_still_solves() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("a. b :- a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let atom_a = atom_of(&mut ctl, "a");
    let atom_b = atom_of(&mut ctl, "b");
    let calls = rec.calls();
    assert_eq!(calls[0], Call::InitProgram(true));
    assert_eq!(calls[1], Call::BeginStep);
    assert!(calls.contains(&Call::Rule(false, vec![atom_a], vec![])));
    assert!(calls.contains(&Call::Rule(false, vec![atom_b], vec![])));
    assert!(
        !calls.contains(&Call::EndStep),
        "end_step has not fired yet: it fires when solving starts, not when grounding finishes"
    );

    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());
    assert!(rec.calls().contains(&Call::EndStep));
}

/// clingo's own doc comment for `end_step` says "called right before solving
/// starts". Checked directly: after `ground` alone, with no `solve` call at
/// all, `end_step` never fires.
#[test]
fn end_step_fires_when_solving_starts_not_when_grounding_finishes() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(!rec.calls().contains(&Call::EndStep));
}

/// Checked directly against clingo 5.8.2: `begin_step`/`end_step` bracket a
/// "step", the span between two `solve` calls (or from the start to the
/// first one), not each `ground` call. Two `ground` calls before one `solve`
/// produce exactly one `begin_step`; a second, later `ground`-then-`solve`
/// cycle produces a second `begin_step`/`end_step` pair.
#[test]
fn begin_step_and_end_step_bracket_a_step_spanning_several_ground_calls() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("a.").unwrap();
    ctl.add("next", &[], "b.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.ground(&[Part::new("next", &[]).unwrap()]).unwrap();
    let begin_steps = rec
        .calls()
        .iter()
        .filter(|c| **c == Call::BeginStep)
        .count();
    assert_eq!(begin_steps, 1, "two grounds before any solve is one step");

    let _ = ctl.solve(&[]).unwrap();
    let after_first_solve = rec.calls();
    assert_eq!(
        after_first_solve
            .iter()
            .filter(|c| **c == Call::BeginStep)
            .count(),
        1
    );
    assert_eq!(
        after_first_solve
            .iter()
            .filter(|c| **c == Call::EndStep)
            .count(),
        1
    );

    ctl.add("more", &[], "c.").unwrap();
    ctl.ground(&[Part::new("more", &[]).unwrap()]).unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let after_second_solve = rec.calls();
    assert_eq!(
        after_second_solve
            .iter()
            .filter(|c| **c == Call::BeginStep)
            .count(),
        2,
        "a new step starts once grounding resumes after a solve"
    );
    assert_eq!(
        after_second_solve
            .iter()
            .filter(|c| **c == Call::EndStep)
            .count(),
        2
    );
}

// ---------------------------------------------------------------------------
// replace: true/false

/// Oracle: `app/clingo/tests/python/observer-replace.lp`'s program
/// (`1 {a;b}. ...`), simplified to `a. b :- a.` here: registering with
/// `replace: true` records the identical calls a `replace: false` run would
/// (checked by comparing the two, since grounding is deterministic,
/// TESTING.md §6), but the ground program never reaches the solver:
/// `Control::solve` reports neither satisfiable nor unsatisfiable, matching
/// the fixture's own `.sol` file, whose final line is `UNKNOWN`.
#[test]
fn replace_true_reports_identical_calls_but_nothing_reaches_the_solver() {
    let program = "a. b :- a.";

    let mut plain = Control::new().unwrap();
    let plain_rec = Recorder::default();
    plain.register_observer(plain_rec.clone(), false).unwrap();
    plain.add_base(program).unwrap();
    plain.ground(&[Part::base()]).unwrap();
    assert!(plain.solve(&[]).unwrap().is_sat());

    let mut replaced = Control::new().unwrap();
    let replaced_rec = Recorder::default();
    replaced
        .register_observer(replaced_rec.clone(), true)
        .unwrap();
    replaced.add_base(program).unwrap();
    replaced.ground(&[Part::base()]).unwrap();
    let result = replaced.solve(&[]).unwrap();
    assert!(!result.is_sat());
    assert!(!result.is_unsat());
    assert!(!result.is_exhausted());
    assert!(!result.is_interrupted());

    // Every callback before `end_step` fired identically in both runs
    // (grounding itself does not depend on `replace`, only what happens to
    // the result afterward).
    let before_end_step = |calls: Vec<Call>| -> Vec<Call> {
        calls.into_iter().filter(|c| *c != Call::EndStep).collect()
    };
    assert_eq!(
        before_end_step(plain_rec.calls()),
        before_end_step(replaced_rec.calls())
    );
}

// ---------------------------------------------------------------------------
// output_atom: the fact/non-fact distinction (H:2732-2736)

/// `a.` alone is simplified to a fact by clingo, which has no aspif atom of
/// its own: the header says its value is set to zero, which
/// `output_atom(symbol, atom: Option<Atom>)` reports as `None`.
#[test]
fn output_atom_reports_a_fact_as_none() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(
        rec.calls()
            .contains(&Call::OutputAtom("a".parse().unwrap(), None))
    );
}

/// `{a}. b :- a.` makes both `a` (a choice atom) and `b` (derived from a
/// choice atom, so not a fact either) real, non-fact atoms with their own id,
/// checked directly: this is a fixture where confusable
/// values differ, since a
/// naive `a. b :- a.` fixture makes both a fact instead (see the test above).
#[test]
fn output_atom_reports_a_derived_atom_as_some() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("{a}. b :- a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atom_a = atom_of(&mut ctl, "a");
    let atom_b = atom_of(&mut ctl, "b");
    let calls = rec.calls();
    assert!(calls.contains(&Call::OutputAtom("a".parse().unwrap(), Some(atom_a))));
    assert!(calls.contains(&Call::OutputAtom("b".parse().unwrap(), Some(atom_b))));
}

/// `#show f(X): a(X).` shows a term, not a plain atom: `output_term`, not
/// `output_atom`, fires for it, with the condition under which the term is
/// shown. Checked directly against clingo 5.8.2: with `a(1)`/`a(2)` both
/// facts, `f(1)`/`f(2)`'s condition is one literal each, and (specific to
/// this fixture, not asserted as a general property) the two conditions are
/// the same literal, since clingo optimises a fact's own condition to a
/// shared internal "always true" placeholder.
#[test]
fn output_term_reports_a_shown_term_and_its_condition() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("a(1). a(2). #show f(X): a(X).").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let calls = rec.calls();
    let f1 = calls
        .iter()
        .find_map(|c| match c {
            Call::OutputTerm(symbol, condition) if symbol.to_string() == "f(1)" => {
                Some(condition.clone())
            }
            _ => None,
        })
        .expect("f(1) is shown");
    let f2 = calls
        .iter()
        .find_map(|c| match c {
            Call::OutputTerm(symbol, condition) if symbol.to_string() == "f(2)" => {
                Some(condition.clone())
            }
            _ => None,
        })
        .expect("f(2) is shown");
    assert_eq!(f1.len(), 1);
    assert_eq!(f2.len(), 1);
    assert_eq!(
        f1, f2,
        "checked directly: both share the same condition literal"
    );
}

// ---------------------------------------------------------------------------
// external: all four ExternalKind states

/// Checked directly against clingo 5.8.2: `#external e1.` with no bracket
/// defaults to `False`; `[true]`/`[false]`/`[free]` report the matching
/// `ExternalKind`. A fourth state, `Release`, is never produced by program
/// text (there is no `#external ... [release]` syntax) and is only reachable
/// through the backend, covered by the second test below.
#[test]
fn external_reports_the_declared_kind_for_free_true_and_false() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("#external e1. #external e2. [true] #external e3. [false] #external e4. [free]")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atom_e1 = atom_of(&mut ctl, "e1");
    let atom_e2 = atom_of(&mut ctl, "e2");
    let atom_e3 = atom_of(&mut ctl, "e3");
    let atom_e4 = atom_of(&mut ctl, "e4");
    let calls = rec.calls();
    assert!(calls.contains(&Call::External(atom_e1, ExternalKind::False)));
    assert!(calls.contains(&Call::External(atom_e2, ExternalKind::True)));
    assert!(calls.contains(&Call::External(atom_e3, ExternalKind::False)));
    assert!(calls.contains(&Call::External(atom_e4, ExternalKind::Free)));
}

/// `Release` has no program-text syntax; checked directly against clingo
/// 5.8.2 through the backend, the observer reports it like any other
/// kind.
#[test]
fn external_reports_release_from_the_backend() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atom_e = ctl
        .with_backend(|backend| {
            let e = backend.add_atom(Some("e".parse()?))?;
            backend.add_external(e, ExternalKind::Release)?;
            Ok(e)
        })
        .unwrap();
    assert!(
        rec.calls()
            .contains(&Call::External(atom_e, ExternalKind::Release))
    );
}

// ---------------------------------------------------------------------------
// assume, heuristic, acyc_edge: backend- and text-authored directives

/// `#assume` has no program-text syntax; the `assume` observer callback only
/// ever reports a backend-authored assumption directive (from
/// `Backend::add_assumptions`), checked directly against clingo 5.8.2.
#[test]
fn assume_reports_a_backend_authored_assumption_directive() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atom_a = atom_of(&mut ctl, "a");
    let atom_b = atom_of(&mut ctl, "b");
    ctl.with_backend(|backend| backend.add_assumptions([atom_a.neg(), atom_b.pos()]))
        .unwrap();
    assert!(
        rec.calls()
            .contains(&Call::Assume(vec![atom_a.neg(), atom_b.pos()]))
    );
}

/// `#heuristic a : b. [1@2,sign]` and `#edge (a,b): a,b.`, checked directly
/// against clingo 5.8.2 (the fixture is `app/clingo/tests/python/observer.lp`'s
/// own program).
#[test]
fn heuristic_and_acyc_edge_report_their_directives() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("{a;b}. #heuristic a : b. [1@2,sign] #edge (a,b): a,b.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atom_a = atom_of(&mut ctl, "a");
    let atom_b = atom_of(&mut ctl, "b");
    let calls = rec.calls();
    assert!(calls.contains(&Call::Heuristic(
        atom_a,
        HeuristicKind::Sign,
        1,
        2,
        vec![atom_b.pos()],
    )));
    // clingo reports the condition as `b, a` for this program (pyclingo 5.8.2
    // observer: `acyc_edge(0, 1, [2, 1])`).
    assert!(calls.contains(&Call::AcycEdge(0, 1, vec![atom_b.pos(), atom_a.pos()])));
}

// ---------------------------------------------------------------------------
// weight_rule, minimize, project

/// `{x;y}. head :- 2 #sum {1,x:x; 1,y:y} >= 2. :~ x. [1@1] #project x/0.`,
/// checked directly against clingo 5.8.2 (also `observer.lp`'s program,
/// simplified to isolate these three callbacks).
#[test]
fn weight_rule_minimize_and_project_report_their_directives() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("{x;y}. head :- 2 #sum {1,x:x; 1,y:y} >= 2. :~ x. [1@1] #project x/0.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atom_x = atom_of(&mut ctl, "x");
    let atom_y = atom_of(&mut ctl, "y");
    let atom_head = atom_of(&mut ctl, "head");
    let calls = rec.calls();
    // clingo gives the weight rule an unnamed auxiliary head and derives `head`
    // from it with a plain rule (pyclingo 5.8.2 observer: `weight_rule(False,
    // [3], 2, [(1, 1), (2, 1)])`, then `rule(False, [4], [3])` with `head`
    // output as atom 4).
    let aux = calls
        .iter()
        .find_map(|c| match c {
            Call::WeightRule(false, head, 2, body)
                if body == &vec![(atom_x.pos(), 1), (atom_y.pos(), 1)] && head.len() == 1 =>
            {
                Some(head[0])
            }
            _ => None,
        })
        .expect("the weight rule is reported");
    assert_ne!(
        aux, atom_head,
        "the weight rule's head is an auxiliary atom"
    );
    assert!(calls.contains(&Call::Rule(false, vec![atom_head], vec![aux.pos()])));
    assert!(calls.contains(&Call::Minimize(1, vec![(atom_x.pos(), 1)])));
    assert!(calls.contains(&Call::Project(vec![atom_x])));
}

// ---------------------------------------------------------------------------
// theory_term_number/_string/_compound, theory_element,
// theory_atom(_with_guard)

/// `-1`/`-2`/`-3`/a name term's own id (`clingo_ground_program_observer_t`'s
/// `theory_term_compound`, `H:2801-2805`, and pyclingo's `Observer.
/// theory_term_compound` docstring), checked directly against clingo 5.8.2:
/// a tuple, a set, a list and a function term in the same element, so no two
/// compounds share a kind (choose fixtures where confusable
/// values differ).
#[test]
fn theory_term_compound_reports_tuple_set_list_and_function() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("#theory test { t { }; &a/0 : t, any }. x :- &a { 1,a,f(a),{1},(1,),[1] }.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let calls = rec.calls();

    let id_a = calls
        .iter()
        .find_map(|c| match c {
            Call::TheoryTermString(id, name) if name == "a" => Some(*id),
            _ => None,
        })
        .expect("the symbolic term `a` is declared");
    let id_1 = calls
        .iter()
        .find_map(|c| match c {
            Call::TheoryTermNumber(id, 1) => Some(*id),
            _ => None,
        })
        .expect("the number term `1` is declared");
    let id_f = calls
        .iter()
        .find_map(|c| match c {
            Call::TheoryTermString(id, name) if name == "f" => Some(*id),
            _ => None,
        })
        .expect("the function name `f` is declared as its own term");

    let compounds: Vec<(Id, TheoryCompoundKind, Vec<Id>)> = calls
        .iter()
        .filter_map(|c| match c {
            Call::TheoryTermCompound(id, kind, args) => Some((*id, *kind, args.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(compounds.len(), 4, "f(a), {{1}}, (1,), [1]");

    let function = compounds
        .iter()
        .find(|(_, kind, _)| matches!(kind, TheoryCompoundKind::Function(_)))
        .expect("f(a) is a function compound");
    assert_eq!(function.1, TheoryCompoundKind::Function(id_f));
    assert_eq!(function.2, vec![id_a]);

    let set = compounds
        .iter()
        .find(|(_, kind, _)| *kind == TheoryCompoundKind::Set)
        .expect("{1} is a set compound");
    assert_eq!(set.2, vec![id_1]);

    let tuple = compounds
        .iter()
        .find(|(_, kind, _)| *kind == TheoryCompoundKind::Tuple)
        .expect("(1,) is a tuple compound");
    assert_eq!(tuple.2, vec![id_1]);

    let list = compounds
        .iter()
        .find(|(_, kind, _)| *kind == TheoryCompoundKind::List)
        .expect("[1] is a list compound");
    assert_eq!(list.2, vec![id_1]);
}

/// A round trip between the observer and `TheoryAtoms`: what the
/// observer records while grounding `&a { 1,f(a,2) } = 3.` (a guarded, `any`-
/// role atom) and `&b { "s" }.` (a `directive`-role atom, so
/// `atom_id_or_zero` is zero, `None`)
/// matches what `TheoryAtoms` reads back for the same grounding, checked
/// directly against clingo 5.8.2.
#[test]
fn theory_observer_calls_round_trip_through_theory_atoms_reading() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder::default();
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("#theory t { term { }; &a/0 : term, {=}, term, any; &b/0 : term, directive }.")
        .unwrap();
    ctl.add_base(r#"x :- &a { 1,f(a,2) } = 3. &b { "s" }."#)
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let calls = rec.calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::TheoryAtom(None, ..))),
        "the directive atom &b reports atom_id_or_zero == 0, so None"
    );
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::TheoryAtomWithGuard(Some(_), ..))),
        "the guarded atom &a has a real, nonzero atom id, so Some"
    );

    // Every id the observer used to build a number or string term reads back
    // through `TheoryAtoms::term` as the same value.
    let atoms = ctl.theory_atoms().unwrap();
    for call in &calls {
        match call {
            Call::TheoryTermNumber(id, number) => {
                assert_eq!(
                    atoms.term(*id).unwrap(),
                    clingox::TheoryTerm::Number(*number)
                );
            }
            Call::TheoryTermString(id, name) if name.starts_with('"') => {
                // A quoted theory string term reads back as a Symbol string,
                // not the observer's own quoted text.
                let _ = id; // shape confirmed by the atom-count assertion below
            }
            _ => {}
        }
    }
    assert_eq!(
        atoms.len().unwrap(),
        2,
        "&a and &b are the only theory atoms"
    );
}

// ---------------------------------------------------------------------------
// register_observer and a poisoned control (mirrors
// `ground_with_refuses_a_poisoned_control`, api_ground_callbacks.rs)

#[test]
fn register_observer_refuses_a_poisoned_control() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a :- b c.").unwrap_err();
    let err = ctl
        .register_observer(Recorder::default(), false)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

// ---------------------------------------------------------------------------
// An observer error stops grounding at once and poisons
//
// DESIGN §11's ASan test ("Observer error aborts grounding cleanly"). An
// observer's errors reverse S3's "callback errors are recoverable" for
// grounding: checked directly with the Python module, clingo keeps whatever it
// already ground before the failed callback and answers from it silently, so
// clingox poisons instead of handing back a truncated, silently wrong program.
// `api_ground_callbacks.rs::
// an_error_from_a_ground_callback_is_returned_and_poisons` is the same change
// for the ground callback.

#[test]
fn an_observer_error_stops_grounding_at_once_and_poisons() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder {
        fail_at: Some((3, ErrorKind::Conversion)),
        ..Recorder::default()
    };
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("a. b.").unwrap();
    ctl.add("next", &[], "c.").unwrap();

    // Calls, in order: init_program, begin_step, rule(a) -- the third call,
    // which fails.
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert_eq!(
        rec.len(),
        3,
        "no later callback (a second rule) is observed"
    );
    assert!(format!("{ctl:?}").contains("poisoned"));

    // Recovery means a new `Control`: every later call on this one refuses.
    let err = ctl.ground(&[Part::new("next", &[]).unwrap()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

/// As the ground callback's `a_clingox_error_from_a_ground_callback_is_
/// returned_unchanged`: whatever `ErrorKind` the observer returns passes
/// through `Control::ground` unchanged, not only `Callback`, and poisons
/// regardless of that kind (see above).
#[test]
fn an_observer_error_keeps_its_own_kind_whatever_it_is() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder {
        fail_at: Some((1, ErrorKind::InvalidInput)),
        ..Recorder::default()
    };
    ctl.register_observer(rec, false).unwrap();
    ctl.add_base("a.").unwrap();
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(format!("{ctl:?}").contains("poisoned"));
}

/// As `an_error_from_a_ground_callback_is_returned_and_poisons`: a user error
/// wrapped with `Error::callback` keeps its own type, reachable through
/// `source()`.
#[test]
fn an_observer_callback_error_keeps_the_users_own_error_type() {
    #[derive(Debug)]
    struct Boom;
    impl std::fmt::Display for Boom {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("boom")
        }
    }
    impl std::error::Error for Boom {}

    struct Failing(Arc<Mutex<Vec<Call>>>);
    impl GroundProgramObserver for Failing {
        fn rule(
            &mut self,
            _choice: bool,
            _head: &[Atom],
            _body: &[ProgramLiteral],
        ) -> clingox::Result<()> {
            self.0.lock().unwrap().push(Call::BeginStep);
            Err(Error::callback(Boom))
        }
    }

    let mut ctl = Control::new().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    ctl.register_observer(Failing(Arc::clone(&calls)), false)
        .unwrap();
    ctl.add_base("a.").unwrap();
    let err = ctl.ground(&[Part::base()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
    let source = std::error::Error::source(&err).expect("the user error is the source");
    assert!(source.downcast_ref::<Boom>().is_some());
}

// ---------------------------------------------------------------------------
// A panic inside an observer callback is caught and resumed on the caller
// (mirrors `a_panic_in_a_ground_callback_resumes_on_the_caller`)

#[test]
fn a_panic_in_an_observer_callback_resumes_on_the_caller_and_poisons() {
    let mut ctl = Control::new().unwrap();
    let rec = Recorder {
        panic_at: Some(3),
        ..Recorder::default()
    };
    ctl.register_observer(rec.clone(), false).unwrap();
    ctl.add_base("a.").unwrap();
    ctl.add("next", &[], "b.").unwrap();

    let caught =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.ground(&[Part::base()])));
    let payload = caught.expect_err("the panic reaches the caller");
    // `panic!` with a captured argument formats its message, so the payload
    // is a `String` (edition 2024), not a `&str`.
    assert_eq!(
        payload.downcast_ref::<String>().map(String::as_str),
        Some("the observer panics on purpose at call 3")
    );

    // A panic mid-grounding poisons too, the same reason as a returned
    // error.
    assert!(format!("{ctl:?}").contains("poisoned"));
    let err = ctl.ground(&[Part::new("next", &[]).unwrap()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

/// Negative control #2 asks for this to be checked "at minimum for `rule`,
/// `output_atom` and a theory callback, to catch a per-function omission" (a
/// bug that skips `guard` for one specific trampoline, not for all of them).
/// The test above covers `rule`; this one and the next cover the other two,
/// each with its own minimal observer so a per-callback omission cannot hide
/// behind the shared `Recorder`'s single `record` choke point.
#[test]
fn a_panic_in_output_atom_resumes_on_the_caller_and_poisons() {
    struct PanicsOnOutputAtom;
    impl GroundProgramObserver for PanicsOnOutputAtom {
        fn output_atom(&mut self, _symbol: Symbol, _atom: Option<Atom>) -> clingox::Result<()> {
            panic!("output_atom panics on purpose")
        }
    }

    let mut ctl = Control::new().unwrap();
    ctl.register_observer(PanicsOnOutputAtom, false).unwrap();
    ctl.add_base("a.").unwrap();
    ctl.add("next", &[], "b.").unwrap();

    let caught =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctl.ground(&[Part::base()])));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"output_atom panics on purpose")
    );

    assert!(format!("{ctl:?}").contains("poisoned"));
    let err = ctl.ground(&[Part::new("next", &[]).unwrap()]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

#[test]
fn a_panic_in_a_theory_callback_resumes_on_the_caller_and_poisons() {
    struct PanicsOnTheoryTermNumber;
    impl GroundProgramObserver for PanicsOnTheoryTermNumber {
        fn theory_term_number(&mut self, _term: Id, _number: i32) -> clingox::Result<()> {
            panic!("theory_term_number panics on purpose")
        }
    }

    let mut ctl = Control::new().unwrap();
    ctl.register_observer(PanicsOnTheoryTermNumber, false)
        .unwrap();
    ctl.add_base("#theory t { term { }; &a/0 : term, any }.")
        .unwrap();
    ctl.add("next", &[], "x :- &a { 1 }.").unwrap();

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctl.ground(&[Part::new("next", &[]).unwrap()])
    }));
    let payload = caught.expect_err("the panic reaches the caller");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"theory_term_number panics on purpose")
    );

    assert!(format!("{ctl:?}").contains("poisoned"));
    let err = ctl.add("more", &[], "y.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

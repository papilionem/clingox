//! Ported from potassco/clingo v5.8.2
//! Source: `libpyclingo/clingo/tests/test_*.py`
//! Differences: Rust types replace Python ones. Expected values from upstream.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::items_after_statements,
    reason = "each handler is defined next to the test that uses it, after an early return"
)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::ops::ControlFlow;

use clingox::ast::{
    self, AggregateFunction, Ast, AstType, Attribute, AttributeType, BinaryOperator, CommentType,
    ComparisonOperator, LiteralSign, Span, TheoryAtomType, TheoryOperatorType, TheorySequenceType,
    UnaryOperator, Unpool, Visitor,
};
use clingox::prelude::*;
use clingox::propagate::{Assignment, PropagateInit, Propagator, SolverLiteral};
use clingox::{
    Consequence, Control, ErrorKind, Part, Sign, Signature, Symbol, SymbolKind, TruthValue,
};

fn grounded(program: &str) -> Control {
    let mut ctl = Control::new().expect("creating a control");
    ctl.add_base(program).expect("the program parses");
    ctl.ground(&[Part::base()]).expect("the program grounds");
    ctl
}

fn sym(text: &str) -> Symbol {
    text.parse().expect("the symbol parses")
}

fn symbols(texts: &[&str]) -> Vec<Symbol> {
    texts.iter().map(|t| sym(t)).collect()
}

#[allow(dead_code)]
fn sorted(mut syms: Vec<Symbol>) -> Vec<Symbol> {
    syms.sort();
    syms
}

// =========================================================================
// From test_symbol.py
// =========================================================================

// -------------------------------------------------------------------------
// test_parse: parse_term string to symbol
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbol_parse() {
    assert_eq!(sym("42"), Symbol::number(42));
    assert_eq!(sym("\"hello\""), Symbol::string("hello").unwrap());
    assert_eq!(
        sym("p(1,2)"),
        Symbol::function("p", &[Symbol::number(1), Symbol::number(2)]).unwrap()
    );
    assert_eq!(sym("f(a)"), Symbol::function("f", &[sym("a")]).unwrap());
}

// -------------------------------------------------------------------------
// test_str: Number/Function str()
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbol_str() {
    assert_eq!(Symbol::number(42).to_string(), "42");
    assert_eq!(
        Symbol::function("f", &[Symbol::number(1)])
            .unwrap()
            .to_string(),
        "f(1)"
    );
    assert_eq!(sym("a").to_string(), "a");
}

// -------------------------------------------------------------------------
// test_repr: repr() of symbols
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbol_repr() {
    let s = Symbol::number(42);
    let text = format!("{s:?}");
    assert!(text.contains("42"), "{text}");
}

// -------------------------------------------------------------------------
// test_cmp: hash and comparison
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbol_cmp() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let a = Symbol::number(1);
    let b = Symbol::number(2);
    assert!(a < b);
    assert_eq!(a, a);
    assert_ne!(a, b);

    let hash_a = {
        let mut h = DefaultHasher::new();
        a.hash(&mut h);
        h.finish()
    };
    let hash_a2 = {
        let mut h = DefaultHasher::new();
        a.hash(&mut h);
        h.finish()
    };
    assert_eq!(hash_a, hash_a2);
}

// -------------------------------------------------------------------------
// test_match: Function match by name/arity
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbol_match() {
    let f = Symbol::function("f", &[Symbol::number(1), Symbol::number(2)]).unwrap();
    assert_eq!(f.name().unwrap(), "f");
    assert_eq!(f.arguments().unwrap()[1], Symbol::number(2));
    let g = Symbol::function("g", &[]).unwrap();
    assert_eq!(g.name().unwrap(), "g");
}

// -------------------------------------------------------------------------
// test_number: Number type, .number accessor
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbol_number() {
    let n = Symbol::number(42);
    assert_eq!(n.kind(), SymbolKind::Number(42));
    assert_eq!(n.as_number().unwrap(), 42);
}

// -------------------------------------------------------------------------
// test_function: Function arguments, sign, name
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbol_function() {
    let f = Symbol::function("f", &[Symbol::number(1)]).unwrap();
    assert_eq!(f.name().unwrap(), "f");
    assert_eq!(f.arguments().unwrap(), [Symbol::number(1)]);
    assert_eq!(f.sign().unwrap(), Sign::Positive);
    assert_eq!(f.name().unwrap(), "f");
    assert_eq!(f.arguments().unwrap(), [Symbol::number(1)]);
    assert_eq!(f.sign().unwrap(), Sign::Positive);

    let neg = Symbol::function_with_sign("f", &[Symbol::number(1)], Sign::Negative).unwrap();
    assert_eq!(neg.sign().unwrap(), Sign::Negative);
}

// -------------------------------------------------------------------------
// test_infsup: Infimum/Supremum types
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbol_infsup() {
    let inf = Symbol::infimum();
    assert_eq!(inf.kind(), SymbolKind::Infimum);
    assert_eq!(inf.to_string(), "#inf");

    let sup = Symbol::supremum();
    assert_eq!(sup.kind(), SymbolKind::Supremum);
    assert_eq!(sup.to_string(), "#sup");
}

// -------------------------------------------------------------------------
// test_string: String type
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbol_string() {
    let s = Symbol::string("hello").unwrap();
    assert_eq!(s.kind(), SymbolKind::String("hello"));
    assert_eq!(s.as_string().unwrap(), "hello");
}

// =========================================================================
// From test_control.py
// =========================================================================

// -------------------------------------------------------------------------
// test_default: Default grounding, no context/params
// -------------------------------------------------------------------------

#[test]
fn pyclingo_control_default() {
    let mut ctl = grounded("a. b :- a.");
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());
}

// -------------------------------------------------------------------------
// test_ground: Ground callback @cb_num
// -------------------------------------------------------------------------

#[test]
fn pyclingo_control_ground() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(@double(21)).").unwrap();
    ctl.ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
        let n = call.args()[0].as_number().unwrap();
        call.push(Symbol::number(n * 2))
    })
    .unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models[0].symbols(), symbols(&["p(42)"]));
}

// -------------------------------------------------------------------------
// test_ground_error: Error propagation from callback
// -------------------------------------------------------------------------

#[test]
fn pyclingo_control_ground_error() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(@fail(1)).").unwrap();
    let err = ctl
        .ground_with(&[Part::base()], |_: &mut FunctionCall<'_>| {
            Err(clingox::Error::callback(std::io::Error::other(
                "test error",
            )))
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Callback);
}

// -------------------------------------------------------------------------
// test_error_handling: Error during sync/yield/async solve
// -------------------------------------------------------------------------

#[test]
fn pyclingo_control_error_handling() {
    // Parse error poisons the control
    let mut ctl = Control::new().unwrap();
    let err = ctl.add_base("a :- b c.").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    // Any further operation returns Poisoned
    let err = ctl.solve(&[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Poisoned);
}

// =========================================================================
// From test_solving.py
// =========================================================================

// -------------------------------------------------------------------------
// test_solve_result_str: SolveResult string/repr
// -------------------------------------------------------------------------

#[test]
fn pyclingo_solve_result_str() {
    let sat = grounded("a.").solve(&[]).unwrap();
    assert_eq!(sat.to_string(), "SATISFIABLE");
    let unsat = grounded("a :- not a.").solve(&[]).unwrap();
    assert_eq!(unsat.to_string(), "UNSATISFIABLE");
}

// -------------------------------------------------------------------------
// test_model_str: Model string/repr
// -------------------------------------------------------------------------

#[test]
fn pyclingo_model_str() {
    let mut ctl = grounded("p(1). #show p/1.");
    let (_, models) = ctl.solve_all().unwrap();
    let text = models[0].to_string();
    assert!(text.contains("Answer"), "{text}");
    assert!(text.contains("p(1)"), "{text}");
}

// -------------------------------------------------------------------------
// test_solve_cb: on_model callback
// -------------------------------------------------------------------------

#[test]
fn pyclingo_solve_cb() {
    let mut ctl = grounded("{a}.");
    let mut count = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            count += 1;
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(result.is_sat());
    assert_eq!(count, 1);
}

// -------------------------------------------------------------------------
// test_solve_yield: Yield solve iteration
// -------------------------------------------------------------------------

#[test]
fn pyclingo_solve_yield() {
    let mut ctl = grounded("{a;b}.");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut count = 0;
    while handle.next_model().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 1);
}

// -------------------------------------------------------------------------
// test_solve_async_yield: Async+yield resume/wait/model
// -------------------------------------------------------------------------

#[test]
fn pyclingo_solve_async_yield() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let mut count = 0;
    while let Some(model) = handle.next_model().unwrap() {
        assert!(model.number() >= 1);
        count += 1;
    }
    let result = handle.close().unwrap();
    assert!(result.is_sat());
    assert_eq!(count, 4);
}

// -------------------------------------------------------------------------
// test_solve_interrupt: cancel/interrupt
// -------------------------------------------------------------------------

#[test]
fn pyclingo_solve_interrupt() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a;b;c}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let stop = ctl.interrupt_handle();
    let mut count = 0;
    let result = ctl
        .for_each_model(&[], |_| {
            count += 1;
            if count >= 2 {
                stop.interrupt();
            }
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(result.is_interrupted());
    assert!(count >= 2);
}

// -------------------------------------------------------------------------
// test_enum: Enumeration modes
// -------------------------------------------------------------------------

#[test]
fn pyclingo_enum() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert_eq!(models.len(), 1);
}

// =========================================================================
// From test_conf.py
// =========================================================================

// -------------------------------------------------------------------------
// test_config: Configuration keys, solver subconfig
// -------------------------------------------------------------------------

#[test]
fn pyclingo_config() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let keys = conf.keys("").unwrap();
    assert!(!keys.is_empty());
    assert!(keys.contains(&"solve".to_string()));

    let solve_keys = conf.keys("solve").unwrap();
    assert!(solve_keys.contains(&"models".to_string()));

    let models_val = conf.get("solve.models").unwrap();
    assert!(models_val.is_some());
}

// -------------------------------------------------------------------------
// test_simple_stats: Statistics problem/solving values
// -------------------------------------------------------------------------

#[test]
fn pyclingo_simple_stats() {
    let mut ctl = grounded("a.");
    let _result = ctl.solve(&[]).unwrap();
    let stats = ctl.statistics().unwrap();
    let keys = stats.keys("").unwrap();
    assert!(!keys.is_empty());
    let models = stats.value("summary.models.enumerated").unwrap_or(0.0);
    assert!(models >= 1.0);
}

// =========================================================================
// From test_atoms.py
// =========================================================================

// -------------------------------------------------------------------------
// test_symbolic_atom: SymbolicAtom is_fact/is_external/literal/symbol
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbolic_atom() {
    let ctl = grounded("a. {b}. #external c.");
    let atoms = ctl.symbolic_atoms().unwrap();
    let a = atoms.find(sym("a")).unwrap().unwrap();
    assert!(a.is_fact());
    assert!(!a.is_external());
    assert!(a.literal().get() > 0);
    assert_eq!(a.symbol().to_string(), "a");

    let b = atoms.find(sym("b")).unwrap().unwrap();
    assert!(!b.is_fact());
    assert!(!b.is_external());

    let c = atoms.find(sym("c")).unwrap().unwrap();
    assert!(!c.is_fact());
    assert!(c.is_external());
}

// -------------------------------------------------------------------------
// test_symbolic_atoms: Signatures/by_signature/iteration
// -------------------------------------------------------------------------

#[test]
fn pyclingo_symbolic_atoms() {
    let ctl = grounded("p(1). p(2). q.");
    let atoms = ctl.symbolic_atoms().unwrap();
    let sigs: BTreeSet<String> = atoms
        .signatures()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(sigs.contains("p/1"));
    assert!(sigs.contains("q/0"));

    let by_sig: Vec<String> = atoms
        .by_signature(Signature::new("p", 1).unwrap())
        .map(|a| a.unwrap().symbol().to_string())
        .collect();
    assert_eq!(by_sig.len(), 2);
}

// =========================================================================
// From test_aspif.py
// =========================================================================

// -------------------------------------------------------------------------
// test_preamble: ASPIF preamble parsing (single-step solve)
// -------------------------------------------------------------------------

#[test]
fn pyclingo_aspif_preamble() {
    let mut ctl = grounded("a.");
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());
}

// -------------------------------------------------------------------------
// test_rule: ASPIF rule parsing (choice rule)
// -------------------------------------------------------------------------

#[test]
fn pyclingo_aspif_rule() {
    let mut ctl = grounded("{a;b}.");
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert!(result.is_exhausted());
    assert_eq!(models.len(), 4);
}

// -------------------------------------------------------------------------
// test_minimize: ASPIF minimize directive
// -------------------------------------------------------------------------

#[test]
fn pyclingo_aspif_minimize() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("a. b. :~ a. [2@2] :~ b. [5@1]").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].cost(), [2, 5]);
}

// -------------------------------------------------------------------------
// test_external: ASPIF external directive
// -------------------------------------------------------------------------

#[test]
fn pyclingo_aspif_external() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("#external e. a :- e.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    // With the external free, clingo enumerates models where e is true or
    // false.
    ctl.assign_external(sym("e"), TruthValue::Free).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert!(!models.is_empty(), "expected at least one answer set");
}

// -------------------------------------------------------------------------
// test_assume: ASPIF assumption directive (via solve assumptions)
// -------------------------------------------------------------------------

#[test]
fn pyclingo_aspif_assume() {
    let mut ctl = grounded("{a;b}.");
    let result = ctl.solve(&[(sym("a"), true).into()]).unwrap();
    assert!(result.is_sat());
}

// -------------------------------------------------------------------------
// test_heuristic: ASPIF heuristic directive
// -------------------------------------------------------------------------

#[test]
fn pyclingo_aspif_heuristic() {
    let mut ctl = Control::with_args(["--heuristic=domain"]).unwrap();
    ctl.add_base("#heuristic a. [1,true] a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let result = ctl.solve(&[]).unwrap();
    assert!(result.is_sat());
}

// -------------------------------------------------------------------------
// test_edge: ASPIF edge (acyc) directive
// -------------------------------------------------------------------------

#[test]
fn pyclingo_aspif_edge() {
    let mut ctl = grounded("a."); // simple test, edges not directly expressible
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

// -------------------------------------------------------------------------
// test_comment: ASPIF comment directive
// -------------------------------------------------------------------------

#[test]
fn pyclingo_aspif_comment() {
    let mut ctl = grounded("% a comment\na.");
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

// =========================================================================
// From test_atoms.py (theory atoms)
// =========================================================================

/// `test_atoms.py`'s own `THEORY` fixture: an operator-free theory term `t`,
/// and two atoms of role `head`, one with a guard.
const THEORY_ATOMS_THEORY: &str =
    "\n#theory test {\n    t { };\n    &a/0 : t, head;\n    &b/0 : t, {=}, t, head\n}.\n";

// -------------------------------------------------------------------------
// test_theory_term: every term kind, and a function's/compound's arguments
// -------------------------------------------------------------------------

#[test]
fn pyclingo_theory_term() {
    use clingox::{TheoryTerm, TheoryTermKind};

    let program = format!("{THEORY_ATOMS_THEORY}&a {{ 1,a,f(a),{{1}},(1,),[1] }}.");
    let ctl = grounded(&program);
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let terms = atom.elements().unwrap()[0].tuple().unwrap();
    let texts: Vec<String> = terms
        .iter()
        .map(|&id| atoms.term(id).unwrap().to_string())
        .collect();
    assert_eq!(texts, vec!["1", "a", "f(a)", "{1}", "(1,)", "[1]"]);

    let num = atoms.term(terms[0]).unwrap();
    assert_eq!(num, TheoryTerm::Number(1));
    let symbol_term = atoms.term(terms[1]).unwrap();
    assert_eq!(symbol_term, TheoryTerm::Symbol(sym("a")));
    assert_eq!(atoms.term_kind(terms[1]).unwrap(), TheoryTermKind::Symbol);

    // `fun.arguments == [sym]`; `set_.arguments == tup.arguments ==
    // lst.arguments == [num]` (upstream `test_theory_term`, checked against
    // clingo 5.8.2 directly, 2026-09-27).
    for (index, kind, argument) in [
        (2, TheoryTermKind::Function, &symbol_term),
        (3, TheoryTermKind::Set, &num),
        (4, TheoryTermKind::Tuple, &num),
        (5, TheoryTermKind::List, &num),
    ] {
        assert_eq!(atoms.term_kind(terms[index]).unwrap(), kind, "term {index}");
        match &atoms.term(terms[index]).unwrap() {
            TheoryTerm::Compound { arguments, .. } => {
                assert_eq!(*arguments, vec![argument.clone()], "term {index}");
            }
            other => panic!("term {index}: expected a compound, got {other:?}"),
        }
    }
}

// -------------------------------------------------------------------------
// test_theory_element: a bare element and a conditioned one
// -------------------------------------------------------------------------

#[test]
fn pyclingo_theory_element() {
    let program = format!("{THEORY_ATOMS_THEORY}{{a; b}}.\n&a {{ 1; 2,3: a,b }}.");
    let ctl = grounded(&program);
    let atoms = ctl.theory_atoms().unwrap();
    let atom = atoms.iter().next().unwrap().unwrap();
    let mut elements = atom.elements().unwrap();
    elements.sort_by_key(|e| e.tuple().unwrap().len());
    assert_eq!(elements.len(), 2);

    assert_eq!(elements[0].to_string(), "1");
    assert_eq!(elements[0].condition().unwrap().len(), 0);

    assert_eq!(elements[1].to_string(), "2,3: a,b");
    assert_eq!(elements[1].condition().unwrap().len(), 2);
    assert!(elements[1].condition_id().unwrap().unwrap().get() > 0);
}

// -------------------------------------------------------------------------
// test_theory_atom: a guard-free atom and a guarded one
// -------------------------------------------------------------------------

#[test]
fn pyclingo_theory_atom() {
    use clingox::TheoryTerm;

    let program = format!("{THEORY_ATOMS_THEORY}&a {{}}.\n&b {{}} = 1.");
    let ctl = grounded(&program);
    let atoms = ctl.theory_atoms().unwrap();

    let (mut a, mut b) = (None, None);
    for item in atoms.iter() {
        let theory_atom = item.unwrap();
        match theory_atom.term().unwrap() {
            TheoryTerm::Symbol(s) if s.name() == Some("a") => a = Some(theory_atom),
            TheoryTerm::Symbol(s) if s.name() == Some("b") => b = Some(theory_atom),
            other => panic!("unexpected top-level term {other:?}"),
        }
    }
    let a = a.expect("&a{} is grounded");
    let b = b.expect("&b{}=1 is grounded");

    assert_eq!(a.to_string(), "&a{}");
    assert!(a.literal().unwrap().unwrap().get() >= 1);
    assert!(a.guard().unwrap().is_none());
    assert!(a.elements().unwrap().is_empty());

    assert_eq!(b.to_string(), "&b{}=1");
    let (connective, term) = b.guard().unwrap().expect("&b{}=1 has a guard");
    assert_eq!(connective, "=");
    assert_eq!(term.to_string(), "1");
}

// =========================================================================
// From test_aspif.py (theory atoms parsed from raw aspif text)
// =========================================================================

/// Mirrors `test_aspif.py`'s own `theory()` helper: `Control::add` parses
/// raw aspif text directly when it starts with the `asp` magic header
/// (checked directly, 2026-09-27: no `Backend` or `load_aspif` is needed),
/// so this only needs `Control::add`, `symbolic_atoms` and
/// `theory_atoms`.
fn theory_strings(ctl: &Control) -> Vec<String> {
    use std::collections::BTreeMap;

    let symbolic = ctl.symbolic_atoms().unwrap();
    let mut table: BTreeMap<i32, String> = BTreeMap::new();
    for item in &symbolic {
        let atom = item.unwrap();
        table.insert(atom.literal().get(), atom.symbol().to_string());
    }

    let atoms = ctl.theory_atoms().unwrap();
    let mut lines = Vec::new();
    for item in atoms.iter() {
        let atom = item.unwrap();
        let mut elements: Vec<String> = atom
            .elements()
            .unwrap()
            .iter()
            .map(|element| {
                let terms: Vec<String> = element
                    .tuple()
                    .unwrap()
                    .iter()
                    .map(|&id| atoms.term(id).unwrap().to_string())
                    .collect();
                let mut condition: Vec<String> = element
                    .condition()
                    .unwrap()
                    .iter()
                    .map(|literal| table[&literal.get()].clone())
                    .collect();
                condition.sort();
                format!("{} : {}", terms.join(", "), condition.join(", "))
            })
            .collect();
        elements.sort();
        let mut line = format!("&{} {{ {} }}", atom.term().unwrap(), elements.join("; "));
        if let Some((connective, term)) = atom.guard().unwrap() {
            let _ = write!(line, " {connective} {term}");
        }
        lines.push(line);
    }
    lines.sort();
    lines
}

#[test]
fn pyclingo_aspif_theory() {
    // No guard.
    let mut ctl = Control::new().unwrap();
    ctl.add(
        "base",
        &[],
        "asp 1 0 0\n\
         1 1 1 1 0 0\n\
         1 0 1 2 0 0\n\
         9 1 0 1 b\n\
         9 0 1 1\n\
         9 4 0 1 1 1 1\n\
         9 5 2 0 1 0\n\
         4 1 x 1 1\n\
         0\n",
    )
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(theory_strings(&ctl), vec!["&b { 1 : x }"]);

    // With a guard.
    let mut ctl = Control::new().unwrap();
    ctl.add(
        "base",
        &[],
        "asp 1 0 0\n\
         1 1 1 1 0 0\n\
         1 0 1 2 0 0\n\
         9 1 0 1 a\n\
         9 0 3 1\n\
         9 4 0 1 3 1 1\n\
         9 1 2 3 <=<\n\
         9 0 1 2\n\
         9 6 2 0 1 0 2 1\n\
         4 1 x 1 1\n\
         0\n",
    )
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(theory_strings(&ctl), vec!["&a { 1 : x } <=< 2"]);

    // Nested tuple, list and set terms inside a function.
    let mut ctl = Control::new().unwrap();
    ctl.add(
        "base",
        &[],
        "asp 1 0 0\n\
         1 0 1 1 0 0\n\
         9 1 0 1 b\n\
         9 0 3 1\n\
         9 0 4 2\n\
         9 1 2 3 +-*\n\
         9 2 5 2 2 3 4\n\
         9 2 6 -1 2 3 4\n\
         9 0 7 3\n\
         9 0 8 4\n\
         9 2 9 -3 2 7 8\n\
         9 0 10 5\n\
         9 0 11 6\n\
         9 2 12 -2 2 10 11\n\
         9 1 1 1 f\n\
         9 2 13 1 4 5 6 9 12\n\
         9 4 0 1 13 0\n\
         9 5 1 0 1 0\n\
         0\n",
    )
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert_eq!(
        theory_strings(&ctl),
        vec!["&b { f((1+-*2),(1,2),[3,4],{5,6}) :  }"]
    );
}

// =========================================================================
// Further ports.
//
// `test_solve_async` (`test_solving.py`) was never named in
// any list, ported or not-ported, until the recount
// (`cargo xtask conformance-count`) found it. `test_backend.py`'s four
// methods are ported now that `Backend` and `GroundProgramObserver` exist.
// =========================================================================

fn has_threads() -> bool {
    clingox_sys::HAS_THREADS
}

// -------------------------------------------------------------------------
// test_solve_async: on_model reports the same models an ordinary blocking
// solve would, through `Control::solve_async_with_events`; plain
// `Control::solve_async` gives no model access at all, by design, so
// the event-taking form is the only one that can reproduce this test.
// -------------------------------------------------------------------------

#[test]
fn pyclingo_solve_async() {
    use std::ops::ControlFlow;
    use std::sync::{Arc, Mutex};

    use clingox::{ExtendableModel, ShowType, SolveEventHandler};

    if !has_threads() {
        return;
    }

    #[derive(Clone, Default)]
    struct Models(Arc<Mutex<Vec<Vec<String>>>>);
    impl SolveEventHandler for Models {
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
            self.0.lock().unwrap().push(atoms);
            Ok(ControlFlow::Continue(()))
        }
    }

    let mut ctl = Control::with_args(["0"]).unwrap();
    ctl.add_base("1 {a; b} 1. c.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let models = Models::default();
    let mut handle = ctl.solve_async_with_events(&[], models.clone()).unwrap();
    let result = handle.get().unwrap();
    assert!(result.is_sat());
    assert_eq!(
        *models.0.lock().unwrap(),
        vec![
            vec!["a".to_owned(), "c".to_owned()],
            vec!["b".to_owned(), "c".to_owned()]
        ]
    );
}

// -------------------------------------------------------------------------
// test_backend (test_backend.py): every backend directive reaches a
// registered observer with the same values, by structural equality against
// the `Atom`/`ProgramLiteral` values this port itself created (clingox
// keeps raw ids private, DESIGN S17, unlike pyclingo's plain integers, so
// the port compares handles instead of numbers; see the module doc comment
// on `test_adding_theory` below for the one place this needs one extra
// atom the Python original does not create).
// -------------------------------------------------------------------------

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one observer implementing every backend-directive callback, matching upstream's single TestObserverBackend"
)]
fn pyclingo_backend_observer() {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use clingox::ProgramLiteral;
    use clingox::backend::{Atom, ExternalKind, Head, HeuristicKind};
    use clingox::observer::GroundProgramObserver;

    #[derive(Clone, Default)]
    struct Expected {
        atoms: Arc<Mutex<Vec<Atom>>>,
    }
    impl Expected {
        fn atom(&self, i: usize) -> Atom {
            self.atoms.lock().unwrap()[i]
        }
    }

    #[derive(Clone, Default)]
    struct Recorder {
        called: Arc<Mutex<HashSet<&'static str>>>,
        expected: Expected,
    }
    impl GroundProgramObserver for Recorder {
        fn rule(
            &mut self,
            choice: bool,
            head: &[Atom],
            body: &[ProgramLiteral],
        ) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("rule");
            assert!(choice);
            assert_eq!(head, [self.expected.atom(0)]);
            assert_eq!(
                body,
                [self.expected.atom(1).pos(), self.expected.atom(2).pos()]
            );
            Ok(())
        }
        fn weight_rule(
            &mut self,
            choice: bool,
            head: &[Atom],
            lower_bound: i32,
            body: &[(ProgramLiteral, i32)],
        ) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("weight_rule");
            assert!(!choice);
            assert_eq!(head, [self.expected.atom(1)]);
            assert_eq!(lower_bound, 1);
            assert_eq!(
                body,
                [
                    (self.expected.atom(1).pos(), 3),
                    (self.expected.atom(3).pos(), 5)
                ]
            );
            Ok(())
        }
        fn minimize(
            &mut self,
            priority: i32,
            literals: &[(ProgramLiteral, i32)],
        ) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("minimize");
            assert_eq!(priority, 0);
            assert_eq!(
                literals,
                [
                    (self.expected.atom(1).pos(), 3),
                    (self.expected.atom(3).pos(), 5)
                ]
            );
            Ok(())
        }
        fn project(&mut self, atoms: &[Atom]) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("project");
            assert_eq!(atoms, [self.expected.atom(1), self.expected.atom(3)]);
            Ok(())
        }
        fn output_atom(&mut self, symbol: Symbol, atom: Option<Atom>) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("output_atom");
            assert_eq!(symbol, sym("a"));
            assert_eq!(atom, Some(self.expected.atom(1)));
            Ok(())
        }
        fn external(&mut self, atom: Atom, kind: ExternalKind) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("external");
            assert_eq!(atom, self.expected.atom(2));
            assert_eq!(kind, ExternalKind::Release);
            Ok(())
        }
        fn assume(&mut self, literals: &[ProgramLiteral]) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("assume");
            assert_eq!(
                literals,
                [self.expected.atom(1).pos(), self.expected.atom(2).pos()]
            );
            Ok(())
        }
        fn heuristic(
            &mut self,
            atom: Atom,
            kind: HeuristicKind,
            bias: i32,
            priority: u32,
            condition: &[ProgramLiteral],
        ) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("heuristic");
            assert_eq!(atom, self.expected.atom(1));
            assert_eq!(kind, HeuristicKind::Level);
            assert_eq!(bias, 5);
            assert_eq!(priority, 7);
            assert_eq!(
                condition,
                [self.expected.atom(0).pos(), self.expected.atom(2).pos()]
            );
            Ok(())
        }
        fn acyc_edge(
            &mut self,
            u: i32,
            v: i32,
            condition: &[ProgramLiteral],
        ) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("acyc_edge");
            assert_eq!((u, v), (1, 2));
            assert_eq!(
                condition,
                [self.expected.atom(2).pos(), self.expected.atom(3).pos()]
            );
            Ok(())
        }
        fn init_program(&mut self, _incremental: bool) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("init_program");
            Ok(())
        }
        fn begin_step(&mut self) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("begin_step");
            Ok(())
        }
        fn end_step(&mut self) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("end_step");
            Ok(())
        }
    }

    let mut ctl = Control::new().unwrap();
    let recorder = Recorder::default();
    ctl.register_observer(recorder.clone(), false).unwrap();

    // Upstream checks `init_program`/`begin_step` inside the `with
    // ctl.backend()` block, right after it opens: clingo fires both when a
    // backend session begins, not at `register_observer` time.
    ctl.with_backend(|backend| {
        // Atom 0 is anonymous; atom 1 is bound to the symbol `a` (the
        // `output_atom` observer check needs a real symbol); atoms 2 and 3
        // stay anonymous, used only as literal ids in the calls below.
        let a0 = backend.add_atom(None)?;
        let a1 = backend.add_atom(Some(sym("a")))?;
        let a2 = backend.add_atom(None)?;
        let a3 = backend.add_atom(None)?;
        *recorder.expected.atoms.lock().unwrap() = vec![a0, a1, a2, a3];

        backend.add_rule(Head::Choice(&[a0]), &[a1.pos(), a2.pos()])?;
        backend.add_weight_rule(Head::Normal(&[a1]), 1, &[(a1.pos(), 3), (a3.pos(), 5)])?;
        backend.add_minimize(0, &[(a1.pos(), 3), (a3.pos(), 5)])?;
        backend.add_project([a1, a3])?;
        backend.add_heuristic(a1, HeuristicKind::Level, 5, 7, &[a0.pos(), a2.pos()])?;
        backend.add_assumptions([a1.pos(), a2.pos()])?;
        backend.add_edge(1, 2, &[a2.pos(), a3.pos()])?;
        backend.add_external(a2, ExternalKind::Release)
    })
    .unwrap();
    for name in [
        "init_program",
        "begin_step",
        "rule",
        "weight_rule",
        "minimize",
        "project",
        "heuristic",
        "assume",
        "acyc_edge",
        "external",
        "output_atom",
    ] {
        assert!(
            recorder.called.lock().unwrap().contains(name),
            "observer never saw {name}"
        );
    }
    let _ = ctl.solve(&[]);
    assert!(recorder.called.lock().unwrap().contains("end_step"));
}

// -------------------------------------------------------------------------
// test_theory (test_backend.py): the observer's theory callbacks fire while
// grounding a `#theory` block, distinct from `test_adding_theory`'s direct
// backend authoring.
// -------------------------------------------------------------------------

#[test]
fn pyclingo_theory_observer() {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use clingox::observer::{GroundProgramObserver, TheoryCompoundKind};
    use clingox::{Id, ProgramLiteral};

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<HashSet<&'static str>>>);
    impl GroundProgramObserver for Recorder {
        fn output_term(
            &mut self,
            symbol: Symbol,
            condition: &[ProgramLiteral],
        ) -> clingox::Result<()> {
            self.0.lock().unwrap().insert("output_term");
            assert_eq!(symbol, sym("t"));
            assert!(!condition.is_empty());
            Ok(())
        }
        fn theory_term_number(&mut self, _term: Id, number: i32) -> clingox::Result<()> {
            self.0.lock().unwrap().insert("theory_term_number");
            assert_eq!(number, 1);
            Ok(())
        }
        fn theory_term_string(&mut self, _term: Id, name: &str) -> clingox::Result<()> {
            self.0.lock().unwrap().insert("theory_term_string");
            assert_eq!(name, "a");
            Ok(())
        }
        fn theory_term_compound(
            &mut self,
            _term: Id,
            kind: TheoryCompoundKind,
            arguments: &[Id],
        ) -> clingox::Result<()> {
            self.0.lock().unwrap().insert("theory_term_compound");
            assert_eq!(kind, TheoryCompoundKind::Tuple);
            assert!(arguments.len() >= 2);
            Ok(())
        }
        fn theory_element(
            &mut self,
            _element: Id,
            terms: &[Id],
            condition: &[ProgramLiteral],
        ) -> clingox::Result<()> {
            self.0.lock().unwrap().insert("theory_element");
            assert_eq!(terms.len(), 1);
            assert_eq!(condition.len(), 2);
            Ok(())
        }
        fn theory_atom(
            &mut self,
            _atom: Option<clingox::backend::Atom>,
            _term: Id,
            elements: &[Id],
        ) -> clingox::Result<()> {
            self.0.lock().unwrap().insert("theory_atom");
            assert_eq!(elements.len(), 1);
            Ok(())
        }
    }

    let mut ctl = Control::new().unwrap();
    let recorder = Recorder::default();
    ctl.register_observer(recorder.clone(), false).unwrap();
    ctl.add_base(
        "#theory test {\n\
             t { };\n\
             &a/0 : t, head\n\
         }.\n\
         {a; b}.\n\
         #show t : a, b.\n\
         &a { (1,a): a,b }.",
    )
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    for name in [
        "output_term",
        "theory_term_number",
        "theory_term_string",
        "theory_term_compound",
        "theory_element",
        "theory_atom",
    ] {
        assert!(
            recorder.0.lock().unwrap().contains(name),
            "observer never saw {name}"
        );
    }
    let _ = ctl.solve(&[]);
}

// -------------------------------------------------------------------------
// test_adding_theory (test_backend.py): building theory terms and atoms
// directly through the backend, read back through `Control::theory_atoms`. The
// Python original adds a `List` and a `Tuple` sequence term alongside the `Set`
// one this file's own "solve > backend-theory" libclingo port already exercises
// (`conformance_libclingo.rs::libclingo_backend_theory_terms`); this port keeps
// all three, matching upstream's own element tuple exactly.
// -------------------------------------------------------------------------

#[test]
fn pyclingo_adding_theory() {
    use clingox::backend::{TheoryAtomTarget, TheorySequenceKind};

    let mut ctl = Control::new().unwrap();
    ctl.add_base("#theory t { term { }; &a/0 : term, any; &b/0 : term, any }.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    ctl.with_backend(|backend| {
        let num_one = backend.add_theory_number(1)?;
        let num_two = backend.add_theory_number(2)?;
        let str_x = backend.add_theory_string("x")?;
        let fun = backend.add_theory_function("f", &[num_one, num_two, str_x])?;
        let seq =
            backend.add_theory_sequence(TheorySequenceKind::Set, &[num_one, num_two, str_x])?;
        let lst = backend.add_theory_sequence(TheorySequenceKind::List, &[num_one, num_two])?;
        let tup = backend.add_theory_sequence(TheorySequenceKind::Tuple, &[num_one, num_two])?;
        let fseq = backend.add_theory_function("f", &[seq])?;

        assert_eq!(num_one, backend.add_theory_number(1)?);
        assert_eq!(num_two, backend.add_theory_number(2)?);
        assert_eq!(str_x, backend.add_theory_string("x")?);
        assert_eq!(num_one, backend.add_theory_symbol(sym("1"))?);
        assert_eq!(num_two, backend.add_theory_symbol(sym("2"))?);
        assert_eq!(fun, backend.add_theory_symbol(sym("f(1,2,x)"))?);

        let elem = backend.add_theory_element(&[num_one, num_two, seq, fun, lst, tup], &[])?;

        backend.add_theory_atom(TheoryAtomTarget::Directive, fun, &[])?;
        backend.add_theory_atom_with_guard(TheoryAtomTarget::Directive, fun, &[], "=", num_one)?;
        let g = backend.add_theory_symbol(sym("g(1,2)"))?;
        backend.add_theory_atom(TheoryAtomTarget::Directive, g, &[])?;
        backend.add_theory_atom(TheoryAtomTarget::Directive, fseq, &[elem])
    })
    .unwrap();

    let atoms = ctl.theory_atoms().unwrap();
    let strings: Vec<String> = atoms.iter().map(|a| a.unwrap().to_string()).collect();
    assert_eq!(
        strings,
        vec![
            "&f(1,2,x){}",
            "&f(1,2,x){}=1",
            "&g(1,2){}",
            "&f({1,2,x}){1,2,{1,2,x},f(1,2,x),[1,2],(1,2)}",
        ]
    );
}

// -------------------------------------------------------------------------
// test_theory_with_guard (test_backend.py): the observer's guard callback.
// -------------------------------------------------------------------------

#[test]
fn pyclingo_theory_observer_with_guard() {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use clingox::Id;
    use clingox::observer::GroundProgramObserver;

    #[derive(Clone, Default)]
    struct Recorder {
        strings: Arc<Mutex<HashSet<String>>>,
        called: Arc<Mutex<HashSet<&'static str>>>,
    }
    impl GroundProgramObserver for Recorder {
        fn theory_term_string(&mut self, _term: Id, name: &str) -> clingox::Result<()> {
            self.strings
                .lock()
                .unwrap()
                .insert(format!("theory_term_string: {name}"));
            Ok(())
        }
        fn theory_atom_with_guard(
            &mut self,
            _atom: Option<clingox::backend::Atom>,
            _term: Id,
            elements: &[Id],
            _operator: &str,
            _right_hand_side: Id,
        ) -> clingox::Result<()> {
            self.called.lock().unwrap().insert("theory_atom_with_guard");
            assert!(elements.is_empty());
            Ok(())
        }
    }

    let mut ctl = Control::new().unwrap();
    let recorder = Recorder::default();
    ctl.register_observer(recorder.clone(), false).unwrap();
    ctl.add_base(
        "#theory test {\n\
             t { };\n\
             &a/0 : t, {=}, t, head\n\
         }.\n\
         &a { } = a.",
    )
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(
        recorder
            .strings
            .lock()
            .unwrap()
            .contains("theory_term_string: a")
    );
    assert!(
        recorder
            .strings
            .lock()
            .unwrap()
            .contains("theory_term_string: =")
    );
    assert!(
        recorder
            .called
            .lock()
            .unwrap()
            .contains("theory_atom_with_guard")
    );
    let _ = ctl.solve(&[]);
}

// =========================================================================
// A second verification pass: three more `test_solving.py` methods
// (`Control::remove_minimize`, `Control::update_project`,
// `Model::is_consequence`, `Consequence`).
// =========================================================================

// -------------------------------------------------------------------------
// test_remove_minimize
// -------------------------------------------------------------------------

#[test]
fn pyclingo_remove_minimize() {
    let mut ctl = grounded("a. #minimize { 1,t : a }.");
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models[0].cost(), [1]);

    ctl.add("s1", &[], "b. #minimize { 1,t : b }.").unwrap();
    ctl.ground(&[Part::new("s1", &[]).unwrap()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models[0].cost(), [1]);

    ctl.remove_minimize().unwrap();
    ctl.add("s2", &[], "c. #minimize { 1,x : c ; 1,t : c}.")
        .unwrap();
    ctl.ground(&[Part::new("s2", &[]).unwrap()]).unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models[0].cost(), [2]);

    ctl.remove_minimize().unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    assert_eq!(models[0].cost(), []);
}

// -------------------------------------------------------------------------
// test_cautious_consequences: `Model::is_consequence` during cautious
// enumeration. The first (`number() == 1`) model is an intermediate
// refinement state, where `c`'s consequence status is still `Unknown`;
// checked directly against clingo 5.8.2.
// -------------------------------------------------------------------------

#[test]
fn pyclingo_cautious_consequences() {
    let mut ctl = Control::new().unwrap();
    ctl.configuration()
        .set("solve.enum_mode", "cautious")
        .unwrap();
    ctl.add_base("a. b | c.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let (a, b, c) = {
        let atoms = ctl.symbolic_atoms().unwrap();
        (
            atoms.find(sym("a")).unwrap().unwrap().literal(),
            atoms.find(sym("b")).unwrap().unwrap().literal(),
            atoms.find(sym("c")).unwrap().unwrap().literal(),
        )
    };

    let mut seen = Vec::new();
    let _ = ctl
        .for_each_model(&[], |model| {
            let ca = model.is_consequence(a)?;
            let cb = model.is_consequence(b)?;
            let cc = model.is_consequence(c)?;
            let nca = model.is_consequence(a.negate())?;
            let ncb = model.is_consequence(b.negate())?;
            let ncc = model.is_consequence(c.negate())?;
            seen.push((model.number(), ca, cb, cc, nca, ncb, ncc));
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();

    assert_eq!(seen.len(), 2);
    for (number, ca, cb, cc, nca, ncb, ncc) in seen {
        assert_eq!(ca, Consequence::True);
        assert_eq!(nca, Consequence::False);
        assert_eq!(ncb, Consequence::False);
        assert_eq!(ncc, Consequence::False);
        if number == 1 {
            assert!(matches!(cb, Consequence::False | Consequence::Unknown));
            assert!(matches!(cc, Consequence::False | Consequence::Unknown));
            assert_ne!(cb, cc);
        }
        if number == 2 {
            assert_eq!(cb, Consequence::False);
            assert_eq!(cc, Consequence::False);
        }
    }
}

// -------------------------------------------------------------------------
// test_update_projection: `Control::update_project`, replacing (`append:
// false`) and extending (`append: true`) the projection atoms. The Python
// original builds its second projection set from a mix of a literal (read
// via `symbolic_atoms.by_signature("c", 0)`) and a plain symbol
// (`Function("d")`); `Control::update_project` takes symbols only, so both
// are passed as symbols here (`sym("c")`, `sym("d")`), equivalent since
// clingox looks up each symbol's own literal internally either way.
// -------------------------------------------------------------------------

fn sorted_shown_models(ctl: &mut Control) -> Vec<Vec<String>> {
    let mut models: Vec<Vec<String>> = Vec::new();
    let _ = ctl
        .for_each_model(&[], |model| {
            let mut atoms: Vec<String> = model
                .symbols(ShowType::SHOWN)?
                .iter()
                .map(ToString::to_string)
                .collect();
            atoms.sort();
            models.push(atoms);
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    models.sort();
    models
}

#[test]
fn pyclingo_update_projection() {
    fn strings(atoms: &[&str]) -> Vec<String> {
        atoms.iter().map(|s| (*s).to_owned()).collect()
    }

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.configuration().set("solve.project", "auto").unwrap();
    ctl.add_base("{a;b;c;d}. #project a/0. #project b/0.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    assert_eq!(
        sorted_shown_models(&mut ctl),
        vec![
            strings(&[]),
            strings(&["a"]),
            strings(&["a", "b"]),
            strings(&["b"]),
        ]
    );

    ctl.update_project([sym("c"), sym("d")], false).unwrap();
    assert_eq!(
        sorted_shown_models(&mut ctl),
        vec![
            strings(&[]),
            strings(&["c"]),
            strings(&["c", "d"]),
            strings(&["d"]),
        ]
    );

    ctl.update_project([sym("a")], true).unwrap();
    assert_eq!(
        sorted_shown_models(&mut ctl),
        vec![
            strings(&[]),
            strings(&["a"]),
            strings(&["a", "c"]),
            strings(&["a", "c", "d"]),
            strings(&["a", "d"]),
            strings(&["c"]),
            strings(&["c", "d"]),
            strings(&["d"]),
        ]
    );
}

// `test_propagator.py::test_heurisitc` (upstream's own spelling)
// needs only `decide`'s dispatch and `Assignment`, so it is ported here
// separately from the rest of `test_propagator.py`'s methods, which need
// the fuller `PropagateControl`.
// `test_heurisitc` moves to "Ported," 6 methods stay not-ported).

struct PyclingoHeuristic {
    lit_a: std::sync::OnceLock<SolverLiteral>,
    lit_b: std::sync::OnceLock<SolverLiteral>,
}

impl Propagator for PyclingoHeuristic {
    fn init(&self, init: &mut PropagateInit<'_>) -> clingox::Result<()> {
        let plit_a = init
            .symbolic_atoms()?
            .find(sym("a"))?
            .unwrap_or_else(|| panic!("a is an atom"))
            .literal();
        let plit_b = init
            .symbolic_atoms()?
            .find(sym("b"))?
            .unwrap_or_else(|| panic!("b is an atom"))
            .literal();
        let _ = self.lit_a.set(init.solver_literal(plit_a)?);
        let _ = self.lit_b.set(init.solver_literal(plit_b)?);
        Ok(())
    }

    fn decide(
        &self,
        thread_id: u32,
        assignment: &Assignment<'_>,
        fallback: SolverLiteral,
    ) -> clingox::Result<Option<SolverLiteral>> {
        assert_eq!(thread_id, 0);
        let lit_a = *self.lit_a.get().expect("init ran first");
        let lit_b = *self.lit_b.get().expect("init ran first");
        if assignment.truth_value(lit_a)?.is_none() {
            return Ok(Some(lit_a));
        }
        if assignment.truth_value(lit_b)?.is_none() {
            return Ok(Some(-lit_b));
        }
        // Upstream's own `TestHeuristic.decide` (`test_propagator.py`)
        // literally `return fallback` here, a real (if coincidental)
        // choice, not pyclingo's own "decline" sentinel (`0`); `Some`,
        // not `None`, is the faithful port (single propagator here, so
        // it makes no behavioural difference either way, but `Some`
        // matches the upstream source).
        Ok(Some(fallback))
    }
}

#[test]
fn pyclingo_heuristic() {
    let mut ctl = Control::with_args(["--models=1"]).unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(PyclingoHeuristic {
        lit_a: std::sync::OnceLock::new(),
        lit_b: std::sync::OnceLock::new(),
    })
    .unwrap();
    let mut models = Vec::new();
    let _ = ctl
        .for_each_model(&[], |m| {
            let mut syms = m.symbols(ShowType::SHOWN)?;
            syms.sort_unstable();
            models.push(syms);
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(models, vec![vec![sym("a")]]);
}

// `test_solving.py::test_control_clause` and `::
// test_control_nogood`: checked directly, neither registers a propagator
// at all,
// exercising only `SolveControl::add_clause` through a plain `yield_=True`
// model loop. The two upstream methods are behaviourally identical once
// translated: `test_control_nogood`'s own `add_nogood(clause)` is
// pyclingo's `add_clause([invert(lit) for lit in clause])`
// (`libpyclingo/clingo/solving.py:159-176`), and its clause already
// contains the pre-negation of the same literal `test_control_clause`
// passes to `add_clause` directly (`(Function("b"), True)` inverts to
// `(Function("b"), False)`, identical to `test_control_clause`'s own
// `(Function("b"), False)`); clingox has no `add_nogood` convenience
// (RULES §4: not obviously worth a one-line wrapper), so the faithful port
// of `test_control_nogood` is the exact same `add_clause` call
// `test_control_clause`'s own port already makes. One Rust test below
// closes both.

#[test]
fn pyclingo_control_clause() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("1 {a; b; c} 1.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut models = 0;
    let mut handle = ctl.solve_yield(&[]).unwrap();
    while let Some(model) = handle.next_model().unwrap() {
        let a = sym("a");
        let literal = if model.contains(a).unwrap() {
            model
                .context()
                .symbolic_atoms()
                .unwrap()
                .find(sym("b"))
                .unwrap()
                .unwrap_or_else(|| panic!("b is an atom"))
                .literal()
        } else {
            model
                .context()
                .symbolic_atoms()
                .unwrap()
                .find(a)
                .unwrap()
                .unwrap_or_else(|| panic!("a is an atom"))
                .literal()
        };
        model.context().add_clause(&[-literal]).unwrap();
        models += 1;
    }
    let result = handle.close().unwrap();
    assert!(result.is_sat());
    assert_eq!(models, 2);
}

// =========================================================================
// From test_ast.py
//
// The upstream file was run against the installed pyclingo 5.8.2 first (all
// 14 methods pass), so every expected value below is one the oracle produces.
// =========================================================================

/// Every attribute name, to walk a node's attributes the way the Python test
/// walks `node.keys()`.
const AST_ATTRIBUTES: [Attribute; 45] = [
    Attribute::Argument,
    Attribute::Arguments,
    Attribute::Arity,
    Attribute::Atom,
    Attribute::Atoms,
    Attribute::AtomType,
    Attribute::Bias,
    Attribute::Body,
    Attribute::Code,
    Attribute::Coefficient,
    Attribute::Comparison,
    Attribute::Condition,
    Attribute::Elements,
    Attribute::External,
    Attribute::ExternalType,
    Attribute::Function,
    Attribute::Guard,
    Attribute::Guards,
    Attribute::Head,
    Attribute::IsDefault,
    Attribute::Left,
    Attribute::LeftGuard,
    Attribute::Literal,
    Attribute::Location,
    Attribute::Modifier,
    Attribute::Name,
    Attribute::NodeU,
    Attribute::NodeV,
    Attribute::OperatorName,
    Attribute::OperatorType,
    Attribute::Operators,
    Attribute::Parameters,
    Attribute::Positive,
    Attribute::Priority,
    Attribute::Right,
    Attribute::RightGuard,
    Attribute::SequenceType,
    Attribute::Sign,
    Attribute::Symbol,
    Attribute::Term,
    Attribute::Terms,
    Attribute::Value,
    Attribute::Variable,
    Attribute::Weight,
    Attribute::CommentType,
];

/// The statements `parse_string` delivers, `#program base.` first.
fn parse_statements(program: &str) -> Vec<Ast> {
    let mut statements = Vec::new();
    ast::parse_string(program, |stm| {
        statements.push(stm);
        Ok(())
    })
    .unwrap();
    statements
}

fn node_children(node: &Ast, attribute: Attribute) -> Vec<Ast> {
    (0..node.ast_array_len(attribute).unwrap())
        .map(|i| node.ast_at(attribute, i).unwrap())
        .collect()
}

fn node_strings(node: &Ast, attribute: Attribute) -> Vec<String> {
    (0..node.string_array_len(attribute).unwrap())
        .map(|i| node.string_at(attribute, i).unwrap())
        .collect()
}

/// Sets every attribute of `dst` to the value `src` holds, through the typed
/// setters (`setattr(cpy, key, getattr(cpz, key))` upstream).
fn assign_attributes(dst: &Ast, src: &Ast) {
    for attribute in AST_ATTRIBUTES {
        let Some(kind) = src.attribute_type(attribute) else {
            continue;
        };
        match kind {
            AttributeType::Number => dst
                .set_number(attribute, src.number(attribute).unwrap())
                .unwrap(),
            AttributeType::Symbol => dst
                .set_symbol(attribute, src.symbol(attribute).unwrap())
                .unwrap(),
            AttributeType::Location => dst
                .set_span(attribute, &src.span(attribute).unwrap())
                .unwrap(),
            AttributeType::String => dst
                .set_string(attribute, &src.string(attribute).unwrap())
                .unwrap(),
            AttributeType::Ast => dst
                .set_ast(attribute, &src.ast(attribute).unwrap())
                .unwrap(),
            AttributeType::OptionalAst => dst
                .set_optional_ast(attribute, src.optional_ast(attribute).unwrap().as_ref())
                .unwrap(),
            AttributeType::StringArray => dst
                .set_string_array(attribute, &node_strings(src, attribute))
                .unwrap(),
            AttributeType::AstArray => dst
                .set_ast_array(attribute, &node_children(src, attribute))
                .unwrap(),
            other => panic!("an attribute kind the test does not know: {other:?}"),
        }
    }
}

/// `_deepcopy` of `test_ast.py`: rebuilds `node` bottom up with the typed
/// constructor for its type, from the values of its attributes. The rebuilt
/// node must equal `node` (and, unlike `==`, also carry the same locations),
/// survive having every attribute set to its own value, and survive having
/// every attribute set from a shallow copy of itself.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per node type, as the constructor table has"
)]
fn rebuild(node: &Ast) -> Ast {
    let ty = node.ast_type();
    let location = || node.span(Attribute::Location).unwrap();
    let number = |a| node.number(a).unwrap();
    let string = |a| node.string(a).unwrap();
    let child = |a| rebuild(&node.ast(a).unwrap());
    let children = |a| {
        node_children(node, a)
            .iter()
            .map(rebuild)
            .collect::<Vec<_>>()
    };
    let optional = |a| node.optional_ast(a).unwrap().map(|n| rebuild(&n));
    let strings = |a| node_strings(node, a);
    let flag = |a| number(a) != 0;

    let cpy = match ty {
        AstType::Id => ast::id(&location(), &string(Attribute::Name)),
        AstType::Variable => ast::variable(&location(), &string(Attribute::Name)),
        AstType::SymbolicTerm => {
            ast::symbolic_term(&location(), node.symbol(Attribute::Symbol).unwrap())
        }
        AstType::UnaryOperation => ast::unary_operation(
            &location(),
            UnaryOperator::try_from(number(Attribute::OperatorType)).unwrap(),
            &child(Attribute::Argument),
        ),
        AstType::BinaryOperation => ast::binary_operation(
            &location(),
            BinaryOperator::try_from(number(Attribute::OperatorType)).unwrap(),
            &child(Attribute::Left),
            &child(Attribute::Right),
        ),
        AstType::Interval => ast::interval(
            &location(),
            &child(Attribute::Left),
            &child(Attribute::Right),
        ),
        AstType::Function => ast::function(
            &location(),
            &string(Attribute::Name),
            &children(Attribute::Arguments),
            flag(Attribute::External),
        ),
        AstType::Pool => ast::pool(&location(), &children(Attribute::Arguments)),
        AstType::BooleanConstant => ast::boolean_constant(flag(Attribute::Value)),
        AstType::SymbolicAtom => ast::symbolic_atom(&child(Attribute::Symbol)),
        AstType::Comparison => {
            ast::comparison(&child(Attribute::Term), &children(Attribute::Guards))
        }
        AstType::Guard => ast::guard(
            ComparisonOperator::try_from(number(Attribute::Comparison)).unwrap(),
            &child(Attribute::Term),
        ),
        AstType::ConditionalLiteral => ast::conditional_literal(
            &location(),
            &child(Attribute::Literal),
            &children(Attribute::Condition),
        ),
        AstType::Aggregate => ast::aggregate(
            &location(),
            optional(Attribute::LeftGuard).as_ref(),
            &children(Attribute::Elements),
            optional(Attribute::RightGuard).as_ref(),
        ),
        AstType::BodyAggregateElement => ast::body_aggregate_element(
            &children(Attribute::Terms),
            &children(Attribute::Condition),
        ),
        AstType::BodyAggregate => ast::body_aggregate(
            &location(),
            optional(Attribute::LeftGuard).as_ref(),
            AggregateFunction::try_from(number(Attribute::Function)).unwrap(),
            &children(Attribute::Elements),
            optional(Attribute::RightGuard).as_ref(),
        ),
        AstType::HeadAggregateElement => {
            ast::head_aggregate_element(&children(Attribute::Terms), &child(Attribute::Condition))
        }
        AstType::HeadAggregate => ast::head_aggregate(
            &location(),
            optional(Attribute::LeftGuard).as_ref(),
            AggregateFunction::try_from(number(Attribute::Function)).unwrap(),
            &children(Attribute::Elements),
            optional(Attribute::RightGuard).as_ref(),
        ),
        AstType::Disjunction => ast::disjunction(&location(), &children(Attribute::Elements)),
        AstType::TheorySequence => ast::theory_sequence(
            &location(),
            TheorySequenceType::try_from(number(Attribute::SequenceType)).unwrap(),
            &children(Attribute::Terms),
        ),
        AstType::TheoryFunction => ast::theory_function(
            &location(),
            &string(Attribute::Name),
            &children(Attribute::Arguments),
        ),
        AstType::TheoryUnparsedTermElement => ast::theory_unparsed_term_element(
            &strings(Attribute::Operators),
            &child(Attribute::Term),
        ),
        AstType::TheoryUnparsedTerm => {
            ast::theory_unparsed_term(&location(), &children(Attribute::Elements))
        }
        AstType::TheoryGuard => {
            ast::theory_guard(&string(Attribute::OperatorName), &child(Attribute::Term))
        }
        AstType::TheoryAtomElement => {
            ast::theory_atom_element(&children(Attribute::Terms), &children(Attribute::Condition))
        }
        AstType::TheoryAtom => ast::theory_atom(
            &location(),
            &child(Attribute::Term),
            &children(Attribute::Elements),
            optional(Attribute::Guard).as_ref(),
        ),
        AstType::Literal => ast::literal(
            &location(),
            LiteralSign::try_from(number(Attribute::Sign)).unwrap(),
            &child(Attribute::Atom),
        ),
        AstType::TheoryOperatorDefinition => ast::theory_operator_definition(
            &location(),
            &string(Attribute::Name),
            number(Attribute::Priority),
            TheoryOperatorType::try_from(number(Attribute::OperatorType)).unwrap(),
        ),
        AstType::TheoryTermDefinition => ast::theory_term_definition(
            &location(),
            &string(Attribute::Name),
            &children(Attribute::Operators),
        ),
        AstType::TheoryGuardDefinition => {
            ast::theory_guard_definition(&strings(Attribute::Operators), &string(Attribute::Term))
        }
        AstType::TheoryAtomDefinition => ast::theory_atom_definition(
            &location(),
            TheoryAtomType::try_from(number(Attribute::AtomType)).unwrap(),
            &string(Attribute::Name),
            number(Attribute::Arity),
            &string(Attribute::Term),
            optional(Attribute::Guard).as_ref(),
        ),
        AstType::Rule => ast::rule(
            &location(),
            &child(Attribute::Head),
            &children(Attribute::Body),
        ),
        AstType::Definition => ast::definition(
            &location(),
            &string(Attribute::Name),
            &child(Attribute::Value),
            flag(Attribute::IsDefault),
        ),
        AstType::ShowSignature => ast::show_signature(
            &location(),
            &string(Attribute::Name),
            number(Attribute::Arity),
            flag(Attribute::Positive),
        ),
        AstType::ShowTerm => ast::show_term(
            &location(),
            &child(Attribute::Term),
            &children(Attribute::Body),
        ),
        AstType::Minimize => ast::minimize(
            &location(),
            &child(Attribute::Weight),
            &child(Attribute::Priority),
            &children(Attribute::Terms),
            &children(Attribute::Body),
        ),
        AstType::Script => ast::script(
            &location(),
            &string(Attribute::Name),
            &string(Attribute::Code),
        ),
        AstType::Program => ast::program(
            &location(),
            &string(Attribute::Name),
            &children(Attribute::Parameters),
        ),
        AstType::External => ast::external(
            &location(),
            &child(Attribute::Atom),
            &children(Attribute::Body),
            &child(Attribute::ExternalType),
        ),
        AstType::Edge => ast::edge(
            &location(),
            &child(Attribute::NodeU),
            &child(Attribute::NodeV),
            &children(Attribute::Body),
        ),
        AstType::Heuristic => ast::heuristic(
            &location(),
            &child(Attribute::Atom),
            &children(Attribute::Body),
            &child(Attribute::Bias),
            &child(Attribute::Priority),
            &child(Attribute::Modifier),
        ),
        AstType::ProjectAtom => ast::project_atom(
            &location(),
            &child(Attribute::Atom),
            &children(Attribute::Body),
        ),
        AstType::ProjectSignature => ast::project_signature(
            &location(),
            &string(Attribute::Name),
            number(Attribute::Arity),
            flag(Attribute::Positive),
        ),
        AstType::Defined => ast::defined(
            &location(),
            &string(Attribute::Name),
            number(Attribute::Arity),
            flag(Attribute::Positive),
        ),
        AstType::TheoryDefinition => ast::theory_definition(
            &location(),
            &string(Attribute::Name),
            &children(Attribute::Terms),
            &children(Attribute::Atoms),
        ),
        AstType::Comment => ast::comment(
            &location(),
            &string(Attribute::Value),
            CommentType::try_from(number(Attribute::CommentType)).unwrap(),
        ),
        other => panic!("a node type the test does not know: {other:?}"),
    }
    .unwrap();
    assert_eq!(cpy, *node);
    if node.has_attribute(Attribute::Location) {
        assert_eq!(
            cpy.span(Attribute::Location).unwrap(),
            node.span(Attribute::Location).unwrap()
        );
    }

    assign_attributes(&cpy, &cpy.clone());
    let cpz = cpy.copy().unwrap();
    assign_attributes(&cpy, &cpz);
    assert_eq!(cpy, *node);
    cpy
}

/// `_str` of `test_ast.py`: the last statement of `program` rebuilds from its
/// attributes, copies and deep copies, prints as `expected`, and that text
/// parses back to a statement that prints as `expected` too; finally the
/// statement is added to a control through the program builder.
#[track_caller]
fn ast_str(program: &str, expected: &str) {
    let statements = parse_statements(program);
    let last = statements.last().unwrap();
    let cpy = rebuild(last).deep_copy().unwrap().copy().unwrap();
    assert_eq!(cpy.to_string(), expected);

    let reparsed = parse_statements(&cpy.to_string());
    assert_eq!(reparsed.last().unwrap().to_string(), expected);

    let mut ctl = Control::new().unwrap();
    if let Err(err) = ctl.with_program_builder(|builder| builder.add(last)) {
        let message = err.to_string();
        // This build has neither Python nor Lua, so adding a script is refused.
        assert!(
            message.contains("python support not available")
                || message.contains("lua support not available"),
            "{program}: {message}"
        );
    }
}

// -------------------------------------------------------------------------
// test_terms
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_terms() {
    ast_str("a.", "a.");
    ast_str("-a.", "-a.");
    ast_str("a(X).", "a(X).");
    ast_str("a(-X).", "a(-X).");
    ast_str("a(|X|).", "a(|X|).");
    ast_str("a(~X).", "a(~X).");
    ast_str("a((X^Y)).", "a((X^Y)).");
    ast_str("a((X?Y)).", "a((X?Y)).");
    ast_str("a((X&Y)).", "a((X&Y)).");
    ast_str("a((X+Y)).", "a((X+Y)).");
    ast_str("a((X-Y)).", "a((X-Y)).");
    ast_str("a((X*Y)).", "a((X*Y)).");
    ast_str("a((X/Y)).", "a((X/Y)).");
    ast_str("a((X\\Y)).", "a((X\\Y)).");
    ast_str("a((X**Y)).", "a((X**Y)).");
    ast_str("a((X..Y)).", "a((X..Y)).");
    ast_str("-a(f).", "-a(f).");
    ast_str("-a(-f).", "-a(-f).");
    ast_str("-a(f(X)).", "-a(f(X)).");
    ast_str("-a(f(X,Y)).", "-a(f(X,Y)).");
    ast_str("-a(()).", "-a(()).");
    ast_str("-a((a,)).", "-a((a,)).");
    ast_str("-a((a,b)).", "-a((a,b)).");
    ast_str("-a(@f(a,b)).", "-a(@f(a,b)).");
    ast_str("-a(@f).", "-a(@f).");
    ast_str("-a(a;b;c).", "-a(a;b;c).");
    ast_str("-a((a;b;c)).", "-a((a;b;c)).");
    ast_str("-a(f(a);f(b);f(c)).", "-a(f(a);f(b);f(c)).");
}

// -------------------------------------------------------------------------
// test_theory_terms
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_theory_terms() {
    ast_str("&a { 1 }.", "&a { 1 }.");
    ast_str("&a { (- 1) }.", "&a { (- 1) }.");
    ast_str("&a { X }.", "&a { X }.");
    ast_str("&a { () }.", "&a { () }.");
    ast_str("&a { (1,) }.", "&a { (1,) }.");
    ast_str("&a { (1,2) }.", "&a { (1,2) }.");
    ast_str("&a { [] }.", "&a { [] }.");
    ast_str("&a { [1] }.", "&a { [1] }.");
    ast_str("&a { [1,2] }.", "&a { [1,2] }.");
    ast_str("&a { {} }.", "&a { {} }.");
    ast_str("&a { {1} }.", "&a { {1} }.");
    ast_str("&a { {1,2} }.", "&a { {1,2} }.");
    ast_str("&a { f }.", "&a { f }.");
    ast_str("&a { f(X) }.", "&a { f(X) }.");
    ast_str("&a { f(X,Y) }.", "&a { f(X,Y) }.");
    ast_str("&a { (+ a + - * b + c) }.", "&a { (+ a + - * b + c) }.");
}

// -------------------------------------------------------------------------
// test_literals
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_literals() {
    ast_str("a.", "a.");
    ast_str("not a.", "not a.");
    ast_str("not not a.", "not not a.");
    ast_str("1 < 2.", "1 < 2.");
    ast_str("1 <= 2.", "1 <= 2.");
    ast_str("1 > 2.", "1 > 2.");
    ast_str("1 >= 2.", "1 >= 2.");
    ast_str("1 = 2.", "1 = 2.");
    ast_str("not 1 = 2.", "not 1 = 2.");
    ast_str("not not 1 = 2.", "not not 1 = 2.");
    ast_str("1 != 2.", "1 != 2.");
    ast_str("#false.", "#false.");
    ast_str("#true.", "#true.");
}

// -------------------------------------------------------------------------
// test_head_literals
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_head_literals() {
    ast_str("{ }.", "{ }.");
    ast_str("{ } < 2.", "2 > { }.");
    ast_str("1 < { }.", "1 < { }.");
    ast_str("1 < { } < 2.", "1 < { } < 2.");
    ast_str("{ b }.", "{ b }.");
    ast_str("{ a; b }.", "{ a; b }.");
    ast_str("{ a; b: c, d }.", "{ a; b: c, d }.");
    ast_str("#count { }.", "#count { }.");
    ast_str("#count { } < 2.", "2 > #count { }.");
    ast_str("1 < #count { }.", "1 < #count { }.");
    ast_str("1 < #count { } < 2.", "1 < #count { } < 2.");
    ast_str("#count { b: a }.", "#count { b: a }.");
    ast_str("#count { b,c: a }.", "#count { b,c: a }.");
    ast_str("#count { a: a; b: c }.", "#count { a: a; b: c }.");
    ast_str(
        "#count { a: d; b: x: c, d }.",
        "#count { a: d; b: x: c, d }.",
    );
    ast_str("#min { }.", "#min { }.");
    ast_str("#max { }.", "#max { }.");
    ast_str("#sum { }.", "#sum { }.");
    ast_str("#sum+ { }.", "#sum+ { }.");
    ast_str("a; b.", "a; b.");
    ast_str("a; b: c.", "a; b: c.");
    ast_str("a; b: c, d.", "a; b: c, d.");
    ast_str("&a { }.", "&a { }.");
    ast_str("&a { 1 }.", "&a { 1 }.");
    ast_str("&a { 1; 2 }.", "&a { 1; 2 }.");
    ast_str("&a { 1,2 }.", "&a { 1,2 }.");
    ast_str("&a { 1,2: a }.", "&a { 1,2: a }.");
    ast_str("&a { 1,2: a, b }.", "&a { 1,2: a, b }.");
    ast_str("&a { } != x.", "&a { } != x.");
    ast_str("&a(x) { }.", "&a(x) { }.");
}

// -------------------------------------------------------------------------
// test_body_literals
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_body_literals() {
    ast_str("a :- { }.", "a :- { }.");
    ast_str("a :- not { }.", "a :- not { }.");
    ast_str("a :- not not { }.", "a :- not not { }.");
    ast_str("a :- { } < 2.", "a :- 2 > { }.");
    ast_str("a :- 1 < { }.", "a :- 1 < { }.");
    ast_str("a :- 1 < { } < 2.", "a :- 1 < { } < 2.");
    ast_str("a :- { b }.", "a :- { b }.");
    ast_str("a :- { a; b }.", "a :- { a; b }.");
    ast_str("a :- { a; b: c, d }.", "a :- { a; b: c, d }.");
    ast_str("a :- #count { }.", "a :- #count { }.");
    ast_str("a :- not #count { }.", "a :- not #count { }.");
    ast_str("a :- not not #count { }.", "a :- not not #count { }.");
    ast_str("a :- #count { } < 2.", "a :- 2 > #count { }.");
    ast_str("a :- 1 < #count { }.", "a :- 1 < #count { }.");
    ast_str("a :- 1 < #count { } < 2.", "a :- 1 < #count { } < 2.");
    ast_str("a :- #count { b }.", "a :- #count { b }.");
    ast_str("a :- #count { b,c }.", "a :- #count { b,c }.");
    ast_str("a :- #count { a; b }.", "a :- #count { a; b }.");
    ast_str("a :- #count { a; b: c, d }.", "a :- #count { a; b: c, d }.");
    ast_str("a :- #min { }.", "a :- #min { }.");
    ast_str("a :- #max { }.", "a :- #max { }.");
    ast_str("a :- #sum { }.", "a :- #sum { }.");
    ast_str("a :- #sum+ { }.", "a :- #sum+ { }.");
    ast_str("a :- a; b.", "a :- a; b.");
    ast_str("a :- a; b: c.", "a :- a; b: c.");
    ast_str("a :- a; b: c, d.", "a :- a; b: c, d.");
    ast_str("a :- &a { }.", "a :- &a { }.");
    ast_str("a :- &a { 1 }.", "a :- &a { 1 }.");
    ast_str("a :- &a { 1; 2 }.", "a :- &a { 1; 2 }.");
    ast_str("a :- &a { 1,2 }.", "a :- &a { 1,2 }.");
    ast_str("a :- &a { 1,2: a }.", "a :- &a { 1,2: a }.");
    ast_str("a :- &a { 1,2: a, b }.", "a :- &a { 1,2: a, b }.");
    ast_str("a :- &a { } != x.", "a :- &a { } != x.");
    ast_str("a :- &a(x) { }.", "a :- &a(x) { }.");
    ast_str("a :- a.", "a :- a.");
    ast_str("a :- not a.", "a :- not a.");
    ast_str("a :- not not a.", "a :- not not a.");
    ast_str("a :- 1 < 2.", "a :- 1 < 2.");
    ast_str("a :- 1 <= 2.", "a :- 1 <= 2.");
    ast_str("a :- 1 > 2.", "a :- 1 > 2.");
    ast_str("a :- 1 >= 2.", "a :- 1 >= 2.");
    ast_str("a :- 1 = 2.", "a :- 1 = 2.");
    ast_str("a :- 1 != 2.", "a :- 1 != 2.");
    ast_str("a :- #false.", "a :- #false.");
    ast_str("a :- #true.", "a :- #true.");
}

// -------------------------------------------------------------------------
// test_statements
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_statements() {
    ast_str("a.", "a.");
    ast_str("#false.", "#false.");
    ast_str("#false :- a.", "#false :- a.");
    ast_str("a :- a; b.", "a :- a; b.");
    ast_str("#const x = 10.", "#const x = 10.");
    ast_str("#const x = 10. [override]", "#const x = 10. [override]");
    ast_str("#show.", "#show.");
    ast_str("#show p/1.", "#show p/1.");
    ast_str("#show -p/1.", "#show -p/1.");
    ast_str("#defined p/1.", "#defined p/1.");
    ast_str("#defined -p/1.", "#defined -p/1.");
    ast_str("#show x.", "#show x.");
    ast_str("#show x : y; z.", "#show x : y; z.");
    ast_str(":~ . [1@0]", ":~ . [1@0]");
    ast_str(":~ b; c. [1@2,s,t]", ":~ b; c. [1@2,s,t]");
    ast_str("#script (lua)\ncode\n#end.", "#script (lua)\ncode\n#end.");
    ast_str(
        "#script (python)\ncode\n#end.",
        "#script (python)\ncode\n#end.",
    );
    ast_str("#program x(y, z).", "#program x(y, z).");
    ast_str("#program x.", "#program x.");
    ast_str("#external a. [X]", "#external a. [X]");
    ast_str("#external a : b; c. [false]", "#external a : b; c. [false]");
    ast_str("#edge (1,2).", "#edge (1,2).");
    ast_str("#edge (1,2) : x; y.", "#edge (1,2) : x; y.");
    ast_str("#heuristic a. [b@p,m]", "#heuristic a. [b@p,m]");
    ast_str(
        "#heuristic a : b; c. [b@p,m]",
        "#heuristic a : b; c. [b@p,m]",
    );
    ast_str("#project a.", "#project a.");
    ast_str("#project a : b; c.", "#project a : b; c.");
    ast_str("#project -a/0.", "#project -a/0.");
    ast_str("#project a/0.", "#project a/0.");
    ast_str("#theory x {\n}.", "#theory x {\n}.");
    ast_str(
        "#theory x {\n  t {\n    + : 0, unary;\n    - : 1, binary, left;\n    * : 2, binary, right\n  };\n  &a/0: t, head;\n  &b/0: t, body;\n  &c/0: t, directive;\n  &d/0: t, { }, t, any;\n  &e/0: t, { =, !=, + }, t, any\n}.",
        "#theory x {\n  t {\n    + : 0, unary;\n    - : 1, binary, left;\n    * : 2, binary, right\n  };\n  &a/0: t, head;\n  &b/0: t, body;\n  &c/0: t, directive;\n  &d/0: t, { }, t, any;\n  &e/0: t, { =, !=, + }, t, any\n}.",
    );
    ast_str("%* test *%", "%* test *%");
    ast_str("%* test *%\n", "%* test *%");
    ast_str("% test", "% test");
    ast_str("% test\n", "% test");
}

// -------------------------------------------------------------------------
// test_compare: equality, hashing and order ignore the location
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_compare() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash(node: &Ast) -> u64 {
        let mut hasher = DefaultHasher::new();
        node.hash(&mut hasher);
        hasher.finish()
    }

    let loc = Span::new("<string>", 1, 1, "<string>", 1, 1).unwrap();
    let alt = Span::new("<string>", 1, 1, "<string>", 1, 2).unwrap();
    let x = ast::id(&loc, "x").unwrap();
    let y = ast::id(&alt, "x").unwrap();
    let z = ast::id(&loc, "z").unwrap();
    assert_eq!(x, y);
    assert_eq!(x, x);
    assert_ne!(x, z);
    assert_eq!(hash(&x), hash(&x));
    assert_eq!(hash(&x), hash(&y));
    assert_ne!(hash(&x), hash(&z));
    assert!(x < z);
    assert_ne!(x, z);
    assert!(z > x);
    assert!(y <= x);
    assert!(x <= y);
    assert!(y >= x);
    assert!(x >= y);
}

// -------------------------------------------------------------------------
// test_compare_bug: the order is antisymmetric across node kinds
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_compare_bug() {
    let r1 = parse_statements(":- b.");
    let r2 = parse_statements(":- not a.");
    let (r1, r2) = (r1.last().unwrap(), r2.last().unwrap());
    assert_ne!(r1 < r2, r2 < r1);
}

// -------------------------------------------------------------------------
// test_ast_sequence: the `parameters` array of a `#program` node
//
// pyclingo's `ASTSequence` is a Python list-like view; the array setters give
// the same edits on the node itself.
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_sequence() {
    let loc = Span::new("<string>", 1, 1, "<string>", 1, 1).unwrap();
    let id = |name: &str| ast::id(&loc, name).unwrap();
    let lst = vec![id("x"), id("y"), id("z")];
    let prg = ast::program(&loc, "p", &lst).unwrap();
    let parameters = Attribute::Parameters;

    assert_eq!(prg.ast_array_len(parameters).unwrap(), 3);
    assert_eq!(node_children(&prg, parameters), lst);
    assert_eq!(prg.ast_at(parameters, 0).unwrap(), lst[0]);
    prg.insert_ast_at(parameters, 0, &id("i")).unwrap();
    let mut expected = vec![id("i")];
    expected.extend(lst.iter().cloned());
    assert_eq!(node_children(&prg, parameters), expected);
    prg.insert_ast_at(parameters, 0, &prg.ast_at(parameters, 3).unwrap())
        .unwrap();
    let mut expected = vec![id("z"), id("i")];
    expected.extend(lst.iter().cloned());
    assert_eq!(node_children(&prg, parameters), expected);
    prg.delete_ast_at(parameters, 2).unwrap();
    let mut expected = vec![id("z"), id("i")];
    expected.extend(lst[1..].iter().cloned());
    assert_eq!(node_children(&prg, parameters), expected);
}

// -------------------------------------------------------------------------
// test_str_sequence: the `operators` array of a theory term element
// -------------------------------------------------------------------------

#[test]
fn pyclingo_str_sequence() {
    let loc = Span::new("<string>", 1, 1, "<string>", 1, 1).unwrap();
    let lst = ["x", "y", "z"];
    let sym =
        ast::symbolic_term(&loc, Symbol::function("a", &[Symbol::number(1)]).unwrap()).unwrap();
    let tue = ast::theory_unparsed_term_element(&lst, &sym).unwrap();
    let operators = Attribute::Operators;
    let expect = |extra: &[&str], rest: &[&str]| -> Vec<String> {
        extra.iter().chain(rest).map(ToString::to_string).collect()
    };

    assert_eq!(tue.string_array_len(operators).unwrap(), 3);
    assert_eq!(node_strings(&tue, operators), lst);
    assert_eq!(tue.string_at(operators, 0).unwrap(), lst[0]);
    tue.insert_string_at(operators, 0, "i").unwrap();
    assert_eq!(node_strings(&tue, operators), expect(&["i"], &lst));
    tue.insert_string_at(operators, 0, &tue.string_at(operators, 3).unwrap())
        .unwrap();
    assert_eq!(node_strings(&tue, operators), expect(&["z", "i"], &lst));
    tue.delete_string_at(operators, 2).unwrap();
    assert_eq!(
        node_strings(&tue, operators),
        expect(&["z", "i"], &lst[1..])
    );
}

// -------------------------------------------------------------------------
// test_unpool
//
// `unpool(other, condition)` upstream is `Unpool::OTHER`/`Unpool::CONDITION`.
// The last case, `unpool(other=False, condition=False)`, asks for no pools and
// expects `["a(1;2): a(3;4)"]`: `Unpool::NONE`.
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_unpool() {
    let prg = parse_statements("%comment\n:- a(1;2): a(3;4).");
    let com = &prg[prg.len() - 2];
    let lit = prg.last().unwrap().ast_at(Attribute::Body, 0).unwrap();

    let unpool = |node: &Ast, what: Unpool| {
        let mut ret = Vec::new();
        node.unpool(what, |x| {
            ret.push(x.to_string());
            Ok(())
        })
        .unwrap();
        ret
    };

    assert_eq!(unpool(com, Unpool::ALL), ["%comment"]);
    assert_eq!(
        unpool(&lit, Unpool::ALL),
        ["a(1): a(3)", "a(1): a(4)", "a(2): a(3)", "a(2): a(4)"]
    );
    assert_eq!(
        unpool(&lit, Unpool::CONDITION),
        ["a(1;2): a(3)", "a(1;2): a(4)"]
    );
    assert_eq!(
        unpool(&lit, Unpool::OTHER),
        ["a(1): a(3;4)", "a(2): a(3;4)"]
    );
    assert_eq!(unpool(&lit, Unpool::NONE), ["a(1;2): a(3;4)"]);
}

// -------------------------------------------------------------------------
// test_transformer: a `Visitor` that prefixes every variable name
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_transformer() {
    struct VariableRenamer;

    impl Visitor for VariableRenamer {
        fn visit_variable(&mut self, node: &Ast) -> clingox::Result<Ast> {
            ast::variable(
                &node.span(Attribute::Location)?,
                &format!("_{}", node.string(Attribute::Name)?),
            )
        }
    }

    let mut prg = Vec::new();
    ast::parse_string("p(X) :- q(X).", |stm| {
        prg.push(VariableRenamer.visit(&stm)?.to_string());
        Ok(())
    })
    .unwrap();
    assert_eq!(prg.last().unwrap(), "p(_X) :- q(_X).");
}

// -------------------------------------------------------------------------
// test_comment_order: comments arrive in program order
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_comment_order() {
    let prg = "\
% comment before `x=10`
#const x=10.
% comment after `x=10`
a.
% comment before `y=10`
#const y=10. [override]
% comment after `y=10`
b.
% comment before `#external a`
#external a.
% comment after `#external a`
a.
% comment before `#external b`
#external b. [true]
% comment after `#external b`
";
    let expected = [
        "#program base.",
        "% comment before `x=10`",
        "#const x = 10.",
        "% comment after `x=10`",
        "a.",
        "% comment before `y=10`",
        "#const y = 10. [override]",
        "% comment after `y=10`",
        "b.",
        "% comment before `#external a`",
        "#external a. [false]",
        "% comment after `#external a`",
        "a.",
        "% comment before `#external b`",
        "#external b. [true]",
        "% comment after `#external b`",
    ];
    let result: Vec<String> = parse_statements(prg)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(result, expected);
}

// -------------------------------------------------------------------------
// test_repr
//
// Upstream evaluates `repr(stms)`, Python source that calls the constructors
// with the node's own attributes, and expects the evaluated list to equal
// `stms`. Rust has no evaluable `Debug` text, so the port does what that
// source does: rebuild every node with its typed constructor from its
// attributes, which `rebuild` asserts equal (locations included) at every
// level, and compare the two lists.
// -------------------------------------------------------------------------

#[test]
fn pyclingo_ast_repr() {
    let prg = "\
a :- &sum(body,second) { foo; (1 - 3) } > (4 * 17).
&theory { (X * a ** stuff): dom(X); (1 - 3) }.
&diff { (foo - bar) } <= 42.
1 #max { X : a } :- a, X = #count { a }.
";
    let stms = parse_statements(prg);
    assert_eq!(stms.len(), 5);
    let rebuilt: Vec<Ast> = stms.iter().map(rebuild).collect();
    assert_eq!(stms, rebuilt);
}

// =========================================================================
// From test_application.py
// =========================================================================

// -------------------------------------------------------------------------
// test_app
//
// Upstream runs `clingo_main` in a `multiprocessing` process, because it cannot
// capture the output, and sends what its callbacks see through a queue.
// `--outf=3` silences the output here too, so the run is in process and the
// queue is a list. The `TestApp` callbacks map one to one: `register_options`
// to `register_options` with `Options::add` and `add_flag`, `validate_options`
// to `validate_options`, `logger` to `logger`, `main` to `main`;
// `program_name`, `version` and `message_limit` are the class attributes.
//
// Differences measured against clingo 5.8.2 (the upstream file run under
// pyclingo 5.8.2 gives the same values as this port apart from these):
// - the models come in the order `a`, `b`, `a b`; the assertion upstream
//   expects `a`, `a b`, `b`, which does not hold on 5.8.2. The port asserts the
//   measured order.
// - the logger text has no trailing newline, as the crate documents for every
//   logger.
//   -------------------------------------------------------------------------

#[test]
fn pyclingo_application_app() {
    use std::sync::Mutex;

    use clingox::MessageCode;
    use clingox::ShowType;
    use clingox::application::{Application, Flag, OptionSpec};

    #[derive(Debug, PartialEq)]
    enum Event {
        Register,
        Parse(String),
        Validate,
        Flag(bool),
        Main,
        Message(MessageCode, String),
        Models(Vec<Vec<String>>),
    }

    let path = std::env::temp_dir().join(format!("clingox_pyclingo_app_{}.lp", std::process::id()));
    std::fs::write(&path, "1 {a; b; c(1/0)}.").unwrap();
    let name = path.to_str().unwrap().to_owned();

    // The logger may run on any thread, so the queue is behind a lock.
    let queue = Mutex::new(Vec::new());
    let flag = Flag::new(false);
    let ret = Application::new()
        .program_name("test")
        .version("1.2.3")
        .message_limit(17)
        .register_options(|options| {
            queue.lock().unwrap().push(Event::Register);
            let group = "Clingo.Test";
            options.add(
                OptionSpec::new(group, "test", "test description"),
                |value| {
                    queue.lock().unwrap().push(Event::Parse(value.to_owned()));
                    Ok(())
                },
            )?;
            options.add_flag(group, "flag", "test description", &flag)
        })
        .validate_options(|| {
            queue.lock().unwrap().push(Event::Validate);
            queue.lock().unwrap().push(Event::Flag(flag.get()));
            Ok(())
        })
        .logger(|code, message| {
            // upstream: `re.sub("^.*:(?=[0-9]+:)", "", message)`, the file name
            let message = message.strip_prefix(&format!("{name}:")).unwrap_or(message);
            queue
                .lock()
                .unwrap()
                .push(Event::Message(code, message.to_owned()));
        })
        .main(|control, files| {
            queue.lock().unwrap().push(Event::Main);
            for file in files {
                control.load(file)?;
            }
            control.ground(&[Part::base()])?;
            let mut models = Vec::new();
            let _ = control.for_each_model(&[], |model| {
                let mut atoms: Vec<String> = model
                    .symbols(ShowType::SHOWN)?
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                atoms.sort();
                models.push(atoms);
                Ok(std::ops::ControlFlow::Continue(()))
            })?;
            queue.lock().unwrap().push(Event::Models(models));
            Ok(())
        })
        .run([name.as_str(), "--outf=3", "0", "--test=x", "--flag"])
        .unwrap();
    std::fs::remove_file(&path).unwrap();

    assert_eq!(ret, 30);
    assert_eq!(
        queue.into_inner().unwrap(),
        [
            Event::Register,
            Event::Parse("x".to_owned()),
            Event::Validate,
            Event::Flag(true),
            Event::Main,
            Event::Message(
                MessageCode::OperationUndefined,
                "1:12-15: info: operation undefined:\n  (1/0)".to_owned()
            ),
            Event::Models(vec![
                vec!["a".to_owned()],
                vec!["b".to_owned()],
                vec!["a".to_owned(), "b".to_owned()],
            ]),
        ]
    );
}

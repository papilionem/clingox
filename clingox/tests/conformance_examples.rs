//! Ported from potassco/clingo v5.8.2
//! Source: `examples/c/`, programs portable to the safe layer
//! Differences: Rust idioms replace C patterns (iterators instead of loops,
//! `SolveHandle` instead of manual resume/model/get, Configuration paths
//! instead of explicit array subscript).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]
#![allow(
    clippy::float_cmp,
    reason = "the values compared exactly are the ones clingo reports as counts"
)]

mod conformance;

#[path = "common/child.rs"]
mod child;

use clingox::prelude::*;

// ---------------------------------------------------------------------------
// version.c -- prints clingo version
// ---------------------------------------------------------------------------

#[test]
fn example_version() {
    let mut ctl = Control::new().unwrap();
    // Check the version is 5.8.2 by trying to use it.
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
}

// ---------------------------------------------------------------------------
// control.c -- add "a :- not b. b :- not a.", solve, print model
// ---------------------------------------------------------------------------

#[test]
fn example_control() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("a :- not b. b :- not a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert!(result.is_exhausted());
    // Two models: {a} and {b}
    assert_eq!(models.len(), 2);
    let model_symbols: std::collections::BTreeSet<String> = models
        .iter()
        .flat_map(|m| {
            let mut syms: Vec<String> = m.symbols().iter().map(ToString::to_string).collect();
            syms.sort();
            syms
        })
        .collect();
    assert_eq!(model_symbols.len(), 2);
    assert!(model_symbols.contains("a"));
    assert!(model_symbols.contains("b"));
}

// ---------------------------------------------------------------------------
// symbol.c -- create Number(42), Id("x"), Function("x", ...); hash; compare
// ---------------------------------------------------------------------------

#[test]
fn example_symbol() {
    use clingox::Symbol;
    // Number(42)
    let n42 = Symbol::number(42);
    assert_eq!(n42.to_string(), "42");
    // Id("x", true) -- a constant, which is a function without args
    let x_const = Symbol::function("x", &[]).unwrap();
    assert_eq!(x_const.to_string(), "x");
    // Function("x", [42, Id("x")], true) -- note: the C example passes
    // symbols[0..2]
    let x_fun = Symbol::function("x", &[n42, x_const]).unwrap();
    assert_eq!(x_fun.to_string(), "x(42,x)");

    // Hash values: they should be stable within the process.
    assert_ne!(n42, x_const);
    assert_ne!(n42, x_fun);
    assert_ne!(x_const, x_fun);

    // Arguments
    let args = x_fun.arguments().unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], n42);
    assert_eq!(args[1], x_const);

    // Compare: 42 != x, and 42 < x in clingo's order
    assert_ne!(n42, x_const);
    assert!(n42 < x_const || x_const < n42);
}

// ---------------------------------------------------------------------------
// configuration.c -- set solve.models=0, solver.heuristic=berkmin, same program
// ---------------------------------------------------------------------------

#[test]
fn example_configuration() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    // Configure: set solver heuristic
    {
        let mut conf = ctl.configuration();
        // The docs say clingo resolves array names through first element:
        // "solver.heuristic" is equivalent to "solver.0.heuristic".
        conf.set("solver.heuristic", "berkmin,100").unwrap();
    }
    ctl.add_base("a :- not b. b :- not a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert!(result.is_exhausted());
    assert_eq!(models.len(), 2);
}

// ---------------------------------------------------------------------------
// symbolic-atoms.c -- iterate symbolic atoms, print symbol + flags
// ---------------------------------------------------------------------------

#[test]
fn example_symbolic_atoms() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a. {b}. #external c.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atoms = ctl.symbolic_atoms().unwrap();

    let mut facts = Vec::new();
    let mut externals = Vec::new();
    let mut choices = Vec::new();

    for atom in &atoms {
        let atom = atom.unwrap();
        if atom.is_fact() {
            facts.push(atom.symbol().to_string());
        }
        if atom.is_external() {
            externals.push(atom.symbol().to_string());
        }
        if !atom.is_fact() && !atom.is_external() {
            choices.push(atom.symbol().to_string());
        }
    }

    assert_eq!(facts, vec!["a"]);
    assert_eq!(externals, vec!["c"]);
    assert_eq!(choices, vec!["b"]);
}

// ---------------------------------------------------------------------------
// solve-async.c -- async solve with model loop, pi approximation omitted
// ---------------------------------------------------------------------------

#[test]
fn example_solve_async() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("#const n = 17. 1 { p(X); q(X) } 1 :- X = 1..n. :- not n+1 { p(1..n); q(1..n) }.")
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let result = ctl.solve(&[]).unwrap();
    assert!(
        result.is_sat() || result.is_unsat(),
        "the program should either be SAT or UNSAT: {result:?}"
    );
    // The C example does not check satisfiability; it runs an async solve with
    // a pi approximation loop in parallel.
}

// ---------------------------------------------------------------------------
// theory-atoms.c -- theory atoms container, atom b/1's guard, solve with its
// literal
//
// Oracle: Python module 5.8.2, checked directly (2026-09-27): the program
// grounds two theory atoms, `&a{(1+2)}` (no guard) and `&b(3){}=17` (a guard),
// in that order from `clingo_control_theory_atoms`; `&b(3)`'s term is a
// function named `b` with one argument, the number `3`; its literal is `1`. The
// C example's own `printf`s are replaced by assertions on the same values.
// ---------------------------------------------------------------------------

#[test]
fn example_theory_atoms() {
    use clingox::{TheoryTerm, TheoryTermKind};

    let mut ctl = Control::new().unwrap();
    ctl.add_base(
        "#theory t {\
           term   { + : 1, binary, left };\
           &a/0 : term, any;\
           &b/1 : term, {=}, term, any\
         }.\
         x :- &a { 1+2 }.\
         y :- &b(3) { } = 17.",
    )
    .unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let atoms = ctl.theory_atoms().unwrap();
    assert_eq!(atoms.len().unwrap(), 2);

    let mut b_literal = None;
    for item in atoms.iter() {
        let atom = item.unwrap();
        if let TheoryTerm::Compound {
            kind: TheoryTermKind::Function,
            name: Some("b"),
            arguments,
        } = &atom.term().unwrap()
        {
            assert_eq!(*arguments, vec![TheoryTerm::Number(3)]);
            assert!(atom.guard().unwrap().is_some(), "&b/1 has a guard");
            b_literal = Some(atom.literal().unwrap().expect("a head atom has a literal"));
        }
    }
    let b_literal = b_literal.expect("&b(3) is one of the grounded theory atoms");

    // Solve with the theory atom's own literal as the sole assumption, as the
    // C example does after finding it.
    let result = ctl.solve(&[b_literal.into()]).unwrap();
    assert!(result.is_sat());
}

// ---------------------------------------------------------------------------
// backend.c -- add an aux atom `d` with `d :- a, b.` and `:- not d, c.`
// through the backend, then enumerate every model
//
// Oracle: the header's own doc comment quotes the expected output (5
// models: `a b`, `a b c`, ``, `a`, `b`); confirmed directly against the
// Python module 5.8.2, 2026-09-27, by deriving `d = a && b` and filtering
// out any combination where `c` is true and `d` is false, over all 8
// combinations of `{a;b;c}.`.
//
// The C example fills one array with `a`/`b`/`c`'s *literals* (from
// `clingo_symbolic_atoms_literal`) and `d`'s *atom id* (from
// `clingo_backend_add_atom`'s out-parameter), then passes entries from that
// same array as both head atoms and body literals: this happens to work
// because a plain grounded atom's id and its positive literal carry the
// same numeric value in this simple case, but it is exactly the kind of
// mixup the separate `Atom`/`ProgramLiteral` types exist to rule
// out at compile time. The port keeps the two properly
// separate instead of reproducing the coincidence.
// ---------------------------------------------------------------------------

#[test]
fn example_backend() {
    use clingox::backend::Head;

    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add_base("{a; b; c}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();

    let a = ctl
        .symbolic_atoms()
        .unwrap()
        .find(sym("a"))
        .unwrap()
        .expect("a is grounded")
        .literal();
    let b = ctl
        .symbolic_atoms()
        .unwrap()
        .find(sym("b"))
        .unwrap()
        .expect("b is grounded")
        .literal();
    let c = ctl
        .symbolic_atoms()
        .unwrap()
        .find(sym("c"))
        .unwrap()
        .expect("c is grounded")
        .literal();

    ctl.with_backend(|backend| {
        let d = backend.add_atom(None)?; // an additional atom, called `d`
        backend.add_rule(Head::Normal(&[d]), &[a, b])?; // d :- a, b.
        backend.add_rule(Head::Constraint, &[d.neg(), c]) // :- not d, c.
    })
    .unwrap();

    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert!(result.is_exhausted());
    assert_eq!(models.len(), 5, "{models:?}");
    clingox::testing::assert_models!(models, ["a b", "a b c", "", "a", "b"]);
}

fn sym(text: &str) -> clingox::Symbol {
    text.parse().expect("the term parses")
}

// ---------------------------------------------------------------------------
// model.c (Model::kind, Model::number) -- one model of a choice, printed
// four ways: shown, atoms, terms, complement-of-atoms.
//
// No arguments are passed (`argv + 1, argc - 1` with none given), so the
// default `--models=1` applies; clasp's default heuristic picks `b` true,
// `a` false for this program, checked directly against clingo 5.8.2.
// ---------------------------------------------------------------------------

#[test]
fn example_model() {
    use clingox::ShowType;

    let mut ctl = Control::new().unwrap();
    ctl.add_base("1 {a; b} 1. #show c : b. #show a/0.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle
        .next_model()
        .unwrap()
        .expect("the program has a model");

    assert_eq!(model.kind().unwrap(), clingox::ModelKind::StableModel);
    assert_eq!(model.number(), 1);

    let texts = |show: ShowType| -> Vec<String> {
        let mut v: Vec<String> = model
            .symbols(show)
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        v.sort();
        v
    };
    assert_eq!(texts(ShowType::SHOWN), vec!["c".to_owned()]);
    assert_eq!(texts(ShowType::ATOMS), vec!["b".to_owned()]);
    assert_eq!(texts(ShowType::TERMS), vec!["c".to_owned()]);
    assert_eq!(
        texts(ShowType::ATOMS | ShowType::COMPLEMENT),
        vec!["a".to_owned()]
    );
}

// ---------------------------------------------------------------------------
// statistics.c (statistics writing) -- clasp's own statistics tree is
// read back the same way `libclingo_statistics`/`libclingo_set_statistics`
// already check it, plus a user-written entry the example itself adds.
// ---------------------------------------------------------------------------

#[test]
fn example_statistics() {
    use std::ops::ControlFlow;

    use clingox::{MutableStatistics, SolveEventHandler, SolveOptions, StatKind};

    struct WriteUserAccu;
    impl SolveEventHandler for WriteUserAccu {
        fn on_statistics(
            &mut self,
            _step: &mut MutableStatistics<'_>,
            accumulated: &mut MutableStatistics<'_>,
        ) -> clingox::Result<ControlFlow<()>> {
            accumulated.add_map_key("", "example", StatKind::Value)?;
            accumulated.set_value("example", 42.0)?;
            Ok(ControlFlow::Continue(()))
        }
    }

    let mut ctl = Control::with_args(["--stats"]).unwrap();
    ctl.add_base("1 {a; b} 1.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let result = ctl
        .solve_with_events(SolveOptions::new(), WriteUserAccu)
        .unwrap();
    assert!(result.is_sat());

    let stats = ctl.statistics().unwrap();
    assert!(stats.value("summary.models.enumerated").unwrap() >= 1.0);
    assert_eq!(stats.value("user_accu.example").unwrap(), 42.0);
}

// ---------------------------------------------------------------------------

// ast.c -- parse a program into syntax trees, append the literal
// `enable` to the body of every rule, add the rewritten statements through the
// program builder, declare `#external enable.` and solve with the external
// false, true and false again.
//
// Checked against the C program itself: `examples/c/ast.c`, compiled and run
// against the libclingo 5.8.2 inside the installed pyclingo, prints exactly
// the transcript asserted below (it takes no arguments, so clingo's default of
// one model per solve call applies; the model with `enable` true is `b`).
// ---------------------------------------------------------------------------

#[test]
fn example_ast() {
    use std::fmt::Write as _;

    use clingox::ShowType;
    use clingox::ast::{self, AstType, Attribute, LiteralSign, Span};

    let mut ctl = Control::new().unwrap();

    // Location "<rewrite>" 0:0, as in the C program.
    let location = Span::new("<rewrite>", 0, 0, "<rewrite>", 0, 0).unwrap();
    // The atom to add: `enable`.
    let sym = Symbol::function("enable", &[]).unwrap();
    let term = ast::symbolic_term(&location, sym).unwrap();
    let atom = ast::symbolic_atom(&term).unwrap();

    ctl.with_program_builder(|builder| {
        ast::parse_string("a :- not b. b :- not a.", |stm| {
            // Pass through all statements that are not rules.
            if stm.ast_type() != AstType::Rule {
                return builder.add(&stm);
            }
            // Create the literal `enable` and append it to the rule body.
            let lit = ast::literal(&location, LiteralSign::NoSign, &atom)?;
            let size = stm.ast_array_len(Attribute::Body)?;
            stm.insert_ast_at(Attribute::Body, size, &lit)?;
            builder.add(&stm)
        })
    })
    .unwrap();

    ctl.add_base("#external enable.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let literal = ctl
        .symbolic_atoms()
        .unwrap()
        .find(sym)
        .unwrap()
        .expect("enable is an atom")
        .literal();

    let mut output = String::new();
    let solve = |ctl: &mut Control, output: &mut String| {
        let mut handle = ctl.solve_yield(&[]).unwrap();
        while let Some(model) = handle.next_model().unwrap() {
            output.push_str("Model:");
            for symbol in model.symbols(ShowType::SHOWN).unwrap() {
                write!(output, " {symbol}").unwrap();
            }
            output.push('\n');
        }
        let _ = handle.close().unwrap();
    };

    output.push_str("Solving with enable = false...\n");
    solve(&mut ctl, &mut output);
    output.push_str("Solving with enable = true...\n");
    ctl.assign_external_literal(literal, TruthValue::True)
        .unwrap();
    solve(&mut ctl, &mut output);
    output.push_str("Solving with enable = false...\n");
    ctl.assign_external_literal(literal, TruthValue::False)
        .unwrap();
    solve(&mut ctl, &mut output);

    assert_eq!(
        output,
        "Solving with enable = false...\n\
         Model:\n\
         Solving with enable = true...\n\
         Model: enable b\n\
         Solving with enable = false...\n\
         Model:\n"
    );
}

// ---------------------------------------------------------------------------
// propagator.c (register_propagator, PropagateInit, PropagateControl,
// add_clause) -- the pigeon-hole propagator: watches every `place/2` placement
// literal and forbids two pigeons sharing a hole purely through the propagator,
// with no ASP-level constraint of its own.
//
// The example's own hard-coded arguments (8 holes, 9 pigeons: one more pigeon
// than holes) make the program unsatisfiable by the pigeonhole principle.
// Checked directly against clingo 5.8.2 on the ASP-level equivalent of the same
// constraint (`:- place(P1,H), place(P2,H), P1 != P2.`): `h=8,p=9` is UNSAT,
// `h=5,p=6` is UNSAT and `h=2,p=2` has exactly the 2 models
// `{place(1,1),place(2,2)}`/`{place(1,2),place(2,1)}` (the last two also
// confirm `libclingo/tests/propagator.cc`'s own `pigeon` section, `unsat`/`sat`
// subsections, whose C++ `PigeonPropagator` is the same algorithm as this port,
// only ported once, here).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Pigeonhole {
    /// Per thread, hole index -> the literal currently occupying it.
    holes: std::sync::Mutex<Vec<Vec<Option<clingox::propagate::SolverLiteral>>>>,
    /// Solver literal -> hole index, read-only once `init` finishes.
    pigeons: std::sync::Mutex<std::collections::HashMap<clingox::propagate::SolverLiteral, u32>>,
}

impl clingox::propagate::Propagator for Pigeonhole {
    fn init(&self, init: &mut clingox::propagate::PropagateInit<'_>) -> clingox::Result<()> {
        let sig = clingox::Signature::new("place", 2)?;
        let mut pigeons = self.pigeons.lock().unwrap();
        let mut hole_count = 0u32;
        for atom in init.symbolic_atoms()?.by_signature(sig) {
            let atom = atom?;
            let args = atom.symbol().arguments().unwrap();
            let hole = u32::try_from(
                args[1]
                    .as_number()
                    .expect("place/2's second argument is a number"),
            )
            .unwrap();
            let slit = init.solver_literal(atom.literal())?;
            pigeons.insert(slit, hole);
            init.add_watch(slit)?;
            hole_count = hole_count.max(hole + 1);
        }
        let threads = usize::try_from(init.number_of_threads()).unwrap();
        *self.holes.lock().unwrap() =
            vec![vec![None; usize::try_from(hole_count).unwrap()]; threads];
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut clingox::propagate::PropagateControl<'_>,
        changes: &[clingox::propagate::SolverLiteral],
    ) -> clingox::Result<()> {
        let pigeons = self.pigeons.lock().unwrap();
        let mut holes = self.holes.lock().unwrap();
        let thread = usize::try_from(control.thread_id()).unwrap();
        for &lit in changes {
            let hole = usize::try_from(pigeons[&lit]).unwrap();
            match holes[thread][hole] {
                None => holes[thread][hole] = Some(lit),
                Some(prev) => {
                    // As the C example does: stop calling further methods
                    // on this `control` once one of them reports
                    // `Flow::Stop`, since clingox itself then refuses any
                    // further call with `ErrorKind::InvalidInput`.
                    if !control
                        .add_clause(&[-lit, -prev], clingox::propagate::ClauseType::Learnt)?
                        .is_stop()
                    {
                        let _ = control.propagate()?;
                    }
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn undo(
        &self,
        control: &clingox::propagate::PropagateControl<'_>,
        changes: &[clingox::propagate::SolverLiteral],
    ) {
        let pigeons = self.pigeons.lock().unwrap();
        let mut holes = self.holes.lock().unwrap();
        let thread = usize::try_from(control.thread_id()).unwrap();
        for &lit in changes {
            let hole = usize::try_from(pigeons[&lit]).unwrap();
            if holes[thread][hole] == Some(lit) {
                holes[thread][hole] = None;
            }
        }
    }
}

fn pigeonhole(holes: i32, pigeons: i32) -> clingox::SolveResult {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add(
        "pigeon",
        &["h", "p"],
        "1 { place(P,H) : H = 1..h } 1 :- P = 1..p.",
    )
    .unwrap();
    ctl.ground(&[Part::new("pigeon", &[Symbol::number(holes), Symbol::number(pigeons)]).unwrap()])
        .unwrap();
    ctl.register_propagator(Pigeonhole::default()).unwrap();
    ctl.solve(&[]).unwrap()
}

#[test]
fn example_propagator() {
    assert!(pigeonhole(8, 9).is_unsat(), "9 pigeons, 8 holes");
}

// libclingo/tests/propagator.cc's own "pigeon" TEST_CASE section (`unsat`/
// `sat` subsections): the identical `PigeonPropagator` algorithm on smaller
// arguments, ported here rather than duplicated in `conformance_libclingo.
// rs`, since it is the same struct and the same propagator example --
// ported as the backend.c/theory-atoms.c examples were
// (`conformance_examples.rs`), and the C++ section tests nothing about
// `PigeonPropagator` that differs from it beyond the concrete numbers.

#[test]
fn libclingo_propagator_pigeon_unsat() {
    assert!(pigeonhole(5, 6).is_unsat(), "6 pigeons, 5 holes");
}

#[test]
fn libclingo_propagator_pigeon_sat() {
    let mut ctl = Control::with_args(["--models=0"]).unwrap();
    ctl.add(
        "pigeon",
        &["h", "p"],
        "1 { place(P,H) : H = 1..h } 1 :- P = 1..p.",
    )
    .unwrap();
    ctl.ground(&[Part::new("pigeon", &[Symbol::number(2), Symbol::number(2)]).unwrap()])
        .unwrap();
    ctl.register_propagator(Pigeonhole::default()).unwrap();
    let (result, models) = ctl.solve_all().unwrap();
    assert!(result.is_sat());
    assert_eq!(models.len(), 2, "{models:?}");
    clingox::testing::assert_models!(models, ["place(1,1) place(2,2)", "place(1,2) place(2,1)"]);
}

// ---------------------------------------------------------------------------
// application.c -- a clingo application with its own option and main
//
// The C program registers the option `--program=<prog>` (group `Example`),
// loads the files (or standard input when there are none), grounds the part
// the option names (`base` by default), and runs a yield-mode solve loop
// that drains the models. clingo prints the version banner, the models and
// the summary itself. The program writes with C stdio, which libtest cannot
// capture, so every case runs in a child process (`common/child.rs`).
//
// The expected transcripts are those of the C program itself, compiled with
// gcc against the libclingo 5.8.2 inside pyclingo (linked as for `ast.c`)
// and run with the same arguments.
// has them. Lines that carry a time are dropped, the temporary file name is
// replaced by `FILE`, and blank lines are ignored (libtest's own output is
// mixed into the child's standard output).
// ---------------------------------------------------------------------------

const APPLICATION_ENTRY: &str = "example_application_child";
const APPLICATION_PROGRAM: &str = "1 {a; b; c(1/0)}.\n#program foo.\nf. {g}.\n";

/// `application.c`, function by function.
fn example_application_run(arguments: &[String]) -> clingox::Result<i32> {
    use clingox::application::{Application, OptionSpec};
    use std::cell::RefCell;

    // `options_t`
    let program: RefCell<Option<String>> = RefCell::new(None);
    Application::new()
        // `name` and `version`
        .program_name("example")
        .version("1.0.0")
        // `register_options` and `parse_option`
        .register_options(|options| {
            options.add(
                OptionSpec::new(
                    "Example",
                    "program",
                    "Override the default program part to ground",
                )
                .argument("<prog>"),
                |value| {
                    *program.borrow_mut() = Some(value.to_owned());
                    Ok(())
                },
            )
        })
        // `main_loop`
        .main(|ctl, files| {
            for file in files {
                ctl.load(file)?;
            }
            // if no files are given read from stdin
            if files.is_empty() {
                ctl.load("-")?;
            }
            let name = program
                .borrow()
                .clone()
                .unwrap_or_else(|| "base".to_owned());
            ctl.ground(&[Part::new(&name, &[])?])?;
            // `solve`: loop over all models, then get the result
            let mut handle = ctl.solve_yield(&[])?;
            while handle.next_model()?.is_some() {}
            let _ = handle.get()?;
            let _ = handle.close()?;
            Ok(())
        })
        .run(arguments)
}

#[test]
#[allow(
    clippy::print_stdout,
    reason = "the child reports its result through standard output"
)]
fn example_application_child() {
    let Some(case) = child::child_case() else {
        return;
    };
    let file = child::fixture("example_application.lp", APPLICATION_PROGRAM);
    let path = child::arg(&file);
    // libtest has printed `test <name> ... ` without a newline
    println!();
    println!("CHILD-FILE {path}");
    let arguments: Vec<String> = match case.as_str() {
        "all" => vec![path, "0".into()],
        "program" => vec!["--program=foo".into(), path, "0".into()],
        "first" => vec!["--program=foo".into(), path],
        "unknown_part" => vec!["--program=nosuch".into(), path, "0".into()],
        "quiet" => vec![path, "--outf=3".into(), "0".into()],
        "stdin" => vec![],
        "stdin_facts" => vec!["0".into()],
        "dash_slash" => {
            use clingox::{Control, ErrorKind};
            // "-/" names a file, not standard input: the load fails with an
            // ordinary error and the control stays usable.
            let mut ctl = Control::new().unwrap();
            let kind = ctl.load("-/").unwrap_err().kind();
            assert_eq!(kind, ErrorKind::Runtime);
            ctl.add_base("x.").unwrap();
            ctl.ground(&[Part::base()]).unwrap();
            assert!(ctl.solve(&[]).unwrap().is_sat());
            println!("CHILD-DASH-SLASH-OK");
            return;
        }
        "missing" => vec!["nofile.lp".into()],
        "version" => vec!["--version".into()],
        "help" => vec!["--help=1".into()],
        "no_value" => vec!["--program".into()],
        other => panic!("unknown child case {other}"),
    };
    match example_application_run(&arguments) {
        Ok(code) => println!("CHILD-RC {code}"),
        Err(error) => println!("CHILD-ERR {:?} {error}", error.kind()),
    }
}

/// What the child did, reduced to the lines clingo printed and the exit code
/// `run` returned (`None` when it returned an error, whose text is the last
/// field).
type ApplicationRun = (Vec<String>, Vec<String>, Option<i32>, String);

fn example_application(case: &str) -> Option<ApplicationRun> {
    example_application_stdin(case, "")
}

fn example_application_stdin(case: &str, input: &str) -> Option<ApplicationRun> {
    let out = child::run_child_with_stdin(APPLICATION_ENTRY, case, input)?;
    assert_eq!(
        out.code,
        Some(0),
        "{case}: the child test failed: {}",
        out.stdout
    );
    let path = out
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("CHILD-FILE "))
        .unwrap()
        .to_owned();
    let code = out
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("CHILD-RC "))
        .map(|c| c.parse().unwrap());
    let error = out
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("CHILD-ERR "))
        .unwrap_or_default()
        .to_owned();
    let clean = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|l| {
                !(l.is_empty()
                    || l.starts_with("CHILD-")
                    || l.starts_with("running ")
                    || l.starts_with("test ")
                    || *l == "ok"
                    || l.starts_with("Time ")
                    || l.starts_with("CPU Time"))
            })
            .map(|l| {
                // clingo shortens a long file name to `...<tail>`
                let l = match l.strip_prefix("Reading from ") {
                    Some(name)
                        if name == path
                            || name.strip_prefix("...").is_some_and(|t| path.ends_with(t)) =>
                    {
                        "Reading from FILE".to_owned()
                    }
                    _ => l.replace(&path, "FILE"),
                };
                match l.find(" (Time: ") {
                    Some(at) => l[..at].to_owned(),
                    None => l,
                }
            })
            .collect()
    };
    Some((clean(&out.stdout), clean(&out.stderr), code, error))
}

#[test]
fn example_application_ground_base_by_default() {
    let Some((stdout, stderr, code, _)) = example_application("all") else {
        return;
    };
    // the models in the order clingo enumerates them; atoms in print order
    assert_eq!(
        stdout,
        [
            "example version 1.0.0",
            "Reading from FILE",
            "Solving...",
            "Answer: 1",
            "a",
            "Answer: 2",
            "b",
            "Answer: 3",
            "b a",
            "SATISFIABLE",
            "Models       : 3",
            "Calls        : 1",
        ]
    );
    assert_eq!(
        stderr,
        ["FILE:1:12-15: info: operation undefined:", "  (1/0)"]
    );
    assert_eq!(code, Some(30));
}

#[test]
fn example_application_program_option_grounds_another_part() {
    let Some((stdout, _, code, _)) = example_application("program") else {
        return;
    };
    assert_eq!(
        stdout,
        [
            "example version 1.0.0",
            "Reading from FILE",
            "Solving...",
            "Answer: 1",
            "f",
            "Answer: 2",
            "f g",
            "SATISFIABLE",
            "Models       : 2",
            "Calls        : 1",
        ]
    );
    assert_eq!(code, Some(30));
}

#[test]
fn example_application_without_a_model_count_stops_at_the_first() {
    let Some((stdout, _, code, _)) = example_application("first") else {
        return;
    };
    assert_eq!(
        stdout,
        [
            "example version 1.0.0",
            "Reading from FILE",
            "Solving...",
            "Answer: 1",
            "f",
            "SATISFIABLE",
            "Models       : 1+",
            "Calls        : 1",
        ]
    );
    assert_eq!(code, Some(10));
}

#[test]
fn example_application_unknown_part_grounds_nothing() {
    // `base` is not grounded, so the program is empty and has one model
    let Some((stdout, _, code, _)) = example_application("unknown_part") else {
        return;
    };
    assert_eq!(
        stdout,
        [
            "example version 1.0.0",
            "Reading from FILE",
            "Solving...",
            "Answer: 1",
            "SATISFIABLE",
            "Models       : 1",
            "Calls        : 1",
        ]
    );
    assert_eq!(code, Some(30));
}

#[test]
fn example_application_quiet_output() {
    let Some((stdout, stderr, code, _)) = example_application("quiet") else {
        return;
    };
    assert_eq!(stdout, Vec::<String>::new());
    assert_eq!(
        stderr,
        ["FILE:1:12-15: info: operation undefined:", "  (1/0)"]
    );
    assert_eq!(code, Some(30));
}

#[test]
fn example_application_reads_standard_input_without_files() {
    // The child's standard input is closed, so the program is empty. The C
    // program's `clingo_control_load(ctl, "-")` reads standard input, and so
    // does `Control::load("-")`.
    let Some((stdout, _, code, error)) = example_application("stdin") else {
        return;
    };
    assert_eq!(
        stdout,
        [
            "example version 1.0.0",
            "Reading from stdin",
            "Solving...",
            "Answer: 1",
            "SATISFIABLE",
            "Models       : 1+",
            "Calls        : 1",
        ],
        "{error}"
    );
    assert_eq!(code, Some(10), "{error}");
}

#[test]
fn example_application_reads_a_program_from_standard_input() {
    let Some((stdout, _, code, error)) = example_application_stdin("stdin_facts", "a. b.\n") else {
        return;
    };
    assert_eq!(
        stdout,
        [
            "example version 1.0.0",
            "Reading from stdin",
            "Solving...",
            "Answer: 1",
            "a b",
            "SATISFIABLE",
            "Models       : 1",
            "Calls        : 1",
        ],
        "{error}"
    );
    assert_eq!(code, Some(30), "{error}");
}

#[test]
fn control_load_dash_slash_is_a_path_not_standard_input() {
    // Standard input carries facts; if "-/" were read as standard input the
    // load would succeed and the child would fail its assertion.
    let Some(out) = child::run_child_with_stdin(APPLICATION_ENTRY, "dash_slash", "a.\n") else {
        return;
    };
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("CHILD-DASH-SLASH-OK"), "{}", out.stdout);
}

#[test]
fn example_application_version_and_help_list_the_option() {
    let Some((stdout, _, code, _)) = example_application("version") else {
        return;
    };
    assert_eq!(stdout[0], "example version 1.0.0");
    assert_eq!(code, Some(0));
    let Some((stdout, _, code, _)) = example_application("help") else {
        return;
    };
    assert_eq!(stdout[0], "example version 1.0.0");
    assert_eq!(stdout[1], "usage: example [number] [options] [files]");
    let at = stdout.iter().position(|l| l == "Example:").unwrap();
    let words: Vec<&str> = stdout[at + 1].split_whitespace().collect();
    assert_eq!(
        words,
        [
            "--program=<prog>",
            ":",
            "Override",
            "the",
            "default",
            "program",
            "part",
            "to",
            "ground"
        ]
    );
    assert_eq!(code, Some(0));
}

#[test]
fn example_application_missing_file_and_missing_value() {
    let Some((stdout, stderr, code, error)) = example_application("missing") else {
        return;
    };
    assert_eq!(
        stdout[..2],
        ["example version 1.0.0", "Reading from nofile.lp"]
    );
    assert!(stdout.contains(&"UNKNOWN".to_owned()), "{stdout:?}");
    // The C program's own `clingo_control_load` fails inside clingo, which
    // prints `<cmd>: error: file could not be opened:` and, after `main_loop`
    // returned false, `*** ERROR: (example): parsing failed` (exit 65).
    // `Control::load` reports the open failure itself, so the error reaches
    // `main`'s return value and clingo prints it as the error line; `run`
    // returns it instead of the exit code.
    assert!(
        stderr.iter().any(|l| l.contains("could not be opened")),
        "{stderr:?}"
    );
    assert_eq!(code, None, "run returns the error: {error}");
    let Some((stdout, stderr, code, _)) = example_application("no_value") else {
        return;
    };
    assert_eq!(stdout, Vec::<String>::new());
    assert_eq!(
        stderr,
        [
            "*** ERROR: (example): SyntaxError: 'program' requires a value!",
            "*** Info : (example): Try '--help' for usage information",
        ]
    );
    assert_eq!(code, Some(1));
}

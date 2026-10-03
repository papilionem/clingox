//! Benchmarks of clingox's own layer (`docs/dev/BENCHMARKS.md`).
//!
//! Each workload is chosen so that the wrapper, not clingo's search, is what
//! the number is about, and most of them come with a baseline that does the
//! same clingo work without the wrapper feature under test (a solve without
//! a propagator, a grounding without a callback), so the difference is the
//! overhead. A few end-to-end solves at the end are sanity references only.
//!
//! Run them with `cargo bench -p clingox --bench layer`; a name filter after
//! `--` selects groups. They are not part of `cargo test`.

#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    clippy::items_after_statements,
    reason = "a benchmark aborts on the first unexpected error and reports its sizes; \
              each group reads best in one piece, with its helpers next to their use"
)]

use std::fmt::Write as _;
use std::hint::black_box;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};

use clingox::application::Application;
use clingox::ast::{self, Ast, Attribute, LiteralSign, Span, Visitor, walk};
use clingox::backend::Atom;
use clingox::observer::GroundProgramObserver;
use clingox::propagate::{PropagateControl, PropagateInit, Propagator, SolverLiteral};
use clingox::{
    ConfigEntry, ConfigKind, Control, FromSymbol, FunctionCall, Part, ProgramLiteral, ShowType,
    StatKind, StatsEntry, StatsTree, Symbol, ToSymbol,
};

const N_SYMBOLS: usize = 1000;

#[derive(Clone, ToSymbol, FromSymbol)]
struct Edge {
    from: i32,
    to: i32,
    #[clingo(string)]
    label: String,
}

#[derive(Clone, ToSymbol, FromSymbol)]
struct P(i32);

fn small<'a>(
    c: &'a mut Criterion,
    name: &str,
) -> criterion::BenchmarkGroup<'a, criterion::measurement::WallTime> {
    let mut group = c.benchmark_group(name);
    group.sample_size(20);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));
    group
}

// ---------------------------------------------------------------------------
// Symbols

fn nested(i: i32) -> Symbol {
    let inner = Symbol::function("f", &[Symbol::number(i), Symbol::string("s").unwrap()]).unwrap();
    Symbol::function(
        "p",
        &[
            Symbol::number(i),
            inner,
            Symbol::tuple(&[Symbol::number(1)]).unwrap(),
        ],
    )
    .unwrap()
}

fn symbols(c: &mut Criterion) {
    let mut g = small(c, "symbol");
    g.throughput(Throughput::Elements(N_SYMBOLS as u64));
    g.bench_function("create_number", |b| {
        b.iter(|| {
            for i in 0..N_SYMBOLS as i32 {
                black_box(Symbol::number(i));
            }
        });
    });
    g.bench_function("create_function", |b| {
        b.iter(|| {
            for i in 0..N_SYMBOLS as i32 {
                black_box(Symbol::function("p", &[Symbol::number(i), Symbol::number(1)]).unwrap());
            }
        });
    });
    g.bench_function("create_string", |b| {
        b.iter(|| {
            for _ in 0..N_SYMBOLS {
                black_box(Symbol::string("hello").unwrap());
            }
        });
    });
    g.bench_function("create_nested", |b| {
        b.iter(|| {
            for i in 0..N_SYMBOLS as i32 {
                black_box(nested(i));
            }
        });
    });
    let texts: Vec<String> = (0..N_SYMBOLS)
        .map(|i| format!("p({i},f(a,\"x\"),-q(1,2))"))
        .collect();
    g.bench_function("parse", |b| {
        b.iter(|| {
            for t in &texts {
                black_box(t.parse::<Symbol>().unwrap());
            }
        });
    });
    let syms: Vec<Symbol> = (0..N_SYMBOLS as i32).map(nested).collect();
    g.bench_function("to_string", |b| {
        b.iter(|| {
            for s in &syms {
                black_box(s.to_string());
            }
        });
    });
    // name, arguments and the number of every node of the tree.
    fn walk_sym(s: Symbol) -> usize {
        match s.arguments() {
            Some(args) => {
                black_box(s.name());
                1 + args.iter().map(|a| walk_sym(*a)).sum::<usize>()
            }
            None => usize::from(s.as_number().is_some() || s.as_string().is_some()),
        }
    }
    g.bench_function("inspect_tree", |b| {
        b.iter(|| {
            let mut n = 0;
            for s in &syms {
                n += walk_sym(*s);
            }
            black_box(n)
        });
    });
    g.bench_function("kind", |b| {
        b.iter(|| {
            for s in &syms {
                black_box(s.kind());
            }
        });
    });
    g.bench_function("eq_hash_ord", |b| {
        b.iter(|| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            let mut less = 0usize;
            for w in syms.windows(2) {
                use std::hash::{Hash, Hasher};
                w[0].hash(&mut h);
                less += usize::from(w[0] < w[1]);
                black_box(w[0] == w[1]);
                black_box(h.finish());
            }
            black_box(less)
        });
    });
    g.finish();

    let mut g = small(c, "convert");
    g.throughput(Throughput::Elements(N_SYMBOLS as u64));
    let edges: Vec<Edge> = (0..N_SYMBOLS as i32)
        .map(|i| Edge {
            from: i,
            to: i + 1,
            label: "road".to_owned(),
        })
        .collect();
    g.bench_function("derive_to_symbol", |b| {
        b.iter(|| {
            for e in &edges {
                black_box(e.to_symbol().unwrap());
            }
        });
    });
    let edge_syms: Vec<Symbol> = edges.iter().map(|e| e.to_symbol().unwrap()).collect();
    g.bench_function("derive_from_symbol", |b| {
        b.iter(|| {
            for s in &edge_syms {
                black_box(Edge::from_symbol(*s).unwrap());
            }
        });
    });
    g.bench_function("derive_round_trip", |b| {
        b.iter(|| {
            for e in &edges {
                black_box(Edge::from_symbol(e.to_symbol().unwrap()).unwrap());
            }
        });
    });
    g.finish();
}

// ---------------------------------------------------------------------------
// Models

const MODELS_PROGRAM: &str = "p(1..100). {q(1..10)}.";
const MODELS: u64 = 1024;

fn models_control() -> Control {
    let mut ctl = Control::with_args(["0"]).unwrap();
    ctl.add_base(MODELS_PROGRAM).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl
}

fn models(c: &mut Criterion) {
    let mut g = small(c, "models");
    g.throughput(Throughput::Elements(MODELS));
    g.bench_function("count_only", |b| {
        b.iter_batched(
            models_control,
            |mut ctl| {
                let mut n = 0u64;
                let _ = ctl
                    .for_each_model(&[], |_m| {
                        n += 1;
                        Ok(ControlFlow::Continue(()))
                    })
                    .unwrap();
                assert_eq!(n, MODELS);
            },
            BatchSize::PerIteration,
        );
    });
    g.bench_function("symbols_shown", |b| {
        b.iter_batched(
            models_control,
            |mut ctl| {
                let mut n = 0usize;
                let _ = ctl
                    .for_each_model(&[], |m| {
                        n += m.symbols(ShowType::SHOWN)?.len();
                        Ok(ControlFlow::Continue(()))
                    })
                    .unwrap();
                assert_eq!(n, 110 * 1024 - 10 * 512);
            },
            BatchSize::PerIteration,
        );
    });
    g.bench_function("symbols_and_strings", |b| {
        b.iter_batched(
            models_control,
            |mut ctl| {
                let _ = ctl
                    .for_each_model(&[], |m| {
                        for s in m.symbols(ShowType::SHOWN)? {
                            black_box(s.to_string());
                        }
                        Ok(ControlFlow::Continue(()))
                    })
                    .unwrap();
            },
            BatchSize::PerIteration,
        );
    });
    g.bench_function("typed_atoms", |b| {
        b.iter_batched(
            models_control,
            |mut ctl| {
                let _ = ctl
                    .for_each_model(&[], |m| {
                        black_box(m.atoms::<P>()?);
                        Ok(ControlFlow::Continue(()))
                    })
                    .unwrap();
            },
            BatchSize::PerIteration,
        );
    });
    g.bench_function("contains", |b| {
        let probe = Symbol::function("q", &[Symbol::number(5)]).unwrap();
        b.iter_batched(
            models_control,
            |mut ctl| {
                let mut n = 0u64;
                let _ = ctl
                    .for_each_model(&[], |m| {
                        n += u64::from(m.contains(probe)?);
                        Ok(ControlFlow::Continue(()))
                    })
                    .unwrap();
                black_box(n);
            },
            BatchSize::PerIteration,
        );
    });
    g.bench_function("solve_all_owned", |b| {
        b.iter_batched(
            models_control,
            |mut ctl| {
                let (_, ms) = ctl.solve_all().unwrap();
                assert_eq!(ms.len() as u64, MODELS);
            },
            BatchSize::PerIteration,
        );
    });
    g.finish();
}

// ---------------------------------------------------------------------------
// Symbolic atoms

const ATOMS: usize = 20_000;

fn atoms_control() -> Control {
    let mut ctl = Control::new().unwrap();
    ctl.add_base(&format!("p(1..{ATOMS}). {{q(1..{ATOMS})}}."))
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl
}

fn atoms(c: &mut Criterion) {
    let mut g = small(c, "atoms");
    g.throughput(Throughput::Elements(2 * ATOMS as u64));
    let ctl = atoms_control();
    g.bench_function("iterate_symbol_literal", |b| {
        b.iter(|| {
            let mut n = 0i64;
            for atom in &ctl.symbolic_atoms().unwrap() {
                let atom = atom.unwrap();
                black_box(atom.symbol());
                n += i64::from(atom.literal().get());
            }
            black_box(n)
        });
    });
    g.bench_function("typed_of", |b| {
        b.iter(|| black_box(ctl.symbolic_atoms().unwrap().of::<P>().unwrap()));
    });
    g.finish();
}

// ---------------------------------------------------------------------------
// Facts

fn facts(c: &mut Criterion) {
    let mut g = small(c, "facts");
    for n in [10_000usize] {
        g.throughput(Throughput::Elements(n as u64));
        let edges: Vec<Edge> = (0..n as i32)
            .map(|i| Edge {
                from: i,
                to: i + 1,
                label: "road".to_owned(),
            })
            .collect();
        g.bench_function("add_facts_10k", |b| {
            b.iter_batched(
                || Control::new().unwrap(),
                |mut ctl| {
                    ctl.add_facts(&edges).unwrap();
                },
                BatchSize::PerIteration,
            );
        });
        // The same facts as program text, added and grounded by hand: what
        // add_facts does minus the conversion and the checks.
        let mut text = String::new();
        for e in &edges {
            writeln!(text, "edge({},{},{}).", e.from, e.to, e.label).unwrap();
        }
        g.bench_function("text_facts_10k", |b| {
            b.iter_batched(
                || Control::new().unwrap(),
                |mut ctl| {
                    ctl.add_base(&text).unwrap();
                    ctl.ground(&[Part::base()]).unwrap();
                },
                BatchSize::PerIteration,
            );
        });
        g.bench_function("to_symbol_only_10k", |b| {
            b.iter(|| {
                for e in &edges {
                    black_box(e.to_symbol().unwrap());
                }
            });
        });
    }
    g.finish();
}

// ---------------------------------------------------------------------------
// Ground callbacks

const CALLS: usize = 20_000;

fn ground_callbacks(c: &mut Criterion) {
    let mut g = small(c, "ground_callbacks");
    g.throughput(Throughput::Elements(CALLS as u64));
    let with = format!("p(1..{CALLS}). q(X, @f(X)) :- p(X).");
    let without = format!("p(1..{CALLS}). q(X, X*2) :- p(X).");
    g.bench_function("baseline_no_callback", |b| {
        b.iter_batched(
            || {
                let mut ctl = Control::new().unwrap();
                ctl.add_base(&without).unwrap();
                ctl
            },
            |mut ctl| ctl.ground(&[Part::base()]).unwrap(),
            BatchSize::PerIteration,
        );
    });
    g.bench_function("callback_20k", |b| {
        b.iter_batched(
            || {
                let mut ctl = Control::new().unwrap();
                ctl.add_base(&with).unwrap();
                ctl
            },
            |mut ctl| {
                ctl.ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
                    let n = call.args()[0].as_number().unwrap();
                    call.push(Symbol::number(n * 2))
                })
                .unwrap();
            },
            BatchSize::PerIteration,
        );
    });
    g.finish();
}

// ---------------------------------------------------------------------------
// Propagator

struct Watching {
    calls: Arc<AtomicUsize>,
    changes: Arc<AtomicUsize>,
}

impl Propagator for Watching {
    fn init(&self, init: &mut PropagateInit<'_>) -> clingox::Result<()> {
        let atoms = init.symbolic_atoms()?;
        let mut literals = Vec::new();
        for atom in &atoms {
            literals.push(atom?.literal());
        }
        for l in literals {
            let s = init.solver_literal(l)?;
            init.add_watch(s)?;
        }
        Ok(())
    }

    fn propagate(
        &self,
        _control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> clingox::Result<()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.changes.fetch_add(changes.len(), Ordering::Relaxed);
        Ok(())
    }
}

fn propagator(c: &mut Criterion) {
    let mut g = small(c, "propagator");
    // Two shapes: every model of a free choice (one propagate call per
    // decision on a watched atom), and a refutation with many conflicts.
    let programs = [
        ("enumerate", "{a(1..16)}.".to_owned(), "0"),
        ("pigeons", pigeons(9, 8), "1"),
    ];
    for (name, program, models) in &programs {
        let calls = Arc::new(AtomicUsize::new(0));
        let changes = Arc::new(AtomicUsize::new(0));
        let make = |with: bool| {
            let mut ctl = Control::with_args([*models]).unwrap();
            ctl.add_base(program).unwrap();
            ctl.ground(&[Part::base()]).unwrap();
            if with {
                ctl.register_propagator(Watching {
                    calls: calls.clone(),
                    changes: changes.clone(),
                })
                .unwrap();
            }
            ctl
        };
        g.bench_function(format!("{name}_baseline"), |b| {
            b.iter_batched(
                || make(false),
                |mut ctl| {
                    let _ = ctl.solve(&[]).unwrap();
                },
                BatchSize::PerIteration,
            );
        });
        g.bench_function(format!("{name}_watch_all"), |b| {
            b.iter_batched(
                || make(true),
                |mut ctl| {
                    let _ = ctl.solve(&[]).unwrap();
                },
                BatchSize::PerIteration,
            );
        });
        // One solve to report how many callbacks the propagator sees.
        calls.store(0, Ordering::Relaxed);
        changes.store(0, Ordering::Relaxed);
        let _ = make(true).solve(&[]).unwrap();
        eprintln!(
            "propagator {name}: {} propagate calls, {} changes per solve",
            calls.load(Ordering::Relaxed),
            changes.load(Ordering::Relaxed)
        );
    }
    g.finish();
}

// ---------------------------------------------------------------------------
// Observer

const OBS_RULES: usize = 30_000;

#[derive(Default)]
struct Counting {
    rules: Arc<AtomicUsize>,
}

impl GroundProgramObserver for Counting {
    fn rule(
        &mut self,
        _choice: bool,
        head: &[Atom],
        body: &[ProgramLiteral],
    ) -> clingox::Result<()> {
        self.rules
            .fetch_add(1 + head.len() + body.len(), Ordering::Relaxed);
        Ok(())
    }
    fn output_atom(&mut self, symbol: Symbol, _atom: Option<Atom>) -> clingox::Result<()> {
        black_box(symbol);
        Ok(())
    }
}

fn observer(c: &mut Criterion) {
    let mut g = small(c, "observer");
    g.throughput(Throughput::Elements(OBS_RULES as u64));
    let program = format!("n(1..{OBS_RULES}). {{a(X)}} :- n(X). b(X) :- a(X), n(X).");
    let make = |with: bool| {
        let mut ctl = Control::new().unwrap();
        if with {
            ctl.register_observer(Counting::default(), false).unwrap();
        }
        ctl.add_base(&program).unwrap();
        ctl
    };
    g.bench_function("baseline_no_observer", |b| {
        b.iter_batched(
            || make(false),
            |mut ctl| ctl.ground(&[Part::base()]).unwrap(),
            BatchSize::PerIteration,
        );
    });
    g.bench_function("counting_observer", |b| {
        b.iter_batched(
            || make(true),
            |mut ctl| ctl.ground(&[Part::base()]).unwrap(),
            BatchSize::PerIteration,
        );
    });
    g.finish();
}

// ---------------------------------------------------------------------------
// AST

const AST_RULES: usize = 3000;

fn ast_program() -> String {
    let mut text = String::new();
    for i in 0..AST_RULES {
        writeln!(
            text,
            "r{i}(X,Y) :- e(X,Z), e(Z,Y), X != Y, not f(X,{i}), #count {{ W : g(W,X) }} > 1."
        )
        .unwrap();
    }
    text
}

struct CountNodes(usize);

impl Visitor for CountNodes {
    fn visit(&mut self, node: &Ast) -> clingox::Result<Ast> {
        self.0 += 1;
        walk(self, node)
    }
}

struct Rename;

impl Visitor for Rename {
    fn visit_variable(&mut self, node: &Ast) -> clingox::Result<Ast> {
        let span = node.span(Attribute::Location)?;
        let name = node.string(Attribute::Name)?;
        ast::variable(&span, &format!("{name}1"))
    }
}

fn ast_benches(c: &mut Criterion) {
    let mut g = small(c, "ast");
    let text = ast_program();
    g.throughput(Throughput::Elements(AST_RULES as u64));
    let parse = |text: &str| {
        let mut all = Vec::new();
        ast::parse_string(text, |s| {
            all.push(s);
            Ok(())
        })
        .unwrap();
        all
    };
    g.bench_function("parse_3k_rules", |b| b.iter(|| black_box(parse(&text))));
    let statements = parse(&text);
    g.bench_function("visit_count", |b| {
        b.iter(|| {
            let mut v = CountNodes(0);
            for s in &statements {
                v.visit(s).unwrap();
            }
            black_box(v.0)
        });
    });
    g.bench_function("visit_rename_variables", |b| {
        b.iter(|| {
            for s in &statements {
                black_box(Rename.visit(s).unwrap());
            }
        });
    });
    g.bench_function("to_string", |b| {
        b.iter(|| {
            for s in &statements {
                black_box(s.to_string());
            }
        });
    });
    g.bench_function("build_3k_facts", |b| {
        let span = Span::synthetic();
        b.iter(|| {
            for i in 0..AST_RULES as i32 {
                let arg = ast::symbolic_term(&span, Symbol::number(i)).unwrap();
                let f = ast::function(&span, "p", &[arg], false).unwrap();
                let atom = ast::symbolic_atom(&f).unwrap();
                let lit = ast::literal(&span, LiteralSign::NoSign, &atom).unwrap();
                black_box(ast::rule(&span, &lit, &[]).unwrap());
            }
        });
    });
    g.bench_function("program_builder_add_3k", |b| {
        b.iter_batched(
            || Control::new().unwrap(),
            |mut ctl| {
                ctl.with_program_builder(|builder| {
                    for s in &statements {
                        builder.add(s)?;
                    }
                    Ok(())
                })
                .unwrap();
            },
            BatchSize::PerIteration,
        );
    });
    g.finish();
}

// ---------------------------------------------------------------------------
// Configuration and statistics

fn walk_config(ctl: &mut Control) -> usize {
    fn go(cfg: &clingox::Configuration<'_>, path: &str, n: &mut usize) {
        *n += 1;
        match cfg.kind(path).unwrap() {
            ConfigKind::Value => {
                black_box(cfg.get(path).unwrap());
            }
            ConfigKind::Array | ConfigKind::ArrayMap => {
                for i in 0..cfg.len(path).unwrap() {
                    go(cfg, &join(path, &i.to_string()), n);
                }
            }
            ConfigKind::Map => {
                for k in cfg.keys(path).unwrap() {
                    go(cfg, &join(path, &k), n);
                }
            }
            _ => {}
        }
    }
    fn join(path: &str, part: &str) -> String {
        if path.is_empty() {
            part.to_owned()
        } else {
            format!("{path}.{part}")
        }
    }
    let cfg = ctl.configuration();
    let mut n = 0;
    go(&cfg, "", &mut n);
    n
}

fn walk_stats(tree: &StatsTree) -> usize {
    match tree {
        StatsTree::Value(v) => usize::from(*v >= 0.0),
        StatsTree::Array(a) => a.iter().map(walk_stats).sum(),
        StatsTree::Map(m) => m.iter().map(|(_, t)| walk_stats(t)).sum(),
        _ => 0,
    }
}

/// The walk of `walk_config` through the entry cursor: every node counted,
/// every value read, one or two C calls per step and no path.
fn walk_config_entries(ctl: &mut Control) -> usize {
    fn go(entry: ConfigEntry<'_>, n: &mut usize) {
        *n += 1;
        match entry.kind().unwrap() {
            ConfigKind::Value => {
                black_box(entry.value().unwrap());
            }
            _ => {
                for child in entry.children().unwrap() {
                    go(child.unwrap().1, n);
                }
            }
        }
    }
    let cfg = ctl.configuration();
    let mut n = 0;
    go(cfg.root().unwrap(), &mut n);
    n
}

/// The walk of `walk_stats` through the entry cursor, without building a tree.
fn walk_stats_entries(entry: StatsEntry<'_>) -> usize {
    match entry.kind().unwrap() {
        StatKind::Value => usize::from(black_box(entry.value().unwrap()) >= 0.0),
        _ => entry
            .children()
            .unwrap()
            .map(|child| walk_stats_entries(child.unwrap().1))
            .sum(),
    }
}

fn config_and_stats(c: &mut Criterion) {
    let mut g = small(c, "config_stats");
    let mut ctl = Control::new().unwrap();
    let nodes = walk_config(&mut ctl);
    eprintln!("configuration: {nodes} nodes");
    g.throughput(Throughput::Elements(nodes as u64));
    g.bench_function("config_walk", |b| {
        b.iter(|| black_box(walk_config(&mut ctl)));
    });
    assert_eq!(walk_config_entries(&mut ctl), nodes);
    g.bench_function("config_walk_entries", |b| {
        b.iter(|| black_box(walk_config_entries(&mut ctl)));
    });
    let mut solved = Control::with_args(["--stats"]).unwrap();
    solved.add_base("p(1..50). {q(1..8)}.").unwrap();
    solved.ground(&[Part::base()]).unwrap();
    let _ = solved.solve(&[]).unwrap();
    let tree = solved.statistics().unwrap().snapshot().unwrap();
    let vals = walk_stats(&tree);
    eprintln!("statistics: {vals} values");
    g.throughput(Throughput::Elements(vals as u64));
    g.bench_function("stats_snapshot", |b| {
        b.iter(|| black_box(solved.statistics().unwrap().snapshot().unwrap()));
    });
    assert_eq!(
        walk_stats_entries(solved.statistics().unwrap().root()),
        vals
    );
    g.bench_function("stats_walk_entries", |b| {
        b.iter(|| {
            let stats = solved.statistics().unwrap();
            black_box(walk_stats_entries(stats.root()))
        });
    });
    g.bench_function("stats_path_reads", |b| {
        b.iter(|| {
            let stats = solved.statistics().unwrap();
            for _ in 0..100 {
                black_box(stats.value("summary.models.enumerated").unwrap());
            }
        });
    });
    g.bench_function("stats_entry_reads", |b| {
        b.iter(|| {
            let stats = solved.statistics().unwrap();
            let entry = stats.entry("summary.models.enumerated").unwrap();
            for _ in 0..100 {
                black_box(entry.value().unwrap());
            }
        });
    });
    g.finish();
}

// ---------------------------------------------------------------------------
// Application::run against a plain control

fn run_body(ctl: &mut clingox::ScopedControl<'_>) -> clingox::Result<()> {
    ctl.add_base("a. {b}.")?;
    ctl.ground(&[Part::base()])?;
    let _ = ctl.solve(&[])?;
    Ok(())
}

fn application(c: &mut Criterion) {
    let mut g = small(c, "application");
    g.bench_function("plain_control", |b| {
        b.iter(|| {
            let mut ctl = Control::new().unwrap();
            ctl.add_base("a. {b}.").unwrap();
            ctl.ground(&[Part::base()]).unwrap();
            let _ = ctl.solve(&[]).unwrap();
        });
    });
    g.bench_function("application_run", |b| {
        b.iter(|| {
            let code = Application::new()
                .main(|ctl, _files| run_body(ctl))
                .run(["--outf=3"])
                .unwrap();
            black_box(code)
        });
    });
    g.finish();
}

// ---------------------------------------------------------------------------
// The cost of a blocking solve on a control that has already solved

/// A multi-shot program of thousands of tiny solves pays the fixed cost of a
/// solve each time. `blocking_solve` is the row that shows it: `Control::solve`
/// of `{a}.` on one control. With threads, `solve` starts clasp's thread for
/// the search unless nothing can interrupt it, so `with_live_handle`, which
/// keeps an `InterruptHandle` alive, is the same call on the path that always
/// starts the thread.
fn blocking_solve(c: &mut Criterion) {
    let mut g = small(c, "blocking_solve");
    let mut ctl = Control::new().unwrap();
    ctl.add_base("{a}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    assert!(ctl.solve(&[]).unwrap().is_sat());
    g.bench_function("multishot_trivial_solve", |b| {
        b.iter(|| black_box(ctl.solve(&[]).unwrap()));
    });
    let handle = ctl.interrupt_handle();
    g.bench_function("multishot_trivial_solve_with_live_handle", |b| {
        b.iter(|| black_box(ctl.solve(&[]).unwrap()));
    });
    drop(handle);
    g.finish();
}

// ---------------------------------------------------------------------------
// End to end

fn queens(n: usize) -> String {
    format!(
        "#const n={n}. n(1..n). {{q(X,Y)}} :- n(X), n(Y). :- n(X), not 1 = {{q(X,Y)}} 1. \
         :- n(Y), not 1 = {{q(X,Y)}} 1. :- q(X,Y), q(X2,Y2), (X,Y) < (X2,Y2), X-Y = X2-Y2. \
         :- q(X,Y), q(X2,Y2), (X,Y) < (X2,Y2), X+Y = X2+Y2."
    )
}

fn pigeons(p: usize, h: usize) -> String {
    format!(
        "p(1..{p}). h(1..{h}). 1 {{ in(P,H) : h(H) }} 1 :- p(P). :- in(P1,H), in(P2,H), P1 < P2."
    )
}

fn end_to_end(c: &mut Criterion) {
    let mut g = small(c, "end_to_end");
    let q = queens(8);
    g.bench_function("queens8_all", |b| {
        b.iter(|| {
            let mut ctl = Control::with_args(["0"]).unwrap();
            ctl.add_base(&q).unwrap();
            ctl.ground(&[Part::base()]).unwrap();
            let mut n = 0;
            let _ = ctl
                .for_each_model(&[], |_m| {
                    n += 1;
                    Ok(ControlFlow::Continue(()))
                })
                .unwrap();
            assert_eq!(n, 92);
        });
    });
    let ph = pigeons(8, 7);
    g.bench_function("pigeons8_7_unsat", |b| {
        b.iter(|| {
            let mut ctl = Control::new().unwrap();
            ctl.add_base(&ph).unwrap();
            ctl.ground(&[Part::base()]).unwrap();
            assert!(ctl.solve(&[]).unwrap().is_unsat());
        });
    });
    g.finish();
}

criterion_group!(
    benches,
    symbols,
    models,
    atoms,
    facts,
    ground_callbacks,
    propagator,
    observer,
    ast_benches,
    config_and_stats,
    application,
    blocking_solve,
    end_to_end
);
criterion_main!(benches);

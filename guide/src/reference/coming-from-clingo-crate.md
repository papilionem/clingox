# Coming from the clingo crate

This page is for programs written against Potassco's
[`clingo`](https://crates.io/crates/clingo) crate (version 0.8, which binds
clingo 5.6). It maps that crate's calls to clingox's, and gives recipes for
the places where the two designs differ enough that a line-by-line port does
not work.

The mapping comes from a real port. The crate's own examples and tests, and
four programs that use it (savan, aspire, dasp and iggy), were ported to
clingox and run against the originals; 13 of 13 examples, 27 of 27 integration
tests and the four applications' test suites pass.

## What changes

- **Errors are one type.** Every fallible call returns `clingox::Result<T>`.
  There is no `expect` on every line: use `?`. See [Errors and
  panics](../concepts/errors-and-panics.md).
- **The control is borrowed, not consumed.** `Control::solve` in the clingo
  crate takes the control by value and the handle owns it until `close`. In
  clingox, `solve_yield` borrows the control for as long as the handle lives,
  and the control is usable again afterwards. The
  [first recipe](#models-over-a-borrowed-control) shows what to do when a
  design relied on the handle owning the control.
- **Propagators take `&self`.** The trait methods are shared, `Send + Sync`,
  and fallible. State that changes lives behind atomics, locks or one slot per
  solver thread. See [the second recipe](#propagator-state-under-self).
- **Clingo 5.8.2, not 5.6.** A few calls changed with it (see
  [Differences that come from clingo 5.8](#differences-that-come-from-clingo-58)).
- **No `unsafe`, no raw pointers.** Where the clingo crate hands out a `u64`
  key or a raw literal, clingox uses a path, a typed handle or a value.

## API mapping

| clingo crate | clingox |
|---|---|
| `control(args)` | `Control::with_args(args)`; `Control::new()` for no arguments |
| `control_with_context` and `ControlCtx` | `Control::builder()`, then `.logger(f)`, `.args(..)`, `.build()`; register a propagator, observer or function handler on the control |
| `ctl.add("base", &[], text)` | `ctl.add("base", &[], text)`; `ctl.add_base(text)` for the base part |
| `Part::new("p", vec![sym])` | `Part::new("p", &[sym])?`; `Part::base()` |
| `ctl.ground(&parts)` | `ctl.ground(&[part])`; with an external function handler, `ctl.ground_with(&[part], f)` |
| `ctl.solve(SolveMode::YIELD, &[])`, then `resume` and `model` | `ctl.solve_yield(&[])`, then `handle.next_model()?` |
| `ctl.solve(SolveMode::ASYNC, &[])` | `ctl.solve_async(&[])`; `AsyncSolveHandle::wait`, `get`, `cancel` |
| `ctl.solve_with_event_handler(mode, &[], h)` | `ctl.solve_with_events(options, h)`, `solve_yield_with_events`, `solve_async_with_events` |
| `handle.get()`, `handle.close()` | `handle.get()`, `handle.close()` (returns the `SolveResult`) |
| `SolveMode::YIELD` loop that collects models | `ctl.solve_all()`, `ctl.solve_first()`, `ctl.solve_optimal()`, `ctl.for_each_model(&[], f)` |
| `SolverLiteral` as an assumption | `Assumption::from((symbol, true))` or `Assumption::from(program_literal)` |
| `ctl.interrupt()` | `ctl.interrupt_handle().interrupt()`; the handle is `Send` |
| `ctl.load(path)` | `ctl.load(path)` |
| `ctl.get_const(name)` | `ctl.get_const(name)?`, which returns an `Option<Symbol>` |
| `ctl.cleanup()` | `ctl.cleanup()` |
| `ctl.assign_external(&sym, TruthValue::True)` | `ctl.assign_external(sym, TruthValue::True)` |
| `ctl.release_external(&sym)` | `ctl.release_external(sym)` |
| `model.symbols(ShowType::SHOWN)` | `model.symbols(ShowType::SHOWN)?` |
| `model.model_type()` and `ModelType` | `model.kind()?` and `ModelKind` (in the prelude) |
| `model.number()` | `model.number()`, a plain `u64` |
| `model.cost()` | `model.cost()?`; on an owned model, `cost()` and `priorities()` |
| `model.contains(sym)` | `model.contains(sym)?` |
| `model.context()` | `model.context()`, returning `SolveControl` |
| `ctl.symbolic_atoms()?.iter()?` | `ctl.symbolic_atoms()?.iter()`, items are `Result<SymbolicAtom>` |
| `atoms.iter()?.find(..)` on a symbol | `atoms.find(symbol)?`, which returns an `Option` |
| typed reads of an atom set | `ctl.symbolic_atoms()?.of::<T>()?` (see [below](#typed-symbolic-atom-reads)) |
| `ctl.theory_atoms()` | `ctl.theory_atoms()`; terms are `TheoryTerm` values and `Id`s |
| `ctl.backend()` | `ctl.with_backend(\|backend\| ..)` |
| `configuration_mut()` and `Id` keys | `ctl.configuration()` with paths such as `"solve.models"` (`get`, `set`, `keys`, `kind`, `len`, `element`), or a `ConfigEntry` from `root()` or `entry(path)`, which holds the key for one entry as the crate's `Id` does, steps with `children()` and carries no path ([Configure the solver](../how-to/configuration.md)) |
| `configuration_type(key)` | `conf.kind(path)?`, a `ConfigKind` |
| `ctl.statistics()` with `u64` keys | `ctl.statistics()?`, paths such as `"summary.times.solve"`, or a `StatsEntry` from `root()` or `entry(path)`, which holds the key for one entry as the crate's `u64` does and steps with `children()`; `.snapshot()?` gives a `StatsTree` ([Reading statistics by entry](../concepts/solve-events.md#reading-statistics-by-entry)) |
| `Symbol::create_id("a", true)` | `Symbol::function("a", &[])?` |
| `Symbol::create_number(n)` | `Symbol::number(n)` |
| `Symbol::create_string(s)` | `Symbol::string(s)?` |
| `Symbol::create_function(name, &args, true)` | `Symbol::function(name, &args)?`; `Symbol::function_with_sign` for a negative sign |
| `symbol.to_string()` | `symbol.to_string()` or `format!("{symbol}")` |
| `Signature::new(name, arity, true)` | `Signature::new(name, arity)?` |
| `Propagator` with `&mut self` methods returning `bool` | `Propagator` with `&self` methods returning `Result` |
| `SolveEventHandler::on_solve_event` | `SolveEventHandler` with one method per event |
| `ast::Location::default()` | `ast::Span::synthetic()` |
| `clingo::version()` | `clingox::version()` |
| `#[derive(ToSymbol)]` from `clingo-derive` | `#[derive(ToSymbol)]` and `FromSymbol` from `clingox` (see [below](#the-derive-names-and-field-types)) |

## Models over a borrowed control

In the clingo crate the solve handle owns the control, so a function can
return the handle and the caller iterates the models. In clingox the handle
borrows the control. A function that wants to hand out models takes
`&mut Control` and returns an iterator that keeps the handle inside it. The
models are copied with `Model::snapshot`, so they outlive the search:

```rust
use clingox::prelude::*;

/// The models of the grounded program, one at a time. The control is
/// borrowed until the iterator is dropped.
fn models<'c>(
    ctl: &'c mut Control,
) -> clingox::Result<impl Iterator<Item = clingox::Result<OwnedModel>> + 'c> {
    let mut handle = ctl.solve_yield(&[])?;
    let mut failed = false;
    Ok(std::iter::from_fn(move || {
        if failed {
            return None;
        }
        match handle.next_model() {
            Ok(Some(model)) => Some(model.snapshot().inspect_err(|_| failed = true)),
            Ok(None) => None,
            Err(error) => {
                failed = true;
                Some(Err(error))
            }
        }
    }))
}

let mut ctl = Control::with_args(["--models=0"])?;
ctl.add_base("{a;b}.")?;
ctl.ground(&[Part::base()])?;

let all = models(&mut ctl)?.collect::<clingox::Result<Vec<_>>>()?;
assert_eq!(all.len(), 4);

// The iterator is gone, so the control is free again.
assert!(ctl.solve(&[])?.is_sat());
# Ok::<(), clingox::Error>(())
```

Dropping the iterator closes the search, so a caller may stop early. When a
struct owns the control, give it a method with the same shape,
`fn models(&mut self) -> Result<impl Iterator<..> + '_>`, and call
`models(&mut self.ctl)` inside it.

Most code does not need this. `solve_all` returns every model as a `Vec`,
`solve_first` and `solve_optimal` return one, and `for_each_model` runs a
closure per model and can stop the search by returning
`ControlFlow::Break(())`.

## Propagator state under `&self`

The clingo crate's propagator methods take `&mut self`, so a propagator can
keep a `Vec` of per-thread state and index it directly. In clingox they take
`&self` and the trait requires `Send + Sync`, because with several solver
threads two of them can call `propagate` on the same propagator at once.

The pattern that replaces `&mut self` is one slot of state per solver thread,
found by `PropagateControl::thread_id`. `init` runs before any solver thread
exists and knows how many there will be, so it builds the slots. Each slot has
its own lock, which no other thread takes, so the lock never waits:

```rust
use clingox::propagate::{PropagateControl, PropagateInit, Propagator, SolverLiteral};
use clingox::prelude::*;
use std::sync::{Mutex, RwLock};

/// Keeps, for each solver thread, the watched literals that are true.
struct Trail {
    per_thread: RwLock<Vec<Mutex<Vec<SolverLiteral>>>>,
}

impl Propagator for Trail {
    fn init(&self, init: &mut PropagateInit<'_>) -> clingox::Result<()> {
        // `init` runs before every solve call, and the thread count can change
        // between calls, so the slots are rebuilt each time.
        let threads = init.number_of_threads() as usize;
        *self.per_thread.write().unwrap() = (0..threads).map(|_| Mutex::default()).collect();

        let mut watched = Vec::new();
        for atom in init.symbolic_atoms()?.by_signature(Signature::new("x", 1)?) {
            watched.push(init.solver_literal(atom?.literal())?);
        }
        for literal in watched {
            init.add_watch(literal)?;
        }
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> clingox::Result<()> {
        let slots = self.per_thread.read().unwrap();
        slots[control.thread_id() as usize].lock().unwrap().extend_from_slice(changes);
        Ok(())
    }

    fn undo(&self, control: &PropagateControl<'_>, changes: &[SolverLiteral]) {
        let slots = self.per_thread.read().unwrap();
        let mut trail = slots[control.thread_id() as usize].lock().unwrap();
        let keep = trail.len().saturating_sub(changes.len());
        trail.truncate(keep);
    }
}

let mut ctl = Control::new()?;
ctl.add_base("{x(1..3)}.")?;
ctl.ground(&[Part::base()])?;
ctl.register_propagator(Trail { per_thread: RwLock::default() })?;
let (_, models) = ctl.solve_all()?;
assert_eq!(models.len(), 8);
# Ok::<(), clingox::Error>(())
```

Two rules keep this sound. Do not hold a lock of your own across a call into
`PropagateControl` (`add_clause`, `propagate`), because a future clasp could
call `undo` from inside it. And a value set once in `init` and only read
afterwards belongs in a `OnceLock`, as the [propagator
guide](../how-to/write-a-propagator.md) shows. There is no public integer
behind a `SolverLiteral`: to use one as a vector index, keep a
`HashMap<SolverLiteral, usize>` built in `init`, or call
`SolverLiteral::variable`, which gives the literal's variable number.

## Typed symbolic-atom reads

The clingo crate walks the symbolic atoms and converts each symbol by hand.
`SymbolicAtoms::of` does this for any type that derives `FromSymbol`: it
selects the atoms with the type's name and arity, and converts each one.

```rust
use clingox::prelude::*;

#[derive(FromSymbol, Debug, PartialEq)]
struct Edge(i32, i32);

let mut ctl = Control::new()?;
ctl.add_base("edge(2,3). {edge(1,2)}. node(1).")?;
ctl.ground(&[Part::base()])?;

// Every ground `edge/2` atom, whether it is a fact, a choice or an
// external, in the order of their symbols.
let edges = ctl.symbolic_atoms()?.of::<Edge>()?;
assert_eq!(edges, [Edge(1, 2), Edge(2, 3)]);
# Ok::<(), clingox::Error>(())
```

It reads the grounding, not a model, so an atom appears whether or not it is
true in some answer set. A symbol that does not fit the type is an
`ErrorKind::Conversion` error, never a skipped atom. For a model, use
`Model::atoms` and `Model::shown`, which work the same way.

## Synthetic spans

The clingo crate builds AST nodes with `Location::default()`. clingox nodes
carry a `Span`, and `Span::synthetic()` is the same idea: the file
`<generated>`, line 1, column 1. clingo accepts any location, and the span
comes back unchanged from the node.

```rust
use clingox::Symbol;
use clingox::ast::{self, Attribute, Span};

let span = Span::synthetic();
let atom = ast::symbolic_atom(&ast::symbolic_term(&span, Symbol::function("e", &[])?)?)?;
// clingo 5.8 reads the starting truth value from a term, not a number.
let kind = ast::symbolic_term(&span, Symbol::function("true", &[])?)?;
let external = ast::external(&span, &atom, &[], &kind)?;
assert_eq!(external.to_string(), "#external e. [true]");
assert_eq!(external.span(Attribute::Location)?, span);
# Ok::<(), clingox::Error>(())
```

Use `Span::new` when you want a real position for error messages.

## The derive: names and field types

`clingox` has its own `ToSymbol` and `FromSymbol` derives, and they are not a
drop-in replacement for `clingo-derive`. Two things differ.

**Names.** `clingo-derive` writes `Test2` as `test_2`. clingox puts an
underscore before each uppercase letter that starts a word and none before a
digit, so `Test2` is `test2` and `Point3D` is `point3_d`. A name that matters
is better spelled out with `#[clingo(name = "..")]`, which works on a type and
on an enum variant. The full rule is in the documentation of
`FromSymbol` in the API reference.

**Field types.** clingo-derive converts `String`, `&str`, `bool` and
`Option` fields on its own. clingox refuses to guess, because a Rust string
can be either a clingo constant (`comp13`) or a clingo string (`"comp13"`),
and the two never match each other in rules:

- A `String` field must carry `#[clingo(string)]` or `#[clingo(constant)]`.
- A `&str` or `&String` field is not supported. Store a `String`, or write a
  small wrapper type with a `ToSymbol` implementation.
- `bool`, `Option<T>` and floats have no clingo value. Model them the way the
  program does: a `bool` as the two constants `true` and `false`, an `Option`
  as two predicates, or one enum with a `none` and a `some(..)` variant.

The workaround for a field type the derive does not know is a wrapper with a
hand-written implementation of the trait, which then derives around it:

```rust
use clingox::{Error, ErrorKind, FromSymbol, Sign, Symbol, ToSymbol};

/// A flag that clingo programs write as the constants `true` and `false`.
#[derive(Debug, PartialEq)]
struct Flag(bool);

impl ToSymbol for Flag {
    fn to_symbol(&self) -> clingox::Result<Symbol> {
        Symbol::function(if self.0 { "true" } else { "false" }, &[])
    }
}

impl FromSymbol for Flag {
    fn from_symbol(symbol: Symbol) -> clingox::Result<Flag> {
        // A constant is a function without arguments; a negative one is `-true`.
        let constant = symbol.sign() == Some(Sign::Positive) && symbol.arguments() == Some(&[]);
        match symbol.name() {
            Some("true") if constant => Ok(Flag(true)),
            Some("false") if constant => Ok(Flag(false)),
            _ => Err(Error::conversion(format!("`{symbol}` is not true or false"))),
        }
    }
}

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Switch {
    #[clingo(constant)]
    name: String,
    on: Flag,
}

let switch = Switch { name: "lamp".into(), on: Flag(true) };
let symbol = switch.to_symbol()?;
assert_eq!(symbol.to_string(), "switch(lamp,true)");
assert_eq!(Switch::from_symbol(symbol)?, switch);
assert_eq!(
    Flag::from_symbol(Symbol::number(1)).unwrap_err().kind(),
    ErrorKind::Conversion,
);
# Ok::<(), clingox::Error>(())
```

## Assuming an atom the grounding does not have

Assumptions follow clingo's C API: an atom that does not occur in the
grounding is false, so assuming it true makes the solve unsatisfiable. The
clingo crate passes the literal it was given, so a port sees the same
behaviour, but pyclingo drops such assumptions and answers satisfiable. If the
program you port depends on that, filter the assumptions with
`SymbolicAtoms::find`, as the documentation of `Assumption` shows:

```rust
use clingox::prelude::*;

let mut ctl = Control::new()?;
ctl.add_base("{a}.")?;
ctl.ground(&[Part::base()])?;
let a = Symbol::function("a", &[])?;
let unknown = Symbol::function("nosuch", &[])?;

// clingo: `nosuch` is false, so assuming it true has no model.
assert!(!ctl.solve(&[(a, true).into(), (unknown, true).into()])?.is_sat());

// Keep only the assumptions on atoms the grounding knows.
let atoms = ctl.symbolic_atoms()?;
let mut known = Vec::new();
for pair in [(a, true), (unknown, true)] {
    if atoms.find(pair.0)?.is_some() {
        known.push(Assumption::from(pair));
    }
}
drop(atoms);
assert!(ctl.solve(&known)?.is_sat());
# Ok::<(), clingox::Error>(())
```

## Differences that come from clingo 5.8

These are properties of clingo 5.8, not of clingox, and they apply to a port
from any binding of 5.6:

- `ast::external` takes its starting truth value as a term (`true`, `false`,
  `free` or `release`), where 5.6 took a number.
- `show_term` and `show_signature` statements lost their CSP flag.
- `clingox::version()` reports 5.8.2 (or the version of the system library
  in a system build).

## Things that are on purpose

Some differences will look like gaps and are not:

- The `SolveResult` returned by `solve`, `get` and `close` is `#[must_use]`,
  because a search can end undecided. Write `let _ =` when you do not care.
- The model printer of a clingo application is supported (see [Clingo
  applications](../how-to/clingo-applications.md)).

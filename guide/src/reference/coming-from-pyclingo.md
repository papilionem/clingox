# Coming from pyclingo

pyclingo is clingo's own Python module, and the mirror layer of clingox follows
its names: if you know `Control.add`, `ground`, `solve` and `assign_external`,
you will find the same steps here. This page lists the calls that differ and
the differences in behaviour that a port has to expect. It was checked against
pyclingo 5.8.2, the same clingo version clingox binds.

## The same program in both

In Python:

```text
ctl = clingo.Control(["--models=0"])
ctl.add("base", [], "{a;b}.")
ctl.ground([("base", [])])
models = []
ctl.solve(on_model=lambda m: models.append(str(m)))
```

In clingox:

```rust
use clingox::prelude::*;

let mut ctl = Control::with_args(["--models=0"])?;
ctl.add("base", &[], "{a;b}.")?;
ctl.ground(&[Part::base()])?;
let mut models = Vec::new();
ctl.for_each_model(&[], |model| {
    models.push(model.to_string());
    Ok(ControlFlow::Continue(()))
})?;
assert_eq!(models.len(), 4);
# Ok::<(), clingox::Error>(())
```

## API mapping

| pyclingo | clingox |
|---|---|
| `Control(args, logger=f, message_limit=n)` | `Control::with_args(args)`, or `Control::builder().args(..).logger(f).message_limit(n).build()` |
| `ctl.add("p", ["x"], text)` | `ctl.add("p", &["x"], text)`; `ctl.add_base(text)` |
| `ctl.load(path)` | `ctl.load(path)` |
| `ctl.ground([("p", [Number(1)])])` | `ctl.ground(&[Part::new("p", &[Symbol::number(1)])?])`; `ctl.ground_with(parts, f)` when the program calls `@f(..)` |
| `ctl.solve(on_model=f)` | `ctl.for_each_model(&[], f)`; `ctl.solve(&[])` when no model is needed |
| `ctl.solve(assumptions=[(a, True)])` | `ctl.solve(&[(a, true).into()])` (mind the [unknown-atom rule](#assumptions-on-atoms-the-grounding-lacks)) |
| `with ctl.solve(yield_=True) as h: for m in h` | `let mut h = ctl.solve_yield(&[])?; while let Some(m) = h.next_model()? { .. }`; `h.close()?` |
| `ctl.solve(async_=True)` | `ctl.solve_async(&[])`; `wait(timeout)`, `get`, `cancel`, `close` |
| `ctl.solve(on_finish=f, on_statistics=g)` | `ctl.solve_with_events(options, handler)`; a `SolveEventHandler` has one method per event |
| `ctl.interrupt()` | `ctl.interrupt_handle().interrupt()` from any thread |
| `SolveHandle.get()` and its `satisfiable` | `handle.get()?`, a `SolveResult` with `is_sat`, `is_unsat`, `is_unknown`, `is_exhausted`, `is_interrupted` |
| first model, optimum, all models | `solve_first`, `solve_optimal`, `solve_all` (owned results) |
| `ctl.assign_external(sym, True)` | `ctl.assign_external(sym, TruthValue::True)`; `release_external(sym)` |
| `ctl.get_const(name)` | `ctl.get_const(name)?`, an `Option<Symbol>` |
| `ctl.cleanup()` | `ctl.cleanup()` |
| `ctl.use_enumeration_assumption = b` | `ctl.set_enable_enumeration_assumption(b)?` |
| `ctl.configuration.solve.models = "0"` | `ctl.configuration().set("solve.models", "0")?` ([Configure the solver](../how-to/configuration.md)) |
| `ctl.statistics["summary"]["models"]["enumerated"]` | `ctl.statistics()?.value("summary.models.enumerated")?`, or `.snapshot()?` for a tree |
| `ctl.symbolic_atoms` | `ctl.symbolic_atoms()?` with `iter`, `by_signature`, `find`, `of::<T>()` |
| `ctl.theory_atoms` | `ctl.theory_atoms()?` |
| `with ctl.backend() as b:` | `ctl.with_backend(\|b\| { .. })?` |
| `ctl.register_propagator(p)` | `ctl.register_propagator(p)?`, with `&self` methods ([Write a propagator](../how-to/write-a-propagator.md)) |
| `ctl.register_observer(o, replace=b)` | `ctl.register_observer(o, replace)?` ([Observing the ground program](../concepts/observer.md)) |
| `ctl.builder` / `ProgramBuilder` | `ctl.with_program_builder(\|b\| { .. })?` ([Syntax trees](../concepts/syntax-trees.md)) |
| `ast.parse_string(text, callback)` | `ast::parse_string(text, callback)?` |
| `ast.Transformer`, `ast.Visitor` | `ast::Visitor`; a rewrite copies the nodes it changes |
| `Function("f", [Number(1)])` | `Symbol::function("f", &[Symbol::number(1)])?` |
| `Function("f", [], False)` | `Symbol::function_with_sign("f", &[], Sign::Negative)?` |
| `String("s")`, `Number(1)`, `Tuple_([..])` | `Symbol::string("s")?`, `Symbol::number(1)`, `Symbol::tuple(&[..])?` |
| `Infimum`, `Supremum` | `Symbol::infimum()`, `Symbol::supremum()` |
| `parse_term("f(1)")` | `"f(1)".parse::<Symbol>()?`, or `sym!(f(1))` checked at compile time |
| `sym.type`, `sym.number`, `sym.name`, `sym.arguments` | `sym.kind()`, `sym.as_number()`, `sym.name()`, `sym.arguments()`; the last three return `Option`s |
| `model.symbols(shown=True)` | `model.symbols(ShowType::SHOWN)?`; `ShowType::ATOMS`, `TERMS`, `COMPLEMENT` combine with `\|` |
| `model.contains(sym)`, `model.is_true(lit)` | the same names, returning `Result<bool>` |
| `model.cost`, `model.number`, `model.type`, `model.optimality_proven` | `cost()?`, `number()`, `kind()?`, `optimality_proven()?` |
| `model.extend(symbols)` | `ExtendableModel::extend(symbols)?` |
| `clingo.Application`, `clingo_main` | `application::Application` ([Clingo applications](../how-to/clingo-applications.md)) |
| `#script (python)` blocks | not available; write the function in Rust with `Script` or `ground_with` (see below) |

## Differences to expect

### Errors, not exceptions

pyclingo raises `RuntimeError` for every failure. clingox returns a
`clingox::Error` with an `ErrorKind` to match on: `Parse` for a syntax error,
`Runtime` for a grounding failure, `Logic` for wrong use of the API, and the
kinds clingox adds itself (`Callback`, `Conversion`, `Nul`, `Poisoned` and
others). A syntax error also carries the position clingo logged. See [Errors
and panics](../concepts/errors-and-panics.md).

A panic in one of your callbacks does not cross into clingo. clingox catches
it, lets clingo unwind, and resumes the panic on the calling thread when the
call returns.

### Poisoning

pyclingo lets you carry on after most failures and find out on the next call
that clingo's state is broken. clingox names the unrecoverable cases. After
one, the `Control` is poisoned: every later call returns
`ErrorKind::Poisoned` with the cause, and the only useful thing to do is drop
the control. A syntax error in the program text you add poisons it, because
clingo cannot ground after a failed parse; so does a `Logic`, `BadAlloc` or
`Unknown` error, and an error or panic that stops `ground_with` partway.
Errors clingox detects before calling clingo (`Nul`, `Conversion`,
`InvalidInput`) and errors after grounding, such as one from the closure of
`for_each_model`, do not. A server that takes program text from users creates
a `Control` per request. The full list is in [Errors and
panics](../concepts/errors-and-panics.md).

### Owned values

Python objects keep clingo memory alive. clingox returns owned values where a
borrow would restrict you: `Configuration::get` gives a `String`,
`Model::symbols` a `Vec<Symbol>`, an AST accessor a new handle. A `Symbol` is
a small `Copy` value that compares and hashes like clingo's symbols do, and
never dangles.

What stays borrowed is what clingo itself invalidates: a `Model` is valid
until the search moves on, so `next_model` lends it and the borrow checker
stops you from keeping it. `Model::snapshot` copies one into an `OwnedModel`
when you need it later, and `solve_first`, `solve_optimal` and `solve_all`
return owned models directly.

### The application control is branded

In pyclingo, `Application.main` receives a `Control` that you could store, and
using it after `main` returns is undefined. In clingox the control passed to
`Application::main` is a `ScopedControl<'r>` for a lifetime `'r` that exists
only inside the call. The compiler refuses to let the control, or anything
borrowed from it, leave. A function that should accept both kinds of control
takes `&mut ScopedControl<'_>`; a `Control` you created is a
`ScopedControl<'static>`. See [Clingo applications](../how-to/clingo-applications.md).

### Assumptions on atoms the grounding lacks

pyclingo drops an assumption on an atom that is not in the grounding, so
`solve(assumptions=[(unknown, True)])` is satisfiable. clingo's C API, and so
clingox, treats such an atom as false: assuming it true makes the program
unsatisfiable. A port that relies on the Python behaviour filters with
`SymbolicAtoms::find`; the recipe is in [Coming from the clingo
crate](coming-from-clingo-crate.md#assuming-an-atom-the-grounding-does-not-have).

### Interrupted searches are unknown

An interrupted or timed-out search is not an error and never says
"unsatisfiable". Its `SolveResult` has `is_interrupted()` true and `is_sat()`
and `is_unsat()` false, and `Outcome::Unknown` separates it from
`Outcome::Unsat`. clingo itself can report an interrupted search as
unsatisfiable and exhausted, and clingox corrects that (see [Known issues in
clingo](known-issues.md)).

### No Python scripting

clingox is built without clingo's Python and Lua support, so a
`#script (python)` or `#script (lua)` block is a `Parse` error ("python
support not available"), which poisons the control like any syntax error. There are two replacements. To give a
program a Rust function to call as `@f(..)`, pass a closure to `ground_with`.
To support a whole language, or blocks that run code at parse time, implement
`Script` and register it once per process ([Custom scripting
languages](../how-to/custom-scripting-languages.md)). Lua is not bound.

### Small ones

- Configuration values are text, as in Python, but you address them by path
  string, not by attribute: `"solver.0.seed"`, not `solver[0].seed`.
- `Symbol` ordering follows clingo's total order, as in Python. A `Symbol` from
  text with arithmetic, such as `"p(1+2)"`, is evaluated to `p(3)`; `sym!`
  does not evaluate.
- Integers must fit `i32`, as in clingo. A Rust `u32` or `i64` outside the
  range is a `Conversion` error when it becomes a symbol, not a wrap.
- Threads are checked at compile time. A `Control` is `Send` but not `Sync`:
  it can move to another thread between calls, and two threads never use it
  at once. The views that borrow it (`Model`, `Statistics`, `SymbolicAtoms`,
  `Configuration`) stay on their thread. The interrupt handle is the one thing
  meant to be shared.

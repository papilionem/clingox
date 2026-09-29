# Solving step by step

`solve_all` and `solve_first` run a search to its end and hand back copies of the
models. This chapter controls the search itself: taking models one at a time,
stopping early, reading the result, and changing the program between searches.

## One model at a time

`solve_yield` starts a search and returns a `SolveHandle`. Each call to
`next_model` runs the search until it finds the next model and lends it to you:

```rust
use clingox::prelude::*;

fn main() -> clingox::Result<()> {
    let mut ctl = Control::with_args(["--models=0"])?;
    ctl.add_base("{ a; b }.")?;
    ctl.ground(&[Part::base()])?;

    let mut handle = ctl.solve_yield(&[])?;
    let mut count = 0;
    while let Some(model) = handle.next_model()? {
        println!("{model}");
        count += 1;
    }
    let result = handle.close()?;
    println!("{result}");
#   assert_eq!(count, 4);
#   assert!(result.is_sat() && result.is_exhausted());
    Ok(())
}
```

```console
$ cargo run
Answer 1:
Answer 2: b
Answer 3: a
Answer 4: a b
SATISFIABLE
```

The program `{ a; b }.` lets each atom be in or out, so it has four answer sets.
`--models=0` asks for all of them; clingo's default is one. The order of the models
is the order the search finds them.

A lent model is valid until the next call on the handle. The compiler enforces this:
code that keeps a `&Model` across `next_model` does not compile. To keep a model,
copy it with `model.snapshot()?`, which returns an `OwnedModel`.

The handle borrows the control, so the program cannot change while the search runs.
`close` ends the search and returns its result. Dropping the handle ends it too.

A model also lends `context()`, a `SolveControl` for the rest of the current
search: `context().add_clause(..)` rules out later models sharing a clause's
literals, without registering a propagator at all. It works the same way no
matter how the model reached you: from this `next_model` loop, from
`for_each_model`, or from a `SolveEventHandler`'s `on_model`, since it is the
same underlying object every time, only reinterpreted. See [Write a
propagator](../how-to/write-a-propagator.md) for the fuller, propagator-based
way to add clauses during search, of which `SolveControl` is the simpler,
propagator-free cousin.

## Stop when you have enough

`for_each_model` runs a closure on each model. The closure returns
`ControlFlow::Break(())` to stop the search:

```rust
use clingox::prelude::*;

#[derive(FromSymbol, Debug)]
struct Pick(i32);

fn main() -> clingox::Result<()> {
    let mut ctl = Control::with_args(["--models=0"])?;
    ctl.add_base("{ pick(1..5) } = 2.")?;
    ctl.ground(&[Part::base()])?;

    let mut found = None;
    let result = ctl.for_each_model(&[], |model| {
        let picks = model.atoms::<Pick>()?;
        let sum: i32 = picks.iter().map(|p| p.0).sum();
        if sum == 7 {
            found = Some(picks);
            return Ok(ControlFlow::Break(()));
        }
        Ok(ControlFlow::Continue(()))
    })?;
    println!("{found:?}");
    println!("interrupted: {}", result.is_interrupted());
#   let picks = found.expect("two numbers from 1 to 5 add up to 7");
#   assert_eq!(picks.iter().map(|p| p.0).sum::<i32>(), 7);
#   assert!(result.is_interrupted() && result.is_sat());
    Ok(())
}
```

The closure runs on your thread, during the search, and may borrow local variables
such as `found`. The `?` inside it returns an error from the closure, which stops the
search and comes back from `for_each_model` unchanged. Which pair it finds first
depends on the search; one run prints:

```console
$ cargo run
Some([Pick(3), Pick(4)])
interrupted: true
```

## What the result says

A `SolveResult` describes how the search ended:

| Method | True when |
|---|---|
| `is_sat()` | at least one model was found |
| `is_unsat()` | the program was proven to have no model |
| `is_unknown()` | neither: the search stopped before it could tell |
| `is_exhausted()` | the whole search space was explored |
| `is_interrupted()` | the search was stopped: by `Break`, a timeout, or an interrupt |

It prints as clingo does: `SATISFIABLE`, `UNSATISFIABLE` or `UNKNOWN`. A search
stopped by `Break` above is satisfiable and interrupted, and not exhausted: models
may remain. An interrupted search is never reported as unsatisfiable, since it could
have missed a model.

## Change the program between searches

A control can solve, change its program, and solve again. clingo calls this
multi-shot solving. Two tools change the program:

- An **external** atom, declared with `#external`, is an atom whose truth you set
  from Rust with `assign_external`. It stays false until you assign it.
- A new **program part**, added with `add` and grounded later, extends the program.
  Parts can take parameters, which `ground` fills in.

```rust
use clingox::prelude::*;

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base("#external raining. wet :- raining.")?;
    ctl.add("hour", &["h"], "alarm(h) :- wet.")?;
    ctl.ground(&[Part::base()])?;

    let (_, models) = ctl.solve_all()?;
    println!("{}", models[0]);
#   assert_eq!(models[0].to_string(), "Answer 1:");

    ctl.assign_external(sym!(raining)?, TruthValue::True)?;
    let (_, models) = ctl.solve_all()?;
    println!("{}", models[0]);
#   assert_eq!(models[0].to_string(), "Answer 1: raining wet");

    ctl.ground(&[Part::new("hour", &[Symbol::number(9)])?])?;
    let (_, models) = ctl.solve_all()?;
    println!("{}", models[0]);
#   assert_eq!(models[0].to_string(), "Answer 1: raining wet alarm(9)");
    Ok(())
}
```

```console
$ cargo run
Answer 1:
Answer 1: raining wet
Answer 1: raining wet alarm(9)
```

The first search has an empty model, because `raining` is false. After
`assign_external`, the same grounding gives `raining` and `wet`. The part `hour` is
then grounded with `h = 9`, and its rule sees the atoms of `base`, which was grounded
before it.

An external keeps its value across searches until you assign it again.
`TruthValue::Free` lets the solver choose, and `release_external` makes it false for
good. Ground a part once per parameter value: grounding `hour(10)` next adds the
rules for hour 10 beside those for hour 9.

## Assume without changing the program

An assumption fixes an atom for one `solve` call and leaves the program alone.
Pass `(Symbol, bool)` pairs, or program literals, as `Assumption`s:

```rust
use clingox::prelude::*;

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base("{a;b}. :- a, b.")?;
    ctl.ground(&[Part::base()])?;

    let a = Symbol::function("a", &[])?;
    let b = Symbol::function("b", &[])?;
    assert!(ctl.solve(&[(a, true).into()])?.is_sat());
    assert!(!ctl.solve(&[(a, true).into(), (b, true).into()])?.is_sat());

    // An atom the grounding does not have counts as false.
    let unknown = Symbol::function("nosuch", &[])?;
    assert!(!ctl.solve(&[(unknown, true).into()])?.is_sat());
    assert!(ctl.solve(&[(unknown, false).into()])?.is_sat());
    Ok(())
}
```

The last two lines are the one place where clingo, and so clingox, differs from
pyclingo. clingo's C API treats an atom that is not in the grounding as false:
assuming it true makes the problem unsatisfiable. pyclingo silently drops such
an assumption, so the same call there is satisfiable. If you port code that
relies on that, filter the pairs with `symbolic_atoms()?.find(symbol)?`, as the
example on [`Assumption`](https://docs.rs/clingox/latest/clingox/struct.Assumption.html) shows, before you solve.

## Tidying up between searches

A few methods only make sense across several `ground`/`solve` calls, since a
single-shot program never needs them.

`update_project` changes which atoms `--project` enumerates over, without
restarting the control:

```rust
use clingox::prelude::*;

fn main() -> clingox::Result<()> {
    let mut ctl = Control::with_args(["--project", "--models=0"])?;
    ctl.add_base("{a}. {b}. {c}.")?;
    ctl.ground(&[Part::base()])?;

    let (_, all) = ctl.solve_all()?;
    println!("{} models", all.len());
#   assert_eq!(all.len(), 8);

    ctl.update_project([sym!(a)?], false)?;
    let (_, projected) = ctl.solve_all()?;
    println!("{} models projected onto a", projected.len());
#   assert_eq!(projected.len(), 2);
    Ok(())
}
```

```console
$ cargo run
8 models
2 models projected onto a
```

`false` replaces the projection atoms; `true` adds to them instead.

`remove_minimize` drops every `:~` statement added so far, so a later
`solve_optimal` no longer optimises:

```rust
use clingox::prelude::*;

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base("{a}. :~ a. [1]")?;
    ctl.ground(&[Part::base()])?;

    ctl.remove_minimize()?;
    let Outcome::Sat(model, _) = ctl.solve_optimal()? else {
        panic!("the program has a model");
    };
    println!("cost: {:?}", model.cost());
#   assert!(model.cost().is_empty());
    Ok(())
}
```

```console
$ cargo run
cost: []
```

clingo cleans up the grounding automatically after each solve call
(`enable_cleanup() == true` by default): atoms known false are dropped and
atoms known true become facts, which keeps later grounding steps smaller.
`set_enable_cleanup(false)` turns this off, for example to inspect a model's
context right after solving before anything is simplified away; `cleanup`
then runs it manually, typically right before grounding a further part.

# The two layers

clingox offers two ways to talk to clingo, in one crate. This chapter explains what
each is for, how they relate, and when to use which.

## The mirror layer

The mirror layer follows clingo's own API. Each clingo concept has one Rust method,
named as in clingo's Python module: `add`, `ground`, `solve`, `assign_external`,
`symbolic_atoms`, `configuration`. Symbols are built with `Symbol::function`,
`Symbol::number` and `Symbol::string`, and read with `name`, `arguments` and
`as_number`. Models give their symbols as a list, selected by `ShowType`.

This layer exists so that clingo's documentation applies. If you know how to do
something with clingo in Python or C, the same steps work here, with Rust types. It
also gives full control: nothing is decided for you.

Rust conventions still shape its form. Failure is a `Result`, "not this kind of
symbol" is an `Option`, constructors are `new` and `builder`, and the borrow checker
enforces clingo's rules on when a model or a handle may be used.

## The typed layer

The typed layer adds what Rust makes possible on top of the mirror layer:

- Rust types as facts and results, through `#[derive(ToSymbol)]`,
  `#[derive(FromSymbol)]`, `add_facts`, and `atoms` and `shown` on a model;
- symbols written in clingo's syntax and checked at compile time, with `sym!`;
- owned results and explicit outcomes: `solve_first`, `solve_optimal` and
  `solve_all` return copies that outlive the search, and `Outcome` separates "no
  answer set" from "the search did not finish";
- helpers for testing rules, in `clingox::testing`.

Most application code needs only this layer. It removes the code that turns Rust
data into symbols and back, and the mistakes that code invites: a string where the
rules expect a constant, a number that wraps, an atom that fails to convert and is
silently skipped.

## One built on the other

The typed layer never calls clingo directly. Every typed call is a sequence of mirror
calls that you could write yourself:

| Typed call | Mirror calls it makes |
|---|---|
| `add_facts(values)` | `ToSymbol::to_symbol` on each value, `add` of a fresh part `__clingox_facts_<n>` with one `symbol.` per fact, then `ground` of that part |
| `model.atoms::<T>()` | `model.symbols(ShowType::ATOMS)`, kept where the sign is positive, the name is `T::NAME` and the arity `T::ARITY` (so `-p(1)` is not read as `p/1`), sorted, each through `T::from_symbol` |
| `model.shown::<T>()` | the same with `ShowType::SHOWN` |
| `sym!(p(1, "x", {n}))` | `Symbol::function`, `Symbol::number`, `Symbol::string`, `Symbol::tuple`, and `to_symbol` for each `{..}` |
| `#[derive(ToSymbol)]` | `Symbol::function` with the fields' symbols |
| `#[derive(FromSymbol)]` | `Symbol::name`, `arguments` and `sign`, then each field's `from_symbol` |
| `for_each_model(f)` | `solve_yield`, then `next_model` until `f` breaks, then `cancel` and `close` |
| `solve_first()` | `solve_yield`, one `next_model`, `Model::snapshot`, `close` |
| `solve_optimal()` | the option `solve.models` set to `-1`, clingo's default, then `solve_yield`, every `next_model`, and the last model; the option is restored |
| `solve_all()` | the option `solve.models` set to `0`, which lifts the limit, then `solve_yield` and a snapshot of every model, sorted; the option is restored |
| `assert_models!(models, lines)` | `OwnedModel::symbols`, and `str::parse::<Symbol>` on each term of each line |

Because of this, the two layers mix freely. A program can add facts with
`add_facts`, then ground a parameterised part with `ground`, set an option through
`configuration`, and read the results with `atoms`. A typed call leaves the control in
the state its mirror calls would.

When a convenience hides a clingo subtlety, its documentation says what it does
underneath. `add_facts` is the main case: it grounds its own part at once, so rules
grounded earlier do not see the facts. The chapter
[Facts and rules](../tutorial/facts-and-rules.md) shows the consequence.

## When to use the mirror layer

Reach for the mirror layer when the typed layer does not express what you need:

- **Terms the derives do not describe.** A predicate whose arguments vary in shape
  can be read as `Symbol`s from `model.symbols(..)`, or through a field of type
  `Symbol`, which accepts any symbol.
- **Symbols from text known only at run time.** `str::parse::<Symbol>()` uses
  clingo's own parser and evaluates arithmetic: `"p(1+2)"` gives `p(3)`. `sym!`
  checks its term at compile time, and accepts no arithmetic.
- **Control over parts and grounding.** Parameterised parts, grounding several parts
  at once, and adding facts to a part of your choosing are all `add` and `ground`.
- **Search control.** Assumptions, `solve_yield` with `get` and `cancel`, time
  budgets with `solve_with`, and configuration all live on `Control` and
  `SolveHandle`.

The layers are not separate modules: both are methods on the same types, and the
prelude brings in both.

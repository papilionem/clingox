# Test your rules

A logic program is code, and a change to one rule can remove answer sets you rely on
or add ones you never meant to allow. This page shows how to test rules with
`cargo test`: compare every answer set with `assert_models!`, check typed results,
test optimisation and programs without answer sets, and test that bad input is
rejected. The helpers live in
[`clingox::testing`](https://docs.rs/clingox/latest/clingox/testing/index.html).

## Compare every answer set

`assert_models!` takes the models of a search and the answer sets you expect, one
string per answer set, written as clingo prints them:

```rust,test_harness
use clingox::testing::assert_models;
use clingox::{Control, Part};

const RULES: &str = "
    { on(L) } :- lamp(L).
    :- on(L), broken(L).
    #show on/1.";

#[test]
fn only_working_lamps_turn_on() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base(RULES)?;
    ctl.add_base("lamp(a). lamp(b). lamp(c). broken(b).")?;
    ctl.ground(&[Part::base()])?;
    let (_, models) = ctl.solve_all()?;
    assert_models!(models, ["", "on(a)", "on(c)", "on(a) on(c)"]);
    Ok(())
}
```

The comparison works on the shown symbols of each model (`OwnedModel::symbols`), so
`#show` decides what you write. It ignores the order of the answer sets and the order
of the symbols in each, because clingo may find them in any order. It does count
them: `["a", "a"]` does not match a single model `a`. An empty string is the empty
answer set, and `[]` expects no answer set at all.

`solve_all` enumerates every answer set, whatever the control's `--models` option
says, and returns them sorted. Each expected string is parsed with clingo's own
parser, so `on(a)` and `on( a )` are the same symbol.

## Read a failure

On a mismatch the macro panics with the line of the call and a message that lists
what is missing and what was not expected. With the facts
`lamp(a). lamp(b). broken(b).`, the rules above have two answer sets, the empty
answer set and `on(a)`, so expecting `["on(b)", ""]` fails with:

```text
models differ (expected 2, found 2)
missing: on(b)
unexpected: on(a)
```

Each missing or unexpected answer set is one line, its symbols sorted, and an empty
answer set reads `(empty)`. An expected string that does not parse panics too, and
names the string.

## Test a program without answer sets

For a program that must have no answer set, expect none, and check the result:

```rust,test_harness
use clingox::testing::assert_models;
use clingox::{Control, Part};

#[test]
fn a_broken_lamp_cannot_be_required() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base(
        "{ on(L) } :- lamp(L).
         :- on(L), broken(L).
         :- not on(b).
         lamp(b). broken(b).",
    )?;
    ctl.ground(&[Part::base()])?;
    let (result, models) = ctl.solve_all()?;
    assert!(result.is_unsat());
    assert_models!(models, []);
    Ok(())
}
```

`is_unsat` is true only for a search that ran to its end. An interrupted search is
never reported as unsatisfiable.

## Build the instance in Rust

Most tests vary the facts and keep the rules. A function that takes the instance as
Rust values keeps each test short. `add_facts` and the derives from
[Facts and rules](../tutorial/facts-and-rules.md) do the conversion, and
`OwnedModel::atoms` reads typed results:

```rust,test_harness
use clingox::prelude::*;

#[derive(ToSymbol)]
struct Lamp(#[clingo(constant)] String);

#[derive(ToSymbol)]
struct Broken(#[clingo(constant)] String);

#[derive(FromSymbol, Debug, PartialEq)]
struct On(#[clingo(constant)] String);

/// Every answer set of the lamp rules for these lamps, as the lamps that are on.
fn switched_on(lamps: &[&str], broken: &[&str]) -> clingox::Result<Vec<Vec<On>>> {
    let mut ctl = Control::new()?;
    ctl.add_base("{ on(L) } :- lamp(L). :- on(L), broken(L).")?;
    ctl.add_facts(lamps.iter().map(|l| Lamp(l.to_string())))?;
    ctl.add_facts(broken.iter().map(|l| Broken(l.to_string())))?;
    ctl.ground(&[Part::base()])?;
    let (_, models) = ctl.solve_all()?;
    models.iter().map(|model| model.atoms::<On>()).collect()
}

#[test]
fn no_lamp_is_on_when_all_are_broken() -> clingox::Result<()> {
    assert_eq!(switched_on(&["a", "b"], &["a", "b"])?, [vec![]]);
    Ok(())
}

#[test]
fn a_working_lamp_can_be_on_or_off() -> clingox::Result<()> {
    let models = switched_on(&["a"], &[])?;
    assert_eq!(models, [vec![], vec![On("a".to_string())]]);
    Ok(())
}
```

`atoms::<On>` reads every true `on/1` atom, shown or not, sorted, and fails with
`ErrorKind::Conversion` for one that does not fit the type. The order of the models
is the order `solve_all` sorts them in, by their shown symbols, then by all their
atoms.

## Compare one answer set with clingo's output

`testing::parse_answer` turns one line of clingo's output into symbols, in the order
of the line. It helps when the expected answer comes from running `clingo` on the
command line:

```rust
use clingox::testing::parse_answer;
use clingox::{Control, Outcome, Part};

let mut ctl = Control::new()?;
ctl.add_base("lamp(a). lamp(c). on(L) :- lamp(L). #show on/1.")?;
ctl.ground(&[Part::base()])?;
let Outcome::Sat(model, _) = ctl.solve_first()? else {
    panic!("the program has an answer set");
};

// A line copied from the output of `clingo lamps.lp`.
let mut expected = parse_answer("on(c) on(a)")?;
expected.sort();
assert_eq!(model.symbols(), expected);
# Ok::<(), clingox::Error>(())
```

The order of the symbols on clingo's line is not fixed. `OwnedModel::symbols` is
sorted, and `parse_answer` keeps the order of the line, so sort before comparing. Terms are separated by whitespace outside strings and
parentheses, so `p(1, 2) s("a b")` is two terms.

## Test optimisation

`solve_optimal` returns the optimal model with its cost. A test can check the model,
the cost, and that optimality was proven:

```rust,test_harness
use clingox::testing::assert_models;
use clingox::{Control, Outcome, Part};

#[test]
fn the_cheapest_lamp_is_chosen() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base(
        "lamp(a, 3). lamp(b, 1). lamp(c, 2).
         1 { on(L) : lamp(L, _) } 1.
         #minimize { C, L : on(L), lamp(L, C) }.
         #show on/1.",
    )?;
    ctl.ground(&[Part::base()])?;
    let Outcome::Sat(best, result) = ctl.solve_optimal()? else {
        panic!("the program has an answer set");
    };
    assert!(result.is_exhausted() && best.optimality_proven());
    assert_eq!(best.cost(), [1]);
    assert_models!([best], ["on(b)"]);
    Ok(())
}
```

`assert_models!` takes any slice of `OwnedModel`s, here an array of one. When several
models are optimal, `solve_optimal` returns one of them; to test all of them, create
the control with `--opt-mode=optN` and keep the models whose `optimality_proven()` is
true. [Optimisation](../tutorial/optimisation.md) explains the modes.

## Test that bad input is rejected

A syntax error is `ErrorKind::Parse`, and its messages carry the position:

```rust,test_harness
use clingox::{Control, ErrorKind};

#[test]
fn a_missing_comma_is_reported_where_it_is() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    let err = ctl.add_base("on(L) :- lamp(L) broken(L).").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Parse);
    let at = err.messages()[0].location().expect("a syntax error has a position");
    assert_eq!((at.line(), at.column()), (1, 18));
    Ok(())
}
```

The control is poisoned after a syntax error, so each such test creates its own. See
[Error kinds](../reference/error-kinds.md#parse).

## Running the tests

- **One control per test.** `cargo test` runs tests on several threads at once. Each
  test that creates its own `Control` is independent of the others.
- **System clingo, or `threads` off.** With a clingo installed on the system, or with
  the `threads` feature off, controls must not solve on several threads at once (see
  [Known issues](../reference/known-issues.md#fixed-in-the-vendored-build-open-with-a-system-clingo)).
  Run such a suite with `cargo test -- --test-threads=1`. The default build needs
  nothing.
- **Order.** The default control has one solver thread and a fixed seed, so a search
  repeats exactly. With several solver threads models come in a different order on
  each run; `solve_all` and `assert_models!` do not depend on it.
- **Other targets.** The same tests run on Android and WebAssembly with `cargo test
  --target`, given a runner for the target; see
  [Run in the browser](browser.md) and [Run on Android](android.md). A test that needs
  threads, such as one with a timeout, has to allow for `ErrorKind::Unsupported` on
  the default WebAssembly build.

# Reading results as Rust types

The [previous chapter](facts-and-rules.md) turned Rust values into facts. This one goes
the other way: it reads the atoms of a model back as Rust values.

## Derive `FromSymbol`

Describe the atoms you want to read as a Rust type, and derive `FromSymbol`. The
mapping is the one `ToSymbol` uses, so a type can derive both:

```rust
use clingox::prelude::*;

#[derive(ToSymbol)]
struct Edge(i32, i32);

#[derive(FromSymbol, Debug, PartialEq)]
struct Reach(i32, i32);

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base(
        "reach(X, Y) :- edge(X, Y).
         reach(X, Z) :- reach(X, Y), edge(Y, Z).",
    )?;
    ctl.add_facts([Edge(1, 2), Edge(2, 3)])?;
    ctl.ground(&[Part::base()])?;

    let Outcome::Sat(model, _) = ctl.solve_first()? else {
        panic!("the program has an answer set");
    };
    let reach: Vec<Reach> = model.atoms()?;
    println!("{reach:?}");
#   assert_eq!(reach, [Reach(1, 2), Reach(1, 3), Reach(2, 3)]);
    Ok(())
}
```

```console
$ cargo run
[Reach(1, 2), Reach(1, 3), Reach(2, 3)]
```

`atoms::<Reach>()` selects the atoms of the predicate `reach/2`, the name and the
number of fields of `Reach`, and converts each one. Other predicates are skipped.
The values come sorted in clingo's order of symbols, so the result does not depend
on the search.

## `atoms` and `shown`

`#show` statements select what clingo prints for a model. `#show.` hides every atom,
and `#show route(X, Y) : reach(X, Y), X = 1.` shows the term `route(X, Y)` for each
match of its condition. clingox gives you both views:

- `atoms::<T>()` reads every atom that is true in the model, whether it is shown or
  not. A `#show` added elsewhere in the program cannot hide atoms from it.
- `shown::<T>()` reads the shown symbols only. They include shown terms that are not
  atoms, such as `route(1,3)` below.

```rust
use clingox::prelude::*;

#[derive(FromSymbol, Debug, PartialEq)]
struct Reach(i32, i32);

#[derive(FromSymbol, Debug, PartialEq)]
struct Route(i32, i32);

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base(
        "edge(1, 2). edge(2, 3).
         reach(X, Y) :- edge(X, Y).
         reach(X, Z) :- reach(X, Y), edge(Y, Z).
         #show.
         #show route(X, Y) : reach(X, Y), X = 1.",
    )?;
    ctl.ground(&[Part::base()])?;

    let Outcome::Sat(model, _) = ctl.solve_first()? else {
        panic!("the program has an answer set");
    };
    println!("{model}");
    println!("atoms: {:?}", model.atoms::<Reach>()?);
    println!("shown: {:?}", model.shown::<Reach>()?);
    println!("routes: {:?}", model.shown::<Route>()?);
#   assert_eq!(model.to_string(), "Answer 1: route(1,2) route(1,3)");
#   assert_eq!(model.atoms::<Reach>()?.len(), 3);
#   assert!(model.shown::<Reach>()?.is_empty());
#   assert_eq!(model.shown::<Route>()?, [Route(1, 2), Route(1, 3)]);
#   assert!(model.atoms::<Route>()?.is_empty());
    Ok(())
}
```

```console
$ cargo run
Answer 1: route(1,2) route(1,3)
atoms: [Reach(1, 2), Reach(1, 3), Reach(2, 3)]
shown: []
routes: [Route(1, 2), Route(1, 3)]
```

Read with `atoms` what the program derives, and with `shown` what it chooses to
report.

## Match the outcome

`solve_first` returns an `Outcome` with three cases. `Outcome` is not
`#[non_exhaustive]`, so the compiler reports a missing case, and a `match` handles
each one without a wildcard:

```rust
use clingox::prelude::*;

#[derive(FromSymbol, Debug)]
struct Assign(i32, i32);

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    // Two tasks, one worker: every task needs a worker of its own.
    ctl.add_base(
        "task(1..2). worker(1).
         { assign(T, W) : worker(W) } = 1 :- task(T).
         :- assign(T1, W), assign(T2, W), T1 < T2.",
    )?;
    ctl.ground(&[Part::base()])?;

    match ctl.solve_first()? {
        Outcome::Sat(model, _) => println!("{:?}", model.atoms::<Assign>()?),
        Outcome::Unsat => println!("no assignment exists"),
        Outcome::Unknown(result) => println!("the search stopped undecided: {result:?}"),
    }
#   assert!(matches!(ctl.solve_first()?, Outcome::Unsat));
    Ok(())
}
```

```console
$ cargo run
no assignment exists
```

`Outcome::Unsat` is a proof that there is no answer set. `Outcome::Unknown` means
the search stopped before it could decide, for example at a time limit. Keeping the
two apart matters: "no model found in time" is not "no model exists".

## How types are named

A derived type reads the predicate named after it: the type's name in snake case,
unless `#[clingo(name = "..")]` on the type says otherwise. Snake case puts `_` before
each uppercase letter that starts a word, lowercases everything, and keeps leading
underscores:

| Rust name | Predicate |
|---|---|
| `Reach` | `reach` |
| `TaskAssignment` | `task_assignment` |
| `HTTPCheck` | `http_check` |
| `Vec2` | `vec2` |
| `Point3D` | `point3_d` |
| `_Hidden` | `_hidden` |

A letter after a digit starts a new word, which is why `Point3D` reads `point3_d`.
When the program uses another name, say so on the type:

```rust
use clingox::prelude::*;

#[derive(FromSymbol, Debug, PartialEq)]
#[clingo(name = "point3d")]
struct Point3D(i32, i32, i32);

fn main() -> clingox::Result<()> {
    let point = Point3D::from_symbol("point3d(1,2,3)".parse()?)?;
    println!("{point:?}");
#   assert_eq!(point, Point3D(1, 2, 3));
#   assert_eq!(<Point3D as Predicate>::NAME, "point3d");
    Ok(())
}
```

## A mismatch is an error

When an atom has the right name and arity but its arguments do not convert, reading
fails with an error of kind `Conversion`. clingox never skips such an atom, because a
skipped atom looks the same as a missing one:

```rust
use clingox::prelude::*;
use clingox::ErrorKind;

#[derive(FromSymbol, Debug)]
struct Score(#[clingo(constant)] String, i32);

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base("score(alice, 3). score(bob, high).")?;
    ctl.ground(&[Part::base()])?;

    let Outcome::Sat(model, _) = ctl.solve_first()? else {
        panic!("the program has an answer set");
    };
    let err = model.atoms::<Score>().unwrap_err();
    println!("{err}");
#   assert_eq!(err.kind(), ErrorKind::Conversion);
#   assert_eq!(
#       err.to_string(),
#       "reading `score(bob,high)` as `Score`, field `1`: `high` is not a number"
#   );
    Ok(())
}
```

```console
$ cargo run
reading `score(bob,high)` as `Score`, field `1`: `high` is not a number
```

The message names the atom, the Rust type and the field. The same happens when a
`#[clingo(constant)]` field meets a string, or a `#[clingo(string)]` field meets a
constant.

## Enums

An enum converts each of its variants as a term of its own: a unit variant is a
constant, and a variant with fields is a function. Use enums for fields that take one
of several forms:

```rust
use clingox::prelude::*;

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
enum Level {
    Low,
    High,
    Custom(i32),
}

#[derive(FromSymbol, Debug, PartialEq)]
struct Alert {
    #[clingo(constant)]
    host: String,
    level: Level,
}

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base("alert(db, high). alert(web, custom(7)).")?;
    ctl.ground(&[Part::base()])?;

    let Outcome::Sat(model, _) = ctl.solve_first()? else {
        panic!("the program has an answer set");
    };
    for alert in model.atoms::<Alert>()? {
        println!("{}: {:?}", alert.host, alert.level);
    }
#   let alerts = model.atoms::<Alert>()?;
#   assert_eq!(alerts[0].level, Level::High);
#   assert_eq!(alerts[1].level, Level::Custom(7));
    Ok(())
}
```

```console
$ cargo run
db: High
web: Custom(7)
```

The variants are named in snake case (`Low` is `low`), or by
`#[clingo(name = "..")]` on the variant. An enum is not a predicate, because its
variants have different names and arities, so `model.atoms::<Level>()` does not
compile. Read each predicate into a struct of its own.

## Integers

clingo's numbers are 32-bit signed integers. `i32` converts directly. Every other
integer type converts with a range check in both directions, and a value out of range
is a `Conversion` error rather than a silently wrapped number:

```rust
use clingox::prelude::*;
use clingox::ErrorKind;

#[derive(ToSymbol, FromSymbol, Debug)]
struct Stock {
    count: u8,
}

fn main() -> clingox::Result<()> {
    let low: Symbol = "stock(3)".parse()?;
    let high: Symbol = "stock(300)".parse()?;
    println!("{:?}", Stock::from_symbol(low)?);
    println!("{}", Stock::from_symbol(high).unwrap_err());
    println!("{}", u32::MAX.to_symbol().unwrap_err());
#   assert_eq!(Stock::from_symbol(low)?.count, 3);
#   assert_eq!(Stock::from_symbol(high).unwrap_err().kind(), ErrorKind::Conversion);
#   assert_eq!(u32::MAX.to_symbol().unwrap_err().kind(), ErrorKind::Conversion);
    Ok(())
}
```

```console
$ cargo run
Stock { count: 3 }
reading `stock(300)` as `Stock`, field `count`: `300` is not a number within the range of `u8`
the u32 4294967295 is outside the range of clingo numbers (-2147483648 to 2147483647)
```

The check matters because clingo itself wraps an out-of-range number in program
text: it reads `p(2147483648)` as `p(-2147483648)`.

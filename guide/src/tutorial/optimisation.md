# Optimisation

Some problems ask for the best answer set, not any answer set. This chapter states
what "best" means in the program and asks clingo for it.

## A goal in the program

A bag holds at most 6 kilograms. Each item has a weight and a value, and the program
packs items to get the highest value that fits:

```text
item(a, 3, 5). item(b, 4, 5). item(c, 2, 3).

{ pack(I) : item(I, _, _) }.
:- #sum { W, I : pack(I), item(I, W, _) } > 6.
#maximize { V, I : pack(I), item(I, _, V) }.
#show pack/1.
```

`item(a, 3, 5)` reads: item `a` weighs 3 and is worth 5. The choice rule may pack any
item, the constraint rejects a bag heavier than 6, and `#maximize` asks for the
highest sum of values. Two bags reach 8: `a` with `c`, and `b` with `c`. Packing `a`
and `b` together would weigh 7.

`#minimize` asks for the lowest sum instead. A weak constraint states a cost in
another form: `:~ pack(I), item(I, W, _). [W, I]` adds the weight `W` for each packed
item. Both forms can carry a priority, written `@2` after the value; clingo optimises
the highest priority first.

## The optimum

`solve_optimal` searches until clingo has proven that no better model exists:

```rust
use clingox::prelude::*;

const BAG: &str = "
    item(a, 3, 5). item(b, 4, 5). item(c, 2, 3).
    { pack(I) : item(I, _, _) }.
    :- #sum { W, I : pack(I), item(I, W, _) } > 6.
    #maximize { V, I : pack(I), item(I, _, V) }.
    #show pack/1.
";

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base(BAG)?;
    ctl.ground(&[Part::base()])?;

    match ctl.solve_optimal()? {
        Outcome::Sat(best, _) => {
            println!("{best}");
            println!("cost {:?}, proven optimal: {}", best.cost(), best.optimality_proven());
#           assert_eq!(best.cost(), [-8]);
#           assert!(best.optimality_proven());
        }
        Outcome::Unsat => println!("nothing fits"),
        Outcome::Unknown(result) => println!("undecided: {result:?}"),
    }
    Ok(())
}
```

```console
$ cargo run
Answer 4: pack(b) pack(c)
cost [-8], proven optimal: true
```

clingo always minimises, so it turns a maximised sum into a cost by negating it: a
value of 8 is the cost `-8`. `cost()` has one entry per priority level, highest
priority first; here there is one level. Under the default `--opt-mode=opt`,
`optimality_proven()` is true only for the model that ends the search, after clingo
has ruled out anything better. `Answer 4` says that three other models came before
it. The second value in `Outcome::Sat`, ignored here, is the result of the whole
search; for a finished search it is exhausted.

## A time budget

A hard problem may take longer to prove optimal than you can wait.
`solve_optimal_with` takes `SolveOptions` with a timeout for the whole call. When the
timeout stops the search, you get the best model found so far, not proven optimal,
and a result that says the search was interrupted:

```rust
use std::time::Duration;

use clingox::prelude::*;
use clingox::{ErrorKind, SolveOptions};

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base("{ a; b; c }. :~ a. [1] :~ b. [2] :~ not c. [3]")?;
    ctl.ground(&[Part::base()])?;

    let options = SolveOptions::new().timeout(Duration::from_secs(10));
    match ctl.solve_optimal_with(options) {
        Ok(Outcome::Sat(best, result)) if result.is_interrupted() => {
            println!("best so far: {best}");
        }
        Ok(Outcome::Sat(best, _)) => {
            println!("optimal: {best}");
#           assert!(best.optimality_proven());
        }
        Ok(_) => println!("no answer set, or none found in time"),
        Err(err) if err.kind() == ErrorKind::Unsupported => {
            println!("this build of clingo has no threads");
        }
        Err(err) => return Err(err),
    }
    Ok(())
}
```

This small problem is solved long before its budget, so it prints the optimum. A
timeout needs a build of clingo with threads. Without them, as on the default
WebAssembly build, the call returns `ErrorKind::Unsupported` before solving
anything, and the control stays usable.

## Why the first model is not the optimum

`solve_first` stops at the first model the search finds. On an optimisation problem
that is whatever the search reached first, not the best model:

```rust
use clingox::prelude::*;
# const BAG: &str = "
#     item(a, 3, 5). item(b, 4, 5). item(c, 2, 3).
#     { pack(I) : item(I, _, _) }.
#     :- #sum { W, I : pack(I), item(I, W, _) } > 6.
#     #maximize { V, I : pack(I), item(I, _, V) }.
#     #show pack/1.
# ";

fn main() -> clingox::Result<()> {
    let mut ctl = Control::new()?;
    ctl.add_base(BAG)?;
    ctl.ground(&[Part::base()])?;

    let Outcome::Sat(first, _) = ctl.solve_first()? else {
        panic!("an empty bag always fits");
    };
    println!("{first}");
    println!("cost {:?}, proven optimal: {}", first.cost(), first.optimality_proven());
#   assert_eq!(first.cost(), [0]);
#   assert!(!first.optimality_proven());
    Ok(())
}
```

```console
$ cargo run
Answer 1:
cost [0], proven optimal: false
```

The first model is the empty bag. It is a valid answer set, and it is worth nothing.
Use `solve_first` when any solution will do, and `solve_optimal` when the goal
matters.

## Every optimal model

This problem has two optimal bags, and `solve_optimal` returns one of them.
`solve_all` returns every model clingo reports, and what clingo reports depends on
its `--opt-mode` option. With `--opt-mode=optN`, clingo first finds the optimal cost,
then reports every model with that cost, each marked as proven optimal:

```rust
use clingox::prelude::*;
# const BAG: &str = "
#     item(a, 3, 5). item(b, 4, 5). item(c, 2, 3).
#     { pack(I) : item(I, _, _) }.
#     :- #sum { W, I : pack(I), item(I, W, _) } > 6.
#     #maximize { V, I : pack(I), item(I, _, V) }.
#     #show pack/1.
# ";

fn main() -> clingox::Result<()> {
    let mut ctl = Control::with_args(["--opt-mode=optN"])?;
    ctl.add_base(BAG)?;
    ctl.ground(&[Part::base()])?;

    let (_, models) = ctl.solve_all()?;
    for model in models.iter().filter(|m| m.optimality_proven()) {
        println!("{model}");
    }
#   let optimal: Vec<String> = models
#       .iter()
#       .filter(|m| m.optimality_proven())
#       .map(ToString::to_string)
#       .collect();
#   assert_eq!(optimal, ["Answer 2: pack(a) pack(c)", "Answer 1: pack(b) pack(c)"]);
    Ok(())
}
```

```console
$ cargo run
Answer 2: pack(a) pack(c)
Answer 1: pack(b) pack(c)
```

The list also holds the models found on the way to the optimum, which are not proven
optimal, so the loop keeps only the proven ones. clingo numbers the models of the
second phase from 1 again. The default mode, `--opt-mode=opt`, reports only models
that improve on the one before, so it never reports a second model of the same cost.

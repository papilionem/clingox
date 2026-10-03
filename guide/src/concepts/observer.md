# Observing the ground program

[The backend](backend.md) lets a program *write* ground directives directly,
bypassing the grounder. A `GroundProgramObserver` is its read-side
counterpart: it *watches* every ground directive as it reaches the solver,
whichever source produced it (program text, the ground callback, or the
backend itself).

```rust
use clingox::observer::GroundProgramObserver;
use clingox::{Control, Part};

#[derive(Default)]
struct CountRules(u32);

impl GroundProgramObserver for CountRules {
    fn rule(
        &mut self,
        _choice: bool,
        _head: &[clingox::backend::Atom],
        _body: &[clingox::ProgramLiteral],
    ) -> clingox::Result<()> {
        self.0 += 1;
        Ok(())
    }
}

let mut ctl = Control::new()?;
let counter = CountRules::default();
ctl.register_observer(counter, false)?;
ctl.add_base("a. b :- a. c :- a, b.")?;
ctl.ground(&[Part::base()])?;
assert!(ctl.solve(&[])?.is_sat());
# Ok::<(), clingox::Error>(())
```

`GroundProgramObserver` has one method per kind of directive clingo's ground
program observer interface reports: `init_program`, `begin_step`/`end_step`,
`rule`, `weight_rule`, `minimize`, `project`, `output_atom`, `output_term`,
`external`, `assume`, `heuristic`, `acyc_edge`, and the theory-term/element/
atom family. Every method has a default `Ok(())` body, so an observer
implements only the ones it cares about.

## Registering an observer

`Control::register_observer` takes the observer by value and keeps it for as
long as it stays registered. This is different from the ground callback
(`Control::ground_with`), which is borrowed and scoped to one call:
`GroundProgramObserver: Send + 'static`, so it cannot borrow anything local to
one grounding, and it goes on watching every later `ground`/`ground_with`
call on the same control.

clingo has no way to unregister an observer. Calling `register_observer`
again does not replace the first one (unless its own `replace` is `true`); it
composes with it, since that is what clingo's own registration engine does
internally. In practice this means: register an observer once, for the whole
life of the control it watches.

`replace` decides whether the ground program still reaches the solver:

```rust
use clingox::observer::GroundProgramObserver;
use clingox::Control;

struct NoOp;
impl GroundProgramObserver for NoOp {}

let mut ctl = Control::new()?;
ctl.register_observer(NoOp, true)?; // watch, but do not solve
ctl.add_base("a.")?;
ctl.ground(&[clingox::Part::base()])?;
let result = ctl.solve(&[])?;
assert!(!result.is_sat() && !result.is_unsat());
# Ok::<(), clingox::Error>(())
```

With `replace: true`, every callback still fires exactly as it would
otherwise, but nothing reaches clasp: the search reports neither satisfiable
nor unsatisfiable.

## Errors, panics and poisoning

An observer callback that returns `Err` stops grounding at once: no later
callback of any kind fires, and `Control::ground`/`ground_with` returns that
same error, unchanged. A panic is caught and resumes on the calling thread
once the call returns. Both poison the control, the same way a failed ground
callback does (see [Errors and panics](errors-and-panics.md)): clingo keeps
whatever it already ground and would answer a later solve from that truncated
program silently, so the only safe response is to refuse to go on.

## The grounding-size guard

Grounding cannot be interrupted, so clingox has no way to cut off a program
that turns out to ground far more atoms or rules than expected, the way a
solve can be stopped with a timeout. `GroundingLimit` and `LimitedObserver`
build a cooperative approximation on top of the observer: the callback that
pushes a count over its limit fails with `ErrorKind::GroundingLimit` (counting
comes before checking), rather than letting grounding run away. It bounds the
size of the ground program, not grounding time or memory
([Set a time budget](../how-to/time-budget.md#grounding-has-no-time-budget) says what that leaves open).

```rust
use clingox::observer::{GroundingLimit, LimitedObserver};
use clingox::{Control, ErrorKind, Part};

struct NoOp;
impl clingox::observer::GroundProgramObserver for NoOp {}

let mut ctl = Control::new()?;
let limit = GroundingLimit::new(None, Some(1));
ctl.register_observer(LimitedObserver::new(NoOp, limit), false)?;
ctl.add_base("a. b.")?;
let err = ctl.ground(&[Part::base()]).unwrap_err();
assert_eq!(err.kind(), ErrorKind::GroundingLimit);
# Ok::<(), clingox::Error>(())
```

`max_rules` counts one per `rule`/`weight_rule` call; `max_atoms` counts
distinct atom ids seen across every callback that carries one (a rule's head
and body, `output_atom`, an external, and so on), each counted once however
many callbacks mention it. The check happens *after* each callback's own
contribution is added, so the callback that pushes a count over its limit is
refused at once, whether or not clingo ever calls another one after it (a
violation on the very last callback of a grounding step is still caught by
this same call, not left to surface on a later, unrelated one).

`LimitedObserver<O>` is a combinator: it wraps any `GroundProgramObserver`, so
the guard and a caller's own observer see exactly the same calls, up to and
excluding the one that is refused. When no observer of one's own is needed,
`Control::ground_with_limit` is a shorthand for wrapping a no-op observer and
grounding in one call:

```rust
use clingox::observer::GroundingLimit;
use clingox::{Control, ErrorKind, Part};

let mut ctl = Control::new()?;
ctl.add_base("a. b.")?;
let limit = GroundingLimit::new(None, Some(1));
let err = ctl.ground_with_limit(&[Part::base()], limit).unwrap_err();
assert_eq!(err.kind(), ErrorKind::GroundingLimit);
# Ok::<(), clingox::Error>(())
```

Because clingo cannot unregister an observer, the guard `ground_with_limit`
registers stays active for every later grounding on the same control too, not
only the call it was passed to.

## Dumping the ground program to a file

`Control::register_backend_writer` is a narrower, special-purpose relative of
`register_observer`: instead of handing callbacks to Rust code, it tells
clingo to write its own ground output to a file, in `BackendWriterKind::ASPIF`,
`REIFY` or `SMODELS` format. It shares `register_observer`'s registration
engine, so `replace` means the same thing.

```rust
use clingox::backend::BackendWriterKind;
use clingox::{Control, Part};

let path = std::env::temp_dir()
    .join(format!("clingox_guide_observer_{}.aspif", std::process::id()));
let mut ctl = Control::new()?;
ctl.register_backend_writer(BackendWriterKind::ASPIF, &path, false)?;
ctl.add_base("a.")?;
ctl.ground(&[Part::base()])?;
assert!(ctl.solve(&[])?.is_sat());
assert!(std::fs::read_to_string(&path).unwrap().starts_with("asp 1 0 0"));
std::fs::remove_file(&path).ok();
# Ok::<(), clingox::Error>(())
```

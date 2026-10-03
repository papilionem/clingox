# Set a time budget

A search can take longer than you can wait: proving that 11 pigeons do not fit into
10 holes takes clasp far longer than any of the examples below allow. This page shows
the ways to bound a search: a timeout for one call, an interrupt from another thread,
a search in the background that you poll, a limit on conflicts that does not depend
on the clock, and a deadline checked between models. It ends with the work none of
these reach: grounding, and the step between grounding and the search.

A search stopped early is not an error. It returns a result that says what happened,
and the control stays usable for the next call.

The examples share this program:

```rust
// Eleven pigeons, ten holes, one pigeon per hole: no answer set, and clasp
// needs a long search to prove it.
const PIGEONS: &str = "
    pigeon(1..11). hole(1..10).
    { in(P, H) : hole(H) } = 1 :- pigeon(P).
    :- in(P, H), in(Q, H), P < Q.";
# let _ = PIGEONS;
```

## Give one call a timeout

`SolveOptions::timeout` sets a budget for one call. When it is spent, the search is
interrupted, and the call returns with a result that `is_interrupted()`:

```rust
use std::time::Duration;

use clingox::{Control, ErrorKind, Part, SolveOptions};
# const PIGEONS: &str = "
#     pigeon(1..11). hole(1..10).
#     { in(P, H) : hole(H) } = 1 :- pigeon(P).
#     :- in(P, H), in(Q, H), P < Q.";

let mut ctl = Control::new()?;
ctl.add_base(PIGEONS)?;
ctl.ground(&[Part::base()])?;

let options = SolveOptions::new().timeout(Duration::from_millis(100));
match ctl.solve_with(options) {
    Ok(result) if result.is_interrupted() => {
        // No model was found in time, so the result is unknown.
        assert!(result.is_unknown());
        println!("undecided after 100 ms");
    }
    Ok(result) => {
        println!("{result}");
#       panic!("the pigeons need more than 100 ms");
    }
    Err(err) if err.kind() == ErrorKind::Unsupported => {
        println!("this build of clingo has no threads");
    }
    Err(err) => return Err(err),
}
# Ok::<(), clingox::Error>(())
```

The budget counts from the start of the call, but it can only stop the search once
the search has started: the preparation before it is not interrupted (see
[below](#before-the-search-starts)). A search that ends sooner returns at once with its
own result. An interrupted result is never reported as unsatisfiable,
even when clingo says so, because the search may have missed a model (see
[Known issues](../reference/known-issues.md#results-to-read-carefully)).

The calls that return models take the same options:

| Call | When the timeout stops the search |
|---|---|
| `solve_first_with` | `Outcome::Unknown` if no model was found yet |
| `solve_optimal_with` | `Outcome::Sat` with the best model so far, not proven optimal, or `Outcome::Unknown` |
| `solve_all_with` | the models found so far, with an interrupted result |
| `solve_with_events` | the events delivered so far, and an interrupted result |

[Optimisation](../tutorial/optimisation.md) shows `solve_optimal_with` returning the
best model so far.

A timeout needs a build of clingo with threads, because something must stop the
search while your thread waits for it. Without threads, as on the default
WebAssembly build, these calls return `ErrorKind::Unsupported` before solving
anything, and the control stays usable. The [last section](#without-threads) lists
what works there.

## Stop a search from another thread

`Control::interrupt_handle` gives an `InterruptHandle`. It is `Send`, `Sync`, `Clone`
and `'static`, does not borrow the control, and may outlive it. `interrupt()` stops
the solve call that is running on the control and returns `true`. When no solve call
is running, it does nothing and returns `false`: clingo would otherwise keep the
interrupt and stop the next solve call at its start, and clingox never lets that
happen.

So a thread that decides to stop the search, for example because a user pressed a
button, has to allow for a search that has not started yet, or has ended:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use clingox::{Control, Part};
# const PIGEONS: &str = "
#     pigeon(1..11). hole(1..10).
#     { in(P, H) : hole(H) } = 1 :- pigeon(P).
#     :- in(P, H), in(Q, H), P < Q.";
# if !clingox_sys::HAS_THREADS { return Ok(()); }

let mut ctl = Control::new()?;
ctl.add_base(PIGEONS)?;
ctl.ground(&[Part::base()])?;

let stop = ctl.interrupt_handle();
let finished = Arc::new(AtomicBool::new(false));
let watcher = thread::spawn({
    let finished = Arc::clone(&finished);
    move || {
        thread::sleep(Duration::from_millis(50));
        // `interrupt` returns `false` until the search runs, and after it ends.
        while !finished.load(Ordering::Acquire) && !stop.interrupt() {
            thread::sleep(Duration::from_millis(1));
        }
    }
});

let result = ctl.solve(&[])?;
finished.store(true, Ordering::Release);
watcher.join().expect("the watcher does not panic");
assert!(result.is_interrupted());
# Ok::<(), clingox::Error>(())
```

Every solve call can be interrupted this way: `Control::solve`, `solve_with`, a
`SolveHandle` and the calls built on it, and an `AsyncSolveHandle`. Grounding cannot.
On a build without threads, `Control::solve` and `solve_with` run the whole search
inside one call to clingo, and `interrupt()` returns `false` for them; searches
through a `SolveHandle`, such as `for_each_model`, can be interrupted there from a
callback.

## Poll a search in the background

`Control::solve_async` starts the search on clasp's own thread and returns an
`AsyncSolveHandle` once clingo has prepared the search. `wait` waits up to a given time and says whether the
search has finished, so your own loop decides when to give up:

```rust
use std::time::{Duration, Instant};

use clingox::{Control, Part};
# const PIGEONS: &str = "
#     pigeon(1..11). hole(1..10).
#     { in(P, H) : hole(H) } = 1 :- pigeon(P).
#     :- in(P, H), in(Q, H), P < Q.";
# if !clingox_sys::HAS_THREADS { return Ok(()); }

let mut ctl = Control::new()?;
ctl.add_base(PIGEONS)?;
ctl.ground(&[Part::base()])?;

let deadline = Instant::now() + Duration::from_millis(100);
let mut handle = ctl.solve_async(&[])?;
while !handle.wait(Duration::from_millis(10)) {
    // Report progress or check for a cancelled request here.
    if Instant::now() >= deadline {
        handle.cancel()?;
    }
}
let result = handle.close()?;
assert!(result.is_interrupted() && result.is_unknown());
# Ok::<(), clingox::Error>(())
```

After `cancel`, `wait` returns `true` at once, so the loop ends. The handle gives no
access to models. Dropping it cancels the search. `solve_async` needs threads and
returns `ErrorKind::Unsupported` without them.

## Limit the search, not the time

clingo's option `--solve-limit=<n>` stops a search after `n` conflicts (and
`--solve-limit=<n>,<m>` also after `m` restarts). The result is unknown, but not
interrupted. Unlike a timeout, the limit does not depend on the machine or its load,
and it needs no threads:

```rust
use clingox::{Control, Outcome, Part};
# const PIGEONS: &str = "
#     pigeon(1..11). hole(1..10).
#     { in(P, H) : hole(H) } = 1 :- pigeon(P).
#     :- in(P, H), in(Q, H), P < Q.";

let mut ctl = Control::with_args(["--solve-limit=1000"])?;
ctl.add_base(PIGEONS)?;
ctl.ground(&[Part::base()])?;

let result = ctl.solve(&[])?;
assert!(result.is_unknown() && !result.is_interrupted());
assert!(matches!(ctl.solve_first()?, Outcome::Unknown(_)));

// The limit is the configuration entry `solve.solve_limit`; this lifts it.
ctl.configuration().set("solve.solve_limit", "umax,umax")?;
# Ok::<(), clingox::Error>(())
```

With one solver thread and a fixed seed, both the defaults, the same program stops at
the same point on every run. That makes a conflict limit a good budget for tests and
for the WebAssembly build. How many conflicts a second buys depends on the program,
so measure it on yours.

## Stop between models

A model loop can check the clock itself. The closure of `for_each_model` returns
`ControlFlow::Break` to stop the search, so it can stop at the first model after a
deadline:

```rust
use std::time::{Duration, Instant};

use clingox::prelude::*;

let mut ctl = Control::with_args(["--models=0"])?;
// About a million answer sets.
ctl.add_base("{ p(1..20) }.")?;
ctl.ground(&[Part::base()])?;

let deadline = Instant::now() + Duration::from_millis(50);
let mut count = 0;
let result = ctl.for_each_model(&[], |_model| {
    count += 1;
    if Instant::now() < deadline {
        Ok(ControlFlow::Continue(()))
    } else {
        Ok(ControlFlow::Break(()))
    }
})?;
println!("{count} models in 50 ms");
assert!(result.is_sat() && result.is_interrupted());
# Ok::<(), clingox::Error>(())
```

The check runs only when a model is found, so it bounds the time between models, not
the time to the first one. It needs no threads.

## Without threads

On a build of clingo without threads, which includes the default WebAssembly build,
there is no second thread to stop a search that runs inside one call to clingo:

| Tool | Without threads |
|---|---|
| `SolveOptions::timeout` | `ErrorKind::Unsupported` |
| `solve_async` | `ErrorKind::Unsupported` |
| `InterruptHandle` with `Control::solve` | `interrupt()` returns `false` |
| `InterruptHandle` from a callback of a `SolveHandle` search | works |
| `--solve-limit` | works |
| a deadline in `for_each_model` | works, between models |

Code that runs on both kinds of build can try the timeout and fall back to a conflict
limit when the call returns `ErrorKind::Unsupported`. [Feature flags](../reference/feature-flags.md#threads)
says which builds have threads.

## Before the search starts

Between grounding and the search, clingo prepares the ground program for clasp inside
the solve call. That step cannot be interrupted, and a timeout counts it but cannot
cut it short. For a large ground program it takes noticeable time: in a release build,
`{ a(1..1000000) }. b(X) :- a(X).` with a timeout of 1 ms returned after about 1.2 s,
and `solve_async` returned its handle after about 2.6 s. Once that step ends, a
deadline that has already passed stops the search at once. Its cost grows with the
size of the ground program, which the guard below can bound.

## Grounding has no time budget

None of the tools above reach grounding: clingo grounds a program inside one call
that cannot be interrupted. clingox can bound the **size** of the ground program: a
guard counts the atoms and rules the grounder produces, and stops it with
`ErrorKind::GroundingLimit` once a count is exceeded:

```rust
use clingox::observer::GroundingLimit;
use clingox::{Control, ErrorKind, Part};

let mut ctl = Control::new()?;
ctl.add_base("p(1..1000000).")?;
let limit = GroundingLimit::new(None, Some(1000)); // at most 1000 rules
let err = ctl.ground_with_limit(&[Part::base()], limit).unwrap_err();
assert_eq!(err.kind(), ErrorKind::GroundingLimit);
# Ok::<(), clingox::Error>(())
```

The guard bounds that count and nothing else. It does not bound the time grounding
takes, the memory it uses, the elements of an aggregate, or a join that does much work
and produces little. Two programs that stay far below a limit of 100,000 atoms and
rules show this, measured in a release build:

- `p(1..N). q :- #sum { X,Y : p(X), p(Y) } > 1.` grounds without an error, and
  needed 420 MB and 3 s for `N = 2000`, 1.7 GB and 13 s for `N = 4000`, and was
  killed at a 4 GB memory cap after 44 s for `N = 8000`;
- `a(1..N). b(X,Y,Z) :- a(X), a(Y), a(Z), X*Y*Z == 99999999.` produces nearly
  nothing, even under a limit of 1000, and took 3.8 s for `N = 400`, eight times
  longer for each doubling of `N`.

To bound the time and memory of grounding a program you do not control, run it in a
separate process with limits from the operating system;
[Use clingox in a server](server.md) shows where that fits.

The error poisons the control, because clingo keeps the part of the program it ground
before the guard stopped it. Create a new control for the next attempt.
[Observing the ground program](../concepts/observer.md#the-grounding-size-guard)
describes the guard.

clingo's own `--time-limit` option belongs to clingo's command-line application. A
`Control` does not accept it, and under
[`Application::run`](clingo-applications.md) it ends the process when it expires.

## Related pages

- [Solving step by step](../tutorial/solving-step-by-step.md) explains
  `SolveResult` and stopping a model loop with `Break`.
- [Safety and threads](../concepts/safety-and-threads.md) says which thread a search
  runs on, and why an interrupt never reaches a later call.
- [Error kinds](../reference/error-kinds.md#unsupported) lists what returns
  `ErrorKind::Unsupported`.

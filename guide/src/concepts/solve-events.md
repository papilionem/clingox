# Solve events and user statistics

A search reports four kinds of event as it runs: a model is found, an
optimisation problem is found unsatisfiable (with the best bound proven so
far), the statistics can be updated, and the search has finished.
`SolveEventHandler` gives one method per event, each with a default body that
does nothing:

```rust
use std::ops::ControlFlow;

use clingox::{Control, ExtendableModel, Part, SolveEventHandler, SolveOptions};

struct CountModels<'a>(&'a mut u32);

impl SolveEventHandler for CountModels<'_> {
    fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> clingox::Result<ControlFlow<()>> {
        *self.0 += 1;
        Ok(ControlFlow::Continue(()))
    }
}

let mut ctl = Control::with_args(["--models=0"])?;
ctl.add_base("{a;b}.")?;
ctl.ground(&[Part::base()])?;
let mut count = 0_u32;
ctl.solve_with_events(SolveOptions::new(), CountModels(&mut count))?;
assert_eq!(count, 4);
# Ok::<(), clingox::Error>(())
```

## The three entry points

`Control::solve_with_events` is the blocking form, mirroring
`Control::solve_with`: it runs the whole search and returns the result. It is
the only one of the three that accepts a handler borrowing the caller's own
locals (as the example above does with `count`), because a blocking call
always closes its search and drops the handler before returning.
`Control::solve_yield_with_events` mirrors `Control::solve_yield`: it returns
a `SolveHandle` that yields models one at a time, exactly as a plain
`solve_yield` search does, while still calling the handler for every event.
`Control::solve_async_with_events` mirrors `Control::solve_async`: the search
runs on clasp's own thread. Both of these need `Send + 'static`: dropping
either kind of handle with `mem::forget` is safe (DESIGN S4), and doing so
leaves the search, and the handler with it, open indefinitely, closed only by
the next call on the control; a borrowed handler's own borrow could then end
while the search was still able to call into it, which cannot happen to
`solve_with_events`. All three additionally require `Send`: parallel solving
can report a model from any solver thread, and a blocking search that
something can interrupt, a live `InterruptHandle` or a timeout, runs on clasp's
own thread while the caller's thread waits. A blocking search that nothing can
interrupt, and a yielding search, run on the caller's thread when there is one
solver thread, but that is not a promise and the bound stays.

All three take a handler by value; none of them hand it back. For
`solve_with_events`, a borrowed local (a closure-shaped handler struct, as the
example above does) is the simplest way to read what the handler saw
afterwards. For the other two, since the handler must be `'static`, capture
what it saw through a shared `Arc<Mutex<_>>` or an atomic instead.

None of the model-only entry points (`Control::for_each_model`,
`Control::solve_first`, `Control::solve_optimal`, `Control::solve_all`, or
their `_with` siblings) has an event-handler variant: they are already built
on a model-only closure, and adding a `_with_events` twin to each of them
would only duplicate the model event's own job.

## `goon` is not the same channel as an error

This is the one thing to get right with this API. Returning
`Ok(ControlFlow::Break(()))` from any method stops the search *gracefully*,
through clingo's own `goon` out-parameter, and nothing is treated as a
failure. This is not the same as an interrupt: clingo reserves its
interrupted flag for a signal delivered from outside the search, so a
graceful `Break` leaves the result exactly as clingo reports it:
satisfiable if a model was found, and neither exhausted nor interrupted.
`Control::for_each_model`'s own early exit differs: it cancels the search,
which clingo does report as interrupted. Returning `Err(e)` is a genuine
error: `e` is returned by the call that owns the search (or a panic resumes
on the caller's thread), exactly as a model closure's own error already
works with `Control::for_each_model`, and it never poisons the control.

An `Err`/panic from any of the four methods, `on_model` included, takes the
same path internally: clingox never returns `false` to clingo for any solve
event (`docs/dev/UPSTREAM- ISSUES.md` U25 and U26). This matters for two different upstream hazards:
clingo aborts the whole process for a `false` return from `on_unsat`,
`on_statistics` or `on_finish` (U25), and, separately, a `false` return from
`on_model` during an async, multi-threaded search leaves clasp's own
internal state corrupted, even though that path is otherwise clingo's
documented-safe one (U26). You never see either: whatever your handler
returns, the call that owns the search reports it as an ordinary `Result`.

```rust
use std::ops::ControlFlow;

use clingox::{Control, ExtendableModel, Part, SolveEventHandler, SolveOptions};

struct StopAfterOne(bool);

impl SolveEventHandler for StopAfterOne {
    fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> clingox::Result<ControlFlow<()>> {
        if std::mem::replace(&mut self.0, true) {
            return Ok(ControlFlow::Break(()));
        }
        Ok(ControlFlow::Continue(()))
    }
}

let mut ctl = Control::with_args(["--models=0"])?;
ctl.add_base("{a;b}.")?;
ctl.ground(&[Part::base()])?;
let result = ctl.solve_with_events(SolveOptions::new(), StopAfterOne(false))?;
assert!(result.is_sat() && !result.is_exhausted() && !result.is_interrupted());
# Ok::<(), clingox::Error>(())
```

## A handler's failure when the handle is dropped instead of closed

Dropping a `SolveHandle` or `AsyncSolveHandle` from `solve_yield_with_events`
or `solve_async_with_events` cancels and closes the search, exactly as
dropping a plain handle does. If the handler's `on_finish` fails or panics
during that drop-triggered close, the failure is discarded along with the
handler: `Drop` cannot return a value to report it with, and there is no
sensible "next call" to blame it on either, since the search that produced
it is already gone.

```rust
use std::ops::ControlFlow;

use clingox::{Control, Error, ErrorKind, Part, SolveEventHandler, SolveResult};

struct FailsOnFinish;

impl SolveEventHandler for FailsOnFinish {
    fn on_finish(&mut self, _result: SolveResult) -> clingox::Result<ControlFlow<()>> {
        Err(Error::new(ErrorKind::InvalidInput, "on_finish fails on purpose"))
    }
}

let mut ctl = Control::with_args(["--models=0"])?;
ctl.add_base("{a;b}.")?;
ctl.ground(&[Part::base()])?;

let mut handle = ctl.solve_yield_with_events(&[], FailsOnFinish)?;
handle.next_model()?;
drop(handle); // The failure never reaches anything: it is dropped with the handler.

// A later, unrelated call is unaffected: no stale failure surfaces here,
// and it does not poison the control either.
assert!(ctl.solve(&[])?.is_sat());
# Ok::<(), clingox::Error>(())
```

Call `SolveHandle::close` (or `AsyncSolveHandle::close`) instead of dropping
the handle if you need to know whether `on_finish` failed: that is the one
call which reports it, because it is the deliberate close the failure can be
attributed to.

## Extending a model

Only the model handed to `on_model` can be extended with `Model::extend`
(clingo.h says so explicitly: only the model passed to a solve-event
callback is extendable). The symbols added only show up in clingo's own
*printed* output, which clingox never produces, so the only way to see the
effect from a library is to read the extended model back inside the same
callback:

```rust
use std::ops::ControlFlow;

use clingox::{Control, ExtendableModel, Part, ShowType, SolveEventHandler, SolveOptions, Symbol};

struct AddSeventeen;

impl SolveEventHandler for AddSeventeen {
    fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> clingox::Result<ControlFlow<()>> {
        model.extend([Symbol::number(17)])?;
        assert!(model.symbols(ShowType::THEORY)?.contains(&Symbol::number(17)));
        Ok(ControlFlow::Continue(()))
    }
}

let mut ctl = Control::new()?;
ctl.add_base("a.")?;
ctl.ground(&[Part::base()])?;
ctl.solve_with_events(SolveOptions::new(), AddSeventeen)?;
# Ok::<(), clingox::Error>(())
```

`ExtendableModel` derefs to `Model`, so every ordinary reading method
(`symbols`, `contains`, `cost`, and the rest) works unchanged; `extend` is
the one method only this type has.

## Writing your own statistics

`on_statistics` hands the handler two writable trees, `step` (this step's
own numbers) and `accumulated` (the running total across every step so
far). Writes land under `user_step`/`user_accu` in `Control::statistics`'s
ordinary, read-only view once the search finishes. The event fires on every
step whether or not `--stats` was given; that option only changes which of
clasp's own entries are registered.

There is no "create a whole path at once" call: the C API only knows "add
one subkey to an existing map or array entry," so a tree is built one call
at a time, from the root down.

```rust
use std::ops::ControlFlow;

use clingox::{Control, MutableStatistics, Part, SolveEventHandler, SolveOptions, StatKind};

struct RecordCounts;

impl SolveEventHandler for RecordCounts {
    fn on_statistics(
        &mut self,
        step: &mut MutableStatistics<'_>,
        _accumulated: &mut MutableStatistics<'_>,
    ) -> clingox::Result<ControlFlow<()>> {
        step.add_map_key("", "mine", StatKind::Map)?;
        step.add_map_key("mine", "seen", StatKind::Value)?;
        step.set_value("mine.seen", 1.0)?;
        step.add_map_key("mine", "notes", StatKind::Array)?;
        let index = step.push_array("mine.notes", StatKind::Value)?;
        step.set_value(&format!("mine.notes.{index}"), 42.0)?;
        Ok(ControlFlow::Continue(()))
    }
}

let mut ctl = Control::with_args(["--stats=2"])?;
ctl.add_base("a.")?;
ctl.ground(&[Part::base()])?;
ctl.solve_with_events(SolveOptions::new(), RecordCounts)?;
let stats = ctl.statistics()?;
assert_eq!(stats.value("user_step.mine.seen")?, 1.0);
assert_eq!(stats.value("user_step.mine.notes.0")?, 42.0);
# Ok::<(), clingox::Error>(())
```

`push_array` returns the new element's own index (the array's size just
before the push), since there is no separate "read back the last element"
call: use it to address the entry you just created, as the example does.

Writing to the wrong kind of entry (`set_value` on a path that is a map, say)
is `ErrorKind::Runtime`, checked before clingo ever sees it (clingo itself
would report a *logic* error there, which would otherwise poison the
control):

```rust
use std::ops::ControlFlow;

use clingox::{Control, ErrorKind, MutableStatistics, Part, SolveEventHandler, SolveOptions, StatKind};

struct WrongKind;

impl SolveEventHandler for WrongKind {
    fn on_statistics(
        &mut self,
        step: &mut MutableStatistics<'_>,
        _accumulated: &mut MutableStatistics<'_>,
    ) -> clingox::Result<ControlFlow<()>> {
        step.add_map_key("", "mine", StatKind::Map)?;
        let err = step.set_value("mine", 1.0).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Runtime);
        Ok(ControlFlow::Continue(()))
    }
}

let mut ctl = Control::with_args(["--stats=2"])?;
ctl.add_base("a.")?;
ctl.ground(&[Part::base()])?;
ctl.solve_with_events(SolveOptions::new(), WrongKind)?;
# Ok::<(), clingox::Error>(())
```

**A note on threads.** clasp registers each kind of statistic the first
time it is used, and doing so was unsafe from several solver threads at
once before a vendored patch (see [Known issues](../reference/known-issues.md)).
That patch also covers your own statistics kinds, registered through
`push_array`/`add_map_key` the same way clasp registers its own; the
vendored build is safe under any number of solver threads, but a system
clingo built without the patch keeps the race.

## Reading statistics by entry

`Statistics::value` and `keys` take a path and resolve it from the root on
every call, which is right for one read. To read an entry again and again, or
to walk the tree, take an entry: `Statistics::entry(path)` resolves the path
once, and `Statistics::root` starts a walk. A `StatsEntry` holds clingo's key
for one place in the tree, so `value` needs no path lookup, and `children`
lists what is below, each child with the `PathSegment` (a name or an index)
it has under its parent.

```rust
use clingox::{Control, Part, PathSegment, StatKind};

let mut ctl = Control::with_args(["--models=0"])?;
ctl.add_base("{a;b}.")?;
ctl.ground(&[Part::base()])?;
ctl.solve(&[])?;
let stats = ctl.statistics()?;

// Resolve once, read as often as needed.
let enumerated = stats.entry("summary.models.enumerated")?;
assert_eq!(enumerated.value()?, 4.0);

// Walk below an entry; maps list names, arrays list indices.
let models = stats.entry("summary.models")?;
assert_eq!(models.kind()?, StatKind::Map);
for child in models.children()? {
    let (segment, entry) = child?;
    if segment == PathSegment::Name("optimal".to_owned()) {
        assert_eq!(entry.value()?, 0.0);
    }
}
# Ok::<(), clingox::Error>(())
```

Reading a map or array as a value is `ErrorKind::Runtime` and does not poison
the control, as it does not by path. `StatsEntry::len` counts the children of
any entry and is 0 for a value, unlike `ConfigEntry::len`, which is an error
for an entry that is not an array. Inside `on_statistics`, `step.root()` gives
the same kind of entry over the tree the handler is writing; `step` and
`accumulated` are the `user_step` and `user_accu` maps, so the root lists only
what the handler added. The entry borrows `step`, so write first and read after.
A `Statistics::snapshot` is still the way to keep the numbers once the control
moves on.

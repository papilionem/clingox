# Write a propagator

A propagator extends the search itself, rather than watching it from outside
(that is [the observer](../concepts/observer.md)) or reacting to its events
(that is [solve events](../concepts/solve-events.md)). It sees the solver's
own partial assignment while it runs, and can add clauses that follow from a
theory clingo's own grammar knows nothing about.

```rust
use clingox::propagate::Propagator;
use clingox::{Control, Part};

struct NoOp;
impl Propagator for NoOp {}

let mut ctl = Control::new()?;
ctl.add_base("1 { a; b }.")?;
ctl.ground(&[Part::base()])?;
ctl.register_propagator(NoOp)?;
assert!(ctl.solve(&[])?.is_sat());
# Ok::<(), clingox::Error>(())
```

`Propagator` has five methods, all defaulted to a no-op: `init`, `propagate`,
`undo`, `check` and `decide`. `Control::register_propagator` registers one
for the whole life of the control, the same way `register_observer` does; a
second registration does not replace the first, it adds a second, independent
propagator, and clingo runs every one of them.

## A complete example: no two adjacent

Here is a full propagator, not just a fragment: `NoTwoAdjacent` forbids two
consecutive `x(i)`/`x(i+1)` from both holding, entirely through the
propagator, with no ASP-level constraint of its own doing the work. It uses
every idea the rest of this chapter explains in more depth: mapping program
literals to solver literals in `init`, watching them, and adding a clause
from `propagate` once two adjacent ones are both about to become true.

```rust
use clingox::propagate::{ClauseType, PropagateControl, PropagateInit, Propagator, SolverLiteral};
use clingox::{Control, Part, Result, Signature};
use std::sync::OnceLock;

struct NoTwoAdjacent {
    /// `x(1)..x(5)`'s own solver literals, in argument order; set once in
    /// `init`.
    lits: OnceLock<Vec<SolverLiteral>>,
}

impl NoTwoAdjacent {
    fn neighbour(&self, lit: SolverLiteral) -> Option<SolverLiteral> {
        let lits = self.lits.get().expect("init ran first");
        let i = lits.iter().position(|&l| l == lit)?;
        lits.get(i + 1).copied()
    }
}

impl Propagator for NoTwoAdjacent {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let sig = Signature::new("x", 1)?;
        let mut by_argument = Vec::new();
        for atom in init.symbolic_atoms()?.by_signature(sig) {
            let atom = atom?;
            let n = atom.symbol().arguments().unwrap()[0]
                .as_number()
                .expect("x/1's own argument is a number");
            by_argument.push((n, atom.literal()));
        }
        // Sorted by x(_)'s own argument (not by the program literal's own
        // id, which grounding order does not promise matches argument
        // order), so `lits[i]`/`lits[i + 1]` are truly adjacent integers.
        by_argument.sort_by_key(|&(n, _)| n);
        let mut lits = Vec::new();
        for (_, plit) in by_argument {
            let slit = init.solver_literal(plit)?;
            init.add_watch(slit)?;
            lits.push(slit);
        }
        let _ = self.lits.set(lits);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        for &lit in changes {
            if let Some(next) = self.neighbour(lit)
                && control
                    .add_clause(&[-lit, -next], ClauseType::Learnt)?
                    .is_stop()
            {
                return Ok(()); // clingo asked us to stop
            }
        }
        Ok(())
    }
}

let mut ctl = Control::with_args(["--models=0"])?;
ctl.add_base("{x(1..5)}.")?;
ctl.ground(&[Part::base()])?;
ctl.register_propagator(NoTwoAdjacent {
    lits: OnceLock::new(),
})?;

let mut models = 0;
let _ = ctl.for_each_model(&[], |_m| {
    models += 1;
    Ok(std::ops::ControlFlow::Continue(()))
})?;
// One model per independent set of a path graph on 5 nodes: Fibonacci
// F(7) = 13, checked directly against clingo 5.8.2.
assert_eq!(models, 13);
# Ok::<(), clingox::Error>(())
```

The rest of this chapter unpacks the pieces this example already uses: why
`init` is the only place that can see `SymbolicAtoms` and map a program
literal to a solver literal, why `Propagator`'s methods take `&self` instead
of `&mut self`, what `ClauseType::Learnt` means, and what `Flow::Stop`
requires of the calls that follow it. [Propagators: literals and
reentrancy](../concepts/propagators.md) is the concepts-level companion to
this how-to, once the mechanics below are familiar: the difference between a
program literal and a solver literal, and exactly what "reentrant" does and
does not mean for a propagator's own calls back into clingo.

## `init`: mapping literals and adding watches

`init` runs once before each solving step, before any solver thread exists.
It is the only place a propagator sees `SymbolicAtoms`/`TheoryAtoms` for the
current grounding, and the only place it maps a program literal (an atom's
own literal) to the *solver* literal clasp actually reasons about:

```rust
use clingox::propagate::{PropagateInit, Propagator, SolverLiteral};
use clingox::{Control, Part, Result, Signature};
use std::sync::OnceLock;

struct WatchBoth {
    a: OnceLock<SolverLiteral>,
    b: OnceLock<SolverLiteral>,
}

impl Propagator for WatchBoth {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let literal_of = |init: &PropagateInit<'_>, name: &str| -> Result<SolverLiteral> {
            let program_literal = init
                .symbolic_atoms()?
                .by_signature(Signature::new(name, 0)?)
                .next()
                .expect("the atom exists")?
                .literal();
            init.solver_literal(program_literal)
        };
        let a = literal_of(init, "a")?;
        let b = literal_of(init, "b")?;
        init.add_watch(a)?;
        init.add_watch(b)?;
        let _ = self.a.set(a);
        let _ = self.b.set(b);
        Ok(())
    }
}

let mut ctl = Control::new()?;
ctl.add_base("1 { a; b }.")?;
ctl.ground(&[Part::base()])?;
ctl.register_propagator(WatchBoth {
    a: OnceLock::new(),
    b: OnceLock::new(),
})?;
ctl.solve(&[])?;
# Ok::<(), clingox::Error>(())
```

`SolverLiteral` has no public constructor from a bare integer: the only way
to hold one is to get it from clingo itself, through `solver_literal`,
`add_literal`, or a `propagate`/`decide` callback's own arguments. An
unregistered or out-of-range solver literal is not a documented error in
clingo, it is undefined behaviour that can segfault the process; closing this
off at the type level is why the restriction exists.

## Interior mutability, not `&mut self`

Every `Propagator` method takes `&self`. clasp releases its own internal
lock while `PropagateControl::add_clause`, `propagate`, `add_watch` and
`add_literal` call into clingo, which *could* let it call `undo` back on the
*same* thread before one of those calls returns; checked directly against
the vendored clasp 5.8.2, this never actually happens (a conflicting
`add_clause`'s own backtrack is always resolved only after control returns
to clasp's ordinary search loop, never while the triggering call is still on
the stack), and clingox's soundness does not depend on which way that goes.
With more than one solver thread, two different threads can also call
`propagate` on the same registered propagator's `&self` at once, which is
not hypothetical: it happens on every multi-thread search. A `Mutex` a
propagator locked in `propagate` and tried to lock again from a reentrant
`undo` would deadlock, not merely race, if a future clasp version ever took
that path; a `RefCell`, though it would only panic rather than corrupt
memory, is not `Sync` and so cannot appear in a `Propagator` type at all
regardless (`Propagator: Send + Sync`; a propagator that is not `Sync` is
rejected at compile time).

Keep any state a propagator needs to update in a container built for this:
an atomic, a `OnceLock` for something set once in `init` and only read
afterward (as `WatchBoth` above does), or, for state that is only ever
touched by one solver thread, a `Cell`/`RefCell` inside a `Vec` sized from
`PropagateInit::number_of_threads` and indexed by `PropagateControl::
thread_id`. **Never hold a lock of your own across a call into
`PropagateControl`.**

## The post-`Stop` guard

`PropagateInit::add_clause`, `add_weight_constraint` and `propagate` (and,
once the search starts, `PropagateControl::add_clause`) return a `Flow`:
`Flow::Continue` or `Flow::Stop`. `Flow::Stop` means clingo has found the
program unsatisfiable from the clauses added so far, and, per clingo's own
header, nothing further should be called on that object. Checked directly
against clingo 5.8.2: a further call after `Stop` is not an error in clingo
itself, it is accepted silently and does nothing useful, so clingox adds its
own guard, refusing it with an `InvalidInput` error before ever reaching
clingo:

```rust
use clingox::propagate::{PropagateInit, Propagator};
use clingox::{Control, ErrorKind, Part, Result};

struct Contradiction;
impl Propagator for Contradiction {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let lit = init.add_literal(true)?;
        init.add_clause(&[lit])?;
        let stop = init.add_clause(&[-lit])?;
        assert!(stop.is_stop());
        // A further call on this `init` is refused by clingox itself.
        let err = init.add_clause(&[lit]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        Ok(())
    }
}

let mut ctl = Control::new()?;
ctl.add_base("a.")?;
ctl.ground(&[Part::base()])?;
ctl.register_propagator(Contradiction)?;
assert!(ctl.solve(&[])?.is_unsat());
# Ok::<(), clingox::Error>(())
```

The guard covers every `PropagateInit`/`PropagateControl` method that would
otherwise reach clingo after `Stop`, not only the ones that themselves
return a `Flow`: `add_watch`, `add_watch_to_thread`, `remove_watch`,
`remove_watch_from_thread`, `add_literal`, `add_minimize` and
`freeze_literal` too, and every fallible query on the `Assignment`/`Trail`
`assignment()` hands back (`level`, `is_fixed`, `is_true`, `is_false`,
`truth_value`, `at`, `decision`, and `Trail`'s `size`/`begin`/`end`/`at`).
`PropagateControl::has_watch` is the one exception to the `InvalidInput`
shape: since it is itself infallible, it answers `false` unconditionally
once stopped, without asking clingo (which does not clear watches on
`Stop`, so its own honest answer would otherwise still be `true` for a
literal watched beforehand). `solver_literal`, `symbolic_atoms` and
`theory_atoms` on `PropagateInit` are refused too. `Assignment::decision_level`,
`root_level`, `has_conflict`, `size`, `is_total` and `has_literal` stay
callable either way. That is clingox's design choice rather than something
clingo's header permits (it forbids functions on the assignment in general
after a false result, `clingo.h:1296-1297` and `1436-1437`): these six return
plain values with no error channel to refuse through, and `has_literal` is
the primitive every guard validates a literal with. The same goes for
`number_of_threads` and the check and undo mode getters and setters.

## Errors and panics

An error or a panic from `init` poisons the whole `Control`, the same way a
failed or panicking ground callback does: clingo's own internal state after
a half-run `init` is not something clingox can safely expose, so recovery
means building a new `Control`. An error or a panic from `propagate`,
`undo`, `check` or `decide`, in contrast, never poisons: clingo itself
stays usable after one of these fails, so a further `solve` on the same
`Control` completes normally, whatever the error's kind. A panic from any
of the five methods is always caught before it can unwind into clingo's own
C++ frames, and resumed on the thread that made the call that led there,
once that call returns — never on a solver thread.

## Reading the assignment and the trail

`Assignment` is the solver's own (partial) assignment: `PropagateInit::
assignment`, `PropagateControl::assignment` and `Propagator::decide`'s own
parameter all hand one out, borrowed for exactly the call that produced it.
Besides the properties usable from `init` (`decision_level`, `root_level`,
`has_conflict`, `size`, `is_total`), it answers questions about individual
solver literals once the search has started:

```rust
use clingox::propagate::{CheckMode, PropagateControl, PropagateInit, Propagator};
use clingox::{Control, Part, Result};

struct WatchTruth;
impl Propagator for WatchTruth {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Total);
        Ok(())
    }

    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        for offset in 0..a.size() {
            let lit = a.at(offset)?;
            // `truth_value` is `None` while a literal is still free, and
            // `Some(true)`/`Some(false)` once the search has settled it;
            // `is_fixed` tells a level-0 fact apart from a literal that
            // could still be undone by backtracking.
            let _ = (a.truth_value(lit)?, a.is_fixed(lit)?, a.level(lit)?);
        }
        Ok(())
    }
}

let mut ctl = Control::new()?;
ctl.add_base("a. {b}.")?;
ctl.ground(&[Part::base()])?;
ctl.register_propagator(WatchTruth)?;
ctl.solve(&[])?;
# Ok::<(), clingox::Error>(())
```

Every accessor that takes a `SolverLiteral`, a level or an offset validates
it against this same assignment first, refusing one that does not belong to
it with an `InvalidInput` error rather than reaching clingo: `has_literal`
is the primitive this is built from, and it is safe to call with any
`SolverLiteral`, including one legitimately obtained from a *different*
control's grounding. `Assignment::level` returns `Option<u32>`, not a raw
`u32`: internally, clasp reports a sentinel value for a literal that is
known but not yet assigned, and clingox wraps that as `None` rather than
exposing the sentinel.

`Assignment::at` enumerates the (positive) literals in ascending, numeric
order; `Assignment::trail` gives a different view over the same state, in
the chronological order the solver actually assigned each literal:

```rust
use clingox::propagate::{CheckMode, PropagateControl, PropagateInit, Propagator};
use clingox::{Control, Part, Result};

struct WatchTrail;
impl Propagator for WatchTrail {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        init.set_check_mode(CheckMode::Total);
        Ok(())
    }

    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let a = control.assignment();
        let t = a.trail();
        for level in 0..=a.decision_level() {
            // Each level's own slice: the literal that made the decision,
            // and everything unit propagation implied from it.
            for lit in t.level(level)? {
                let _ = lit;
            }
        }
        // `&Trail` is iterable too, yielding every literal in the order
        // above, one call at a time.
        for lit in &t {
            let _ = lit?;
        }
        Ok(())
    }
}

let mut ctl = Control::new()?;
ctl.add_base("a. {b}.")?;
ctl.ground(&[Part::base()])?;
ctl.register_propagator(WatchTrail)?;
ctl.solve(&[])?;
# Ok::<(), clingox::Error>(())
```

`Trail::begin`/`Trail::end` bound one level's own slice of the trail, and
both validate their `level` the same way; `Trail::end` is the one place
clingox invents a check clasp itself does not have (clasp's own `trailEnd`
never fails on its own for a level at or above the current decision level),
added only so `Trail::begin` and `Trail::end` behave the same way at their
own shared boundary.

## Adding clauses: `ClauseType`

`PropagateControl::add_clause` takes a `ClauseType`, which chooses how long
the clause survives, not what it means:

| `ClauseType` | Exempt from the solver's own deletion policy? | Survives into the *next* solving step? |
|---|---|---|
| `Learnt` | No | Not guaranteed either way (governed by clasp's own activity-based deletion, not step boundaries) |
| `Static` | Yes | Yes |
| `Volatile` | No | No |
| `VolatileStatic` | Yes | No |

Checked directly against clingo 5.8.2: a clause forbidding an atom, added `Volatile` or
`VolatileStatic` in one solving step, no longer forbids it in the next (the
atom becomes satisfiable again); the same clause added `Static` still
forbids it. `Volatile`/`VolatileStatic`'s own difference only matters
*within* a step, not across one: "static" here means "exempt from the
solver's regular activity-based deletion," never "survives past this step."
Use `Volatile` for a clause that only makes sense for the current solving
step (as the reentrancy example below does), and `Static` for one that
should keep holding across every later step on the same control.

## Watches, and a fresh literal, from `PropagateControl`

`PropagateInit::add_watch` sets up watches before the search starts, on
every solver thread by default. Once solving is under way,
`PropagateControl::add_watch`/`has_watch`/`remove_watch` do the same thing
for the *current* solver thread only — watching a literal from one thread's
own `propagate` call never affects another thread's watches for that same
literal, and removing one never touches `PropagateInit`'s own, every-thread
registration. `PropagateControl::add_literal` adds a fresh, volatile solver
literal, usable for the rest of the current solving step and thread only:
unlike `PropagateInit::add_literal`'s own literal, there is no `freeze`
parameter to make it survive further, and reusing it in a later solving
step is undefined behaviour clingox closes off the same way it closes off a
literal from a different control (both report unknown to
`assignment().has_literal`, refused with `InvalidInput` before ever
reaching clingo):

```rust
use clingox::propagate::{PropagateControl, Propagator};
use clingox::{Control, Part, Result};
use std::sync::Mutex;

// Run this only once (`added`): `check` fires again whenever the
// assignment is total, and a fresh, unassigned literal never becomes true
// or false on its own, so a second call would add another one, keeping the
// assignment non-total forever and firing `check` again, forever.
#[derive(Default)]
struct WatchDuringCheck {
    added: Mutex<bool>,
}
impl Propagator for WatchDuringCheck {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let mut added = self.added.lock().unwrap();
        if !*added {
            *added = true;
            let lit = control.add_literal()?;
            assert!(!control.has_watch(lit));
            control.add_watch(lit)?;
            assert!(control.has_watch(lit));
            control.remove_watch(lit)?;
            assert!(!control.has_watch(lit));
        }
        Ok(())
    }
}

let mut ctl = Control::new()?;
ctl.add_base("a.")?;
ctl.ground(&[Part::base()])?;
ctl.register_propagator(WatchDuringCheck::default())?;
ctl.solve(&[])?;
# Ok::<(), clingox::Error>(())
```

`PropagateControl::propagate` propagates the consequences of whatever has
been added so far during the current call, before the outer
`propagate`/`undo`/`check` returns; called with nothing pending, it
succeeds immediately with `Flow::Continue`.

**A known, benign clasp-internal race with several solver threads.** Call
`add_watch`/`has_watch`/`remove_watch` from more than one solver thread of a
propagator registered with plain `Control::register_propagator` (not
`_sequential`), and clasp reads its master solver's own assignment word with
no lock, which can race a write from that same thread's ordinary
decision-making (`docs/dev/UPSTREAM-ISSUES.md`'s U28). The bit actually read
never changes once the search starts, so this has never produced a wrong
answer in practice, and `cargo xtask sanitize` suppresses exactly this one,
narrowly — but it is a genuine data race in C++'s own formal sense. If you
need to be certain no such race exists in your own build's clasp,
`Control::register_propagator_sequential` avoids it entirely, at the cost of
serialising every call into the propagator.

## Check modes

`PropagateInit::set_check_mode` chooses when `Propagator::check` runs,
independent of any watch: `CheckMode::Off` never calls it,
`CheckMode::Total` calls it once the assignment is total,
`CheckMode::Fixpoint` calls it at every propagation fixpoint (typically many
more times than `Total`, since a search reaches many fixpoints on the way
to one total assignment), and `CheckMode::Both` fires at every point either
of the other two would. The default is `CheckMode::Total`. `check` runs
even for a propagator that added no watches at all, which makes it the
place to enforce a constraint that genuinely needs to see the *whole*
assignment rather than reacting to individual literals becoming true — as
the assignment-reading example above already does.

## Reentrancy, concretely: a forced backjump

"Interior mutability, not `&mut self`," above, states the rule. Here is why
`clingox` states it as a rule rather than a one-off warning: clasp releases
its own internal lock for the duration of `PropagateControl::add_clause`,
`propagate`, `add_watch` and `add_literal`, and a clause that conflicts with
the assignment at a decision level *below* the current one forces a
backtrack past the propagator's own watched literals, calling
`Propagator::undo` one or more times. Checked directly against the vendored
clasp 5.8.2 (`clasp/src/clingo.cpp`'s own `ClingoPropagator::Control`
constructor always sets a flag, `state_ctrl`, that makes `add_clause`'s and
`propagate`'s own conflict-handling code defer the actual backjump instead
of resolving it inline): **`undo` never runs while the call that triggered
it is still on the same thread's own stack in this clasp version** — the
backjump is always resolved afterward, from clasp's own outer search loop.
`clingox`'s own soundness never depended on which way this went, though:
`Propagator`'s methods take `&self`, are `Send + Sync`, and no lock of
clingox's own is ever held across a call into `PropagateControl`, so the
design stays sound whether a future clasp version ever calls `undo` back
synchronously or not.

Forcing the conflict needs somewhere for clasp to backjump *to*: a plain
fact has no decision to undo, so this example uses three genuine choices,
`{b; d; e}.`, and `Propagator::decide` to force them true one at a time, so
the scenario reproduces the same way every run (adapted from the crate's
own `clingox/tests/propagator_reentrancy.rs`, which runs the identical
shape under the thread sanitizer, at 1, 2 and 8 solver threads). It measures
the claim directly, with a per-thread "currently inside `propagate`" flag
`undo` checks against, rather than trusting the source reading alone:

```rust
use clingox::propagate::{
    Assignment, ClauseType, PropagateControl, PropagateInit, Propagator, SolverLiteral,
};
use clingox::{Control, Part, Result, Signature};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

struct ForcesABackjump {
    b: OnceLock<SolverLiteral>,
    d: OnceLock<SolverLiteral>,
    e: OnceLock<SolverLiteral>,
    fired: Mutex<bool>,
    in_propagate: AtomicBool,
    undo_calls: Arc<AtomicU32>,
    undo_was_reentrant: Arc<AtomicBool>,
}

impl Propagator for ForcesABackjump {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let lit = |init: &PropagateInit<'_>, name: &str| -> Result<SolverLiteral> {
            let plit = init
                .symbolic_atoms()?
                .by_signature(Signature::new(name, 0)?)
                .next()
                .expect("the atom exists")?
                .literal();
            init.solver_literal(plit)
        };
        let (b, d, e) = (lit(init, "b")?, lit(init, "d")?, lit(init, "e")?);
        for l in [b, d, e] {
            init.add_watch(l)?;
            init.add_watch(-l)?;
        }
        let _ = self.b.set(b);
        let _ = self.d.set(d);
        let _ = self.e.set(e);
        Ok(())
    }

    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        self.in_propagate.store(true, Ordering::SeqCst);
        let e = *self.e.get().expect("init ran first");
        if changes.contains(&e) {
            let mut fired = self.fired.lock().unwrap();
            if !*fired {
                *fired = true;
                let b = *self.b.get().expect("init ran first");
                let forced = if control.assignment().is_true(b)? { -b } else { b };
                // A unit, volatile clause conflicting with `b`'s own,
                // lower-level decision forces a backjump that undoes `d`
                // and `e`'s own decisions -- but not synchronously here:
                // clingo 5.8.2 always resolves it after this call returns.
                let _ = control.add_clause(&[forced], ClauseType::Volatile)?;
            }
        }
        self.in_propagate.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn undo(&self, _control: &PropagateControl<'_>, _changes: &[SolverLiteral]) {
        self.undo_calls.fetch_add(1, Ordering::SeqCst);
        if self.in_propagate.load(Ordering::SeqCst) {
            self.undo_was_reentrant.store(true, Ordering::SeqCst);
        }
    }

    fn decide(
        &self,
        _thread_id: u32,
        assignment: &Assignment<'_>,
        fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        // Forces b, then d, then e true, so the scenario above reproduces
        // the same way every run.
        for slot in [&self.b, &self.d, &self.e] {
            let l = *slot.get().expect("init ran first");
            if assignment.truth_value(l)?.is_none() {
                return Ok(Some(l));
            }
        }
        Ok(Some(fallback))
    }
}

let undo_calls = Arc::new(AtomicU32::new(0));
let undo_was_reentrant = Arc::new(AtomicBool::new(false));

let mut ctl = Control::new()?;
ctl.add_base("{b; d; e}.")?;
ctl.ground(&[Part::base()])?;
ctl.register_propagator(ForcesABackjump {
    b: OnceLock::new(),
    d: OnceLock::new(),
    e: OnceLock::new(),
    fired: Mutex::new(false),
    in_propagate: AtomicBool::new(false),
    undo_calls: Arc::clone(&undo_calls),
    undo_was_reentrant: Arc::clone(&undo_was_reentrant),
})?;
ctl.solve(&[])?;

// The backjump genuinely happens: `undo` fires...
assert!(undo_calls.load(Ordering::SeqCst) > 0);
// ...but, in this clasp version, never while `add_clause`'s own call was
// still on the stack.
assert!(!undo_was_reentrant.load(Ordering::SeqCst));
# Ok::<(), clingox::Error>(())
```

The full, verified mechanism (the `state_ctrl` flag, and exactly where it
makes `ClingoPropagator::addClause` defer instead of resolving inline) is in
DESIGN.md's own S11, checked against the vendored `clasp/src/clingo.cpp`.
`clingox/tests/propagator_reentrancy.rs`'s own
`undo_never_runs_inside_propagate_after_add_clause_at_{one,two,eight}_threads`
pin exactly this under the thread sanitizer — DESIGN S11's own gate: "the
design is final only after those pass under TSan," which for the current
clasp version means confirming `undo` is *not* reentrant here, not the
reverse. Keep holding to "never hold a lock of your own across a call into
`PropagateControl`" anyway: it costs nothing today, and is exactly what
keeps this sound if a future clasp version ever does call `undo` back
synchronously.

## What is here

This chapter covers registration, `PropagateInit`, watches, the propagator
trait's dispatch (wired for all five methods from the start), the complete,
read-only `Assignment`/`Trail`, and `PropagateControl`'s complete surface:
`thread_id`, `assignment`, `add_clause` (with `ClauseType`), `add_literal`,
`add_watch`, `has_watch`, `remove_watch` and this type's own `propagate`.
`decide` returns `Ok(None)` to let the next propagator, and finally clasp,
choose, or `Ok(Some(literal))` to decide; the heuristic tests
(`clingox/tests/api_propagator_decide_heuristic.rs`) show it in use.

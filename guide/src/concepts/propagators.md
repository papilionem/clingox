# Propagators: literals and reentrancy

[Write a propagator](../how-to/write-a-propagator.md) is the worked how-to;
this page is the two ideas a propagator cannot be written correctly without
first having straight: which kind of literal a given `clingox` value is, and
what "reentrant" does and does not mean for the calls a propagator makes
back into clingo.

## Program literals and solver literals are not interchangeable

clingox has two distinct literal types, and mixing them up is a real
correctness hazard, not a style choice:

- A [`ProgramLiteral`](https://docs.rs/clingox/latest/clingox/struct.ProgramLiteral.html)
  names an atom in the *grounded program*: what `SymbolicAtom::literal`,
  `TheoryAtom::literal` and the backend's own `Atom::pos`/`Atom::neg` all
  return. It is stable for as long as the grounding it came from is.
- A [`SolverLiteral`](https://docs.rs/clingox/latest/clingox/propagate/struct.SolverLiteral.html)
  names a variable in *clasp's own search*, after preprocessing: what
  `PropagateInit::solver_literal`, `PropagateInit::add_literal`,
  `PropagateControl::add_literal`, and every `propagate`/`decide` callback's
  own arguments hand out. It is what `PropagateControl::add_clause`,
  `add_watch` and `Assignment` all take and report; nothing else does.

Preprocessing means the two numbering schemes are not simply offset from one
another, and are not usually equal even by coincidence for a program of any
size: two different program literals can map to the same solver literal (if
clasp determined they are equivalent), and a program literal can map to no
solver literal at all (if clasp eliminated it entirely). `PropagateInit::
solver_literal` is the only sanctioned way to go from one to the other, and
it only works during `init`, before a solving step starts.

`SolverLiteral` has no public constructor from a bare integer, on purpose:
an unregistered or out-of-range solver literal is not a documented error in
clingo, it is undefined behaviour that can segfault the process (checked
directly against clingo 5.8.2). Every clingox method that takes one
validates it against the *current* assignment first (`Assignment::
has_literal` is the primitive this is built from), refusing a literal that
does not belong to it — including one legitimately obtained from a
*different* `Control`'s own grounding — with an `InvalidInput` error rather
than reaching clingo. Holding onto a `SolverLiteral` past the solving step
that produced it and reusing it in a later one is the same kind of mistake,
and is refused the same way.

## Reentrancy: what actually calls back into what

A propagator's five methods (`init`, `propagate`, `undo`, `check`,
`decide`) are dispatch points clingo calls *into*; the reentrancy question
is about the opposite direction — what a propagator's own call *out*, into
`PropagateInit`/`PropagateControl`, can cause clingo to call back in, before
that outer call returns.

clasp releases its own internal lock for the duration of
`PropagateControl::add_clause`, `propagate`, `add_watch` and `add_literal`
(and their `PropagateInit` equivalents during `init`). This is what makes a
propagator's methods need `&self` rather than `&mut self`, and `Send +
Sync`: with more than one solver thread, two different threads can call
`propagate` on the same registered propagator at once, and — the sharper
case — a single thread's own call into `add_clause` can, in general, cause
clingo to call back into that *same* propagator's `undo` before
`add_clause` itself returns, if the clause it just added forces a conflict
and a backjump past the propagator's own watched literals.

Traced directly against the vendored clasp 5.8.2 source for this crate
(`clasp/src/clingo.cpp`'s `ClingoPropagator::Control` constructor always
sets a flag, `state_ctrl`, that makes `add_clause`'s and `propagate`'s own
conflict-handling code *defer* the actual backjump instead of resolving it
inline): in this clasp version, **`undo` never runs while the call that
triggered it is still on the same thread's own stack** — the backjump is
always resolved afterward, from clasp's own outer search loop, once
`add_clause`/`propagate` have already returned. The how-to's own "Forced
backjump" example measures this directly (a per-thread "currently inside
`propagate`" flag `undo` checks against) rather than trusting the source
reading alone, and `clingox/tests/propagator_reentrancy.rs` pins it under
the thread sanitizer at 1, 2 and 8 solver threads.

**This is a property of the current clasp version, not a guarantee this
crate's own API makes.** Nothing about clingox's soundness depends on which
way it goes: `Propagator`'s `&self`/`Send + Sync` shape and "never hold a
lock of your own across a call into `PropagateControl`" (the how-to's own
rule) stay correct whether a future clasp version ever calls `undo` back
synchronously from inside `add_clause` or not. Write every propagator as if
it might.

## One table: every method's own profile

| Method | Called from | Runs on | If it panics or errors |
|---|---|---|---|
| `init` | Once per solving step, before any solver thread exists | The thread that started solving | Poisons the whole `Control` (like a ground callback): the step may be half-configured, so recovery means a new `Control` |
| `propagate` | Whenever a watched literal becomes true, with a non-empty change set | Any solver thread; concurrently with other threads unless registered with `Control::register_propagator_sequential` | Stops the current solving step; reported by whichever call is running it. Does **not** poison the `Control` — a later, ordinary `solve` on the same control works |
| `undo` | Whenever the solver undoes assignments to watched literals, to backtrack | The same thread `propagate` ran on for that state; may run while a call this same propagator made into `add_clause`/`propagate` is still on that thread's own stack (see above) | Infallible at the C level (`Result` is not part of its signature); a panic is caught and resumed once the outer call that led here returns, never unwinding into clingo's own C++ frames |
| `check` | On a propagation fixpoint or a total assignment, as `PropagateInit::set_check_mode` configures; runs even with no watches at all | Any solver thread | As `propagate`: stops the step, does not poison the control |
| `decide` | Whenever propagation reaches a fixpoint and clasp needs a free literal to decide, in registration order, only once every earlier-registered propagator with a `decide` has declined | Any solver thread | As `propagate`: stops the step, does not poison the control |

"Poisons" means every later call on that `Control` answers
[`ErrorKind::Poisoned`](https://docs.rs/clingox/latest/clingox/enum.ErrorKind.html);
see [Errors and panics](errors-and-panics.md) for what poisoning means
project-wide, and the how-to's own "Errors and panics" section for the
propagator-specific case in more depth.

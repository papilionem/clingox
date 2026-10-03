# Safety and threads

clingox is a safe binding: code that uses it needs no `unsafe`, and no use of its safe
API, with any input, is meant to cause undefined behaviour. This chapter explains how
that promise is kept, which of it the compiler enforces, and the rules for threads:
which types may cross them, where your callbacks run, and what holds for several
controls on several threads.

## Where the `unsafe` code is

All of clingox's `unsafe` code is in one private module, `clingox::raw`: one thin
wrapper per clingo C function, and the primitives they share. Every other module of
the library is compiled with `#![deny(unsafe_code)]` and calls only `raw`. Every
`unsafe` block carries a comment that names the invariant it relies on, and
`cargo xtask check` counts the blocks, so a new one cannot appear unnoticed.

The declarations of clingo's C API are in the separate crate `clingox-sys`. Its
functions are `unsafe` to call and come with no guarantees beyond clingo's header;
you do not need it to use clingox.

CI runs the test suite on Linux under AddressSanitizer and LeakSanitizer, the thread
tests under ThreadSanitizer, and the unit tests and the callback trampolines under
Miri ([Platform support](../reference/platforms.md#when-the-jobs-run)).

## What the compiler enforces

clingo's C API has rules about when an object may be used: a model is valid only
until the search moves on, statistics must not be read while a search runs, the
program must not change while a solve handle is open. clingox expresses these rules
as borrows, so breaking one is a compile error rather than a crash:

- A `SolveHandle`, `AsyncSolveHandle`, `Backend`, `ProgramBuilder` or `Configuration`
  borrows its control mutably, so nothing else can use the control meanwhile.
- `SymbolicAtoms`, `TheoryAtoms` and `Statistics` borrow it immutably, so `ground` and
  `solve` cannot run while one of them is alive.
- A `ConfigEntry` borrows its `Configuration`, so `set` cannot run while one is alive,
  and a `StatsEntry` borrows the control like `Statistics` does. An entry holds
  clingo's key for one node of the tree, which the borrow keeps valid.
- `SolveHandle::next_model` lends a `Model` that lives until the next call on the
  handle. `Model::snapshot` makes an `OwnedModel` that you can keep.

This does not compile, because `stats` still borrows `ctl` when `solve` needs it:

```rust,compile_fail,E0502
use clingox::{Control, Part};

let mut ctl = Control::new()?;
ctl.add_base("a.")?;
ctl.ground(&[Part::base()])?;
let stats = ctl.statistics()?;
ctl.solve(&[])?; // error[E0502]: `ctl` is also borrowed as immutable
println!("{}", stats.value("summary.models.enumerated")?);
# Ok::<(), clingox::Error>(())
```

Two more rules hold without the compiler's help:

- **Leaking is safe.** `std::mem::forget` is safe Rust, so no guarantee rests on a
  destructor running. A control records an open search, backend or program builder,
  and every later call on it closes what was left open before it does anything else.
- **Controls that clingo owns cannot escape.** The control that
  [`Application::main`](../how-to/clingo-applications.md) receives belongs to clingo,
  which frees it after the call. Its type carries a lifetime brand that stops it, and
  everything borrowed from it, from leaving the call.

## Errors and panics stay on the Rust side

clingo reports failures through return codes, and a Rust panic must never unwind
through its C++ frames. clingox turns every failure into an `Error`, catches every
panic in a callback and resumes it on your thread when the call returns, and poisons
a control after an error that leaves clingo's state unknown.
[Errors and panics](errors-and-panics.md) explains each part, and
[Error kinds](../reference/error-kinds.md) lists the kinds.

clingox also checks, before calling clingo, the values that clingo is known to
mishandle: literals, indices, counts, thread numbers, arities, strings with NUL bytes,
and syntax trees that would become cyclic. For every value clingox checks, a bad value
is an `Err`, never a call that clingo would answer with a crash or a wrong result.
Some input still reaches defects in clingo itself: a deeply nested term overflows the
stack, a huge `#project` arity exhausts memory, and some divisions crash a clingo
installed on the system. [Known issues](../reference/known-issues.md) lists them; the
vendored build patches several.

## Which types cross threads

| Type | `Send` | `Sync` | Notes |
|---|---|---|---|
| `Control` | yes | no | moves between threads between calls; one thread at a time |
| `SolveHandle`, `AsyncSolveHandle`, `Backend`, `ProgramBuilder` | yes | no | they borrow the control mutably, so they move with it |
| `Model`, `Statistics`, `SymbolicAtoms`, `TheoryAtoms`, `Configuration` | no | no | views that stay on the thread where they were made |
| `ConfigEntry`, `StatsEntry` and their `children()` iterators | no | no | they borrow a view or the control |
| `Symbol`, `Signature` | yes | yes | also `Copy` and `'static` |
| `PathSegment` | yes | yes | an owned name or index, `'static` |
| `OwnedModel`, `SolveResult`, `StatsTree`, `Error`, `SolveOptions` | yes | yes | owned values, independent of the control |
| `InterruptHandle` | yes | yes | also `Clone` and `'static`; it does not borrow the control and may outlive it |
| `ast::Ast` | no | no | clingo counts references to a node without atomic operations |

`Control` is `Send` because clingo keeps no state tied to the thread that created a
control, apart from its error state, which clingox sets and reads within each call. A
control created on one thread, solved on another and dropped on a third ran clean
under ThreadSanitizer with 1, 4 and 8 solver threads. It is not `Sync`, because two
threads must never call into the same control at once. To share one, put it behind a
`Mutex` ([Use clingox in a server](../how-to/server.md#ground-once-answer-many-queries)
does).

A control moves to another thread like any `Send` value:

```rust
use std::thread;

use clingox::{Control, Part};
# if cfg!(all(target_family = "wasm", not(target_feature = "atomics"))) { return Ok(()); }

let mut ctl = Control::new()?;
ctl.add_base("{ a; b }.")?;
let ctl = thread::spawn(move || -> clingox::Result<Control> {
    ctl.ground(&[Part::base()])?;
    Ok(ctl)
})
.join()
.expect("the thread does not panic")?;

let mut ctl = ctl;
let (_, models) = ctl.solve_all()?;
assert_eq!(models.len(), 4);
# Ok::<(), clingox::Error>(())
```

Symbols live in one table for the whole process. The vendored build never frees it,
so a symbol stays valid on every thread until the process ends, and the table only
grows. A clingo installed on the system frees it when the process exits, so join every
thread that uses symbols before `main` returns (see
[Known issues](../reference/known-issues.md#fixed-in-the-vendored-build-open-with-a-system-clingo)).

## Where your callbacks run

Each kind of callback has the bounds its threads require:

| Callback | Runs on | Bound |
|---|---|---|
| the function of `ground_with`, a `GroundProgramObserver` | the calling thread, during the call | the function: none; the observer: `Send + 'static` |
| the closure of `ast::parse_string` and `parse_files` | the calling thread | none |
| a `Script`'s methods | the calling thread | `Send + Sync + 'static` |
| the closure of `for_each_model` | the calling thread, between models | none |
| a `SolveEventHandler` | the thread that runs the search | `Send`; also `'static` for the yield and async calls |
| a `Propagator` | the threads that run the search; `init` on the thread that starts it, before it runs | `Send + Sync` |
| the logger | any thread | `Send + 'static` |

Which thread runs a search depends on the call and the build:

- **`solve_yield` and the calls built on it**, `for_each_model`, `solve_first`,
  `solve_optimal`, `solve_all` and their `_with` forms, run the search inside the
  handle's calls, on the calling thread.
- **`Control::solve`, and `solve_with` and `solve_with_events` without a timeout**,
  with one solver thread, run the search inside the call, on the calling thread, when
  nothing can interrupt it: on a build without threads, or when no `InterruptHandle`
  of the control exists. On a build with threads, when one exists, or with a timeout,
  the search runs on a thread of clasp's own while the calling thread waits, so that
  the interrupt can reach it. On a build with threads, the control that
  `Application::main` receives always solves on clasp's thread.
- **`solve_async`** runs the search on a thread of clasp's own.
- **With more than one solver thread** (`ControlBuilder::threads`, or clingo's
  `--parallel-mode`), every solver thread runs part of the search, and propagators and
  solve-event handlers are called from whichever thread is at work.

Write callbacks for any thread their bounds allow, and do not rely on which one runs
them: the choice can change with the options and the build. The bounds stay the same
in every case. clingox calls a `SolveEventHandler` and the logger one call at a time.
A `Propagator` is called from several solver threads at once unless it was registered
with `register_propagator_sequential`; [Propagators](propagators.md) explains how to
keep its state.

A search on the calling thread uses that thread's stack. Grounding always does, and
deeply nested programs need a large one. clasp's search itself needs little: for the
three programs the repository tested, it ran on a 32 KiB stack in a debug build, the
smallest size tried.

## Interrupts reach only the running search

clingo keeps an interrupt that arrives while nothing is solving, and stops the next
solve call at its start. clingox does not let that happen: `InterruptHandle::interrupt`
acts only while a solve call runs, under a lock that the control also takes when it
starts a search and when it is dropped. An interrupt can therefore not reach a later
search, nor a control that has already been freed. About 400,000 racing solves under
ThreadSanitizer back this up. [Set a time budget](../how-to/time-budget.md) shows the
handle in use.

An interrupted search is never reported as unsatisfiable or exhausted, because clingo
5.8.2 sometimes reports an interrupted search of a satisfiable program as
unsatisfiable.

## Several controls on several threads

Separate controls on separate threads share no state that clingox manages, and
clingox takes no process-wide lock. Two shared things in clasp itself matter:

- clasp writes a few process-wide values that every control shares. Each write stores
  the value already there, or a flag bit that no reader consults for that value, and
  33,600 concurrent solves matched a single-threaded reference exactly, so clingox
  accepts this known race.
- clasp registers each kind of statistic in a process-wide list the first time it is
  used, without a lock. The vendored build with the `threads` feature patches this.
  With the feature off, or with a system clingo, do not let controls on several
  threads solve at once; see
  [Known issues](../reference/known-issues.md#fixed-in-the-vendored-build-open-with-a-system-clingo).

Scripts registered with `clingox::script::register` and clingo's application are also
process-wide; [Custom scripting languages](../how-to/custom-scripting-languages.md)
and [Clingo applications](../how-to/clingo-applications.md) say what that means for
threads.

## Related pages

- [Errors and panics](errors-and-panics.md) covers poisoning and panics in callbacks.
- [Propagators: literals and reentrancy](propagators.md) covers the threads a
  propagator runs on in detail.
- [Use clingox in a server](../how-to/server.md) applies these rules to a service.
- [Feature flags](../reference/feature-flags.md#threads) says which builds have
  threads.

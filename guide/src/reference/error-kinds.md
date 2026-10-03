# Error kinds

Every fallible call in clingox returns `clingox::Result<T>`, whose error is
[`clingox::Error`](https://docs.rs/clingox/latest/clingox/struct.Error.html). Its
`kind()` is an
[`ErrorKind`](https://docs.rs/clingox/latest/clingox/enum.ErrorKind.html). This page
lists every kind: what produces it, whether it poisons the `Control`, and what to do
about it. [Errors and panics](../concepts/errors-and-panics.md) explains the error
type itself, the messages it carries, and why some errors poison a control.

A poisoned control answers every later call with `ErrorKind::Poisoned`. The only
thing left to do with it is to drop it and create a new one.

## Summary

| Kind | Displayed as | Poisons the control |
|---|---|---|
| [`Parse`](#parse) | `parse error` | yes, when a control method raised it |
| [`Runtime`](#runtime) | `runtime error` | no, with the exceptions listed below |
| [`Logic`](#logic) | `logic error` | yes |
| [`BadAlloc`](#badalloc) | `out of memory` | yes |
| [`Unknown`](#unknown) | `unknown error` | yes |
| [`Callback`](#callback) | `callback error` | depends on the callback, see [below](#errors-your-own-code-returns) |
| [`Conversion`](#conversion) | `conversion error` | no, unless a grounding callback returned it |
| [`Utf8`](#utf8) | `invalid UTF-8` | no |
| [`Nul`](#nul) | `NUL byte in a string` | no, unless a grounding callback returned it |
| [`Poisoned`](#poisoned) | `poisoned control` | the control already is |
| [`Version`](#version) | `clingo version mismatch` | no |
| [`Unsupported`](#unsupported) | `unsupported operation` | no |
| [`InvalidInput`](#invalidinput) | `invalid input` | no, unless a grounding callback returned it |
| [`GroundingLimit`](#groundinglimit) | `grounding-size limit exceeded` | yes, always |

The second column is the `Display` text of the `ErrorKind` itself. The `Display` text
of an `Error` is longer: it names the operation and gives clingo's message.

`ErrorKind` is `#[non_exhaustive]`, so a `match` on it needs a wildcard arm. A later
release can add a kind without breaking your code:

```rust
use clingox::{Control, ErrorKind};

fn describe(err: &clingox::Error) -> &'static str {
    match err.kind() {
        ErrorKind::Parse => "the program has a syntax error",
        ErrorKind::Poisoned => "the control must be replaced",
        ErrorKind::Unsupported => "this build of clingo lacks a feature",
        _ => "another error",
    }
}

let mut ctl = Control::new()?;
let err = ctl.add_base("a :- b c.").unwrap_err();
assert_eq!(describe(&err), "the program has a syntax error");
# Ok::<(), clingox::Error>(())
```

## Poisoning in one example

Errors that clingox detects before it calls clingo leave the control as it was. A
syntax error poisons it:

```rust
use clingox::{Control, ErrorKind, Part};

let mut ctl = Control::new()?;

// Refused before clingo sees anything: the control is unharmed.
let err = ctl.add_base("p(\"a\0b\").").unwrap_err();
assert_eq!(err.kind(), ErrorKind::Nul);
let err = ctl.add("__clingox_facts_1", &[], "p.").unwrap_err();
assert_eq!(err.kind(), ErrorKind::InvalidInput);

// A runtime error that leaves clingo intact does not poison either.
let err = ctl.configuration().set("solve.models", "many").unwrap_err();
assert_eq!(err.kind(), ErrorKind::Runtime);

ctl.add_base("p.")?;
ctl.ground(&[Part::base()])?;
assert!(ctl.solve(&[])?.is_sat());

// A syntax error poisons the control.
let err = ctl.add_base("q :- r s.").unwrap_err();
assert_eq!(err.kind(), ErrorKind::Parse);
let err = ctl.solve(&[]).unwrap_err();
assert_eq!(err.kind(), ErrorKind::Poisoned);
# Ok::<(), clingox::Error>(())
```

## Kinds that mirror clingo's error codes

clingo reports a failure with one of four codes. clingox maps them to the first five
kinds, and refines `Runtime` to `Parse` where the failure is a syntax error.

### `Parse`

**Produced by** a syntax error in program text:

- `Control::add`, `Control::add_base` and `Control::load` (a file that opens but
  does not parse);
- `Control::load_aspif`, for a malformed aspif file;
- `ProgramBuilder::add`, for a tree that is not a statement or has a child of the
  wrong kind, and `Control::with_program_builder` once the control has logged an
  error (see [Syntax trees](../concepts/syntax-trees.md));
- functions that parse without a control: `str::parse::<Symbol>()`,
  `ast::parse_string`, `ast::parse_files` and `testing::parse_answer`.

**Poisons** the control when a control method raised it, because clingo cannot ground
after a failed parse. The functions without a control have nothing to poison.

**What to do.** `Error::messages()` gives clingo's messages, each with a `location()`
(file, line and column) where clingo gave one. Fix the text, then create a new
`Control`. A server that accepts program text creates one control per request (see
[Use clingox in a server](../how-to/server.md)).

### `Runtime`

**Produced by** clingo's runtime errors, and by a few failures clingox reports the
same way:

- a failed grounding (`Control::ground`), with clingo's messages attached;
- an unknown configuration path, a value clingo rejects, or a `set` of a map or array
  (see [Configure the solver](../how-to/configuration.md));
- `Control::try_assign_external` on an atom that is not an external;
- `Control::load` and `Control::load_aspif` on a file that cannot be opened;
- a search that cannot start or fails;
- the thread that keeps a timeout failing to start.

**Poisons** the control only in these cases, whatever the kind:

- a failure of `Control::load` other than a missing file or a syntax error, such as a
  `#script` block of a language that is not registered: clingo keeps the rest of the
  file queued;
- a failure of `Control::load_aspif` after every file was opened;
- a failure to restore the model limit after `solve_optimal` or `solve_all`, because
  the control no longer behaves as configured.

**What to do.** Read the message, which names the path, atom or file. A control that
is not poisoned can be used again.

### `Logic`

**Produced by** clingo when it detects wrong use of its API. Through clingox the
documented cases are:

- an unknown or invalid command-line option given to `Control::with_args` or
  `ControlBuilder::args`: clingo 5.8.2 reports it as a logic error, although its
  header documents a runtime error. No control is created then;
- an option specification that clingo rejects when an
  [application](../how-to/clingo-applications.md) registers its own options.

**Poisons** the control, because clingo does not say how much of its state survived.

**What to do.** Check the options against `clingo --help`. clingox checks its own
calls into clingo, so a `Logic` error from any other call on a control is
unexpected; report it with the program that produces it.

### `BadAlloc`

**Produced by** clingo running out of memory. On WebAssembly the default heap is
16 MB and does not grow, so a moderately large grounding fails this way unless the
application allows memory growth (see
[Platform support](platforms.md#memory)).

**Poisons** the control.

**What to do.** Create a new control. The
[grounding-size guard](../concepts/observer.md#the-grounding-size-guard) rejects a
program whose ground program has too many atoms or rules, but it does not bound
memory: grounding can exhaust memory below any such limit (see
[Set a time budget](../how-to/time-budget.md#grounding-has-no-time-budget)). For
programs you do not control, set a memory limit on a separate process.

### `Unknown`

**Produced by** clingo failing without an error code clingox knows, or by an
inconsistent answer from clingo, such as a null string where text was expected.

**Poisons** the control.

**What to do.** Create a new control. If you can reproduce it, report it with the
program: it points at a defect in clingo or clingox.

## Kinds from clingox's own checks

### `Callback`

**Produced by** `Error::callback`, which your own code calls to wrap its own error
type. `source()` returns your error, and `downcast_ref` recovers its type.

**Poisons** depending on where the callback ran; see
[Errors your own code returns](#errors-your-own-code-returns).

**What to do.** Inspect `std::error::Error::source(&err)`.
[Errors and panics](../concepts/errors-and-panics.md#errors-from-your-callbacks) has an
example.

### `Conversion`

**Produced by** a value that does not convert between Rust and clingo:

- `FromSymbol` on a symbol of the wrong shape, including through `Model::atoms`,
  `Model::shown` and their `OwnedModel` versions;
- `ToSymbol` on a value that has no symbol, such as an `i64` outside the range of
  `i32`, and `Control::add_facts` with a symbol that is not a fact, such as a number;
- `Error::conversion` in a hand-written `ToSymbol` or `FromSymbol`.

**Poisons** only when a grounding callback returned it.

**What to do.** The message names the value and the type. Check that the Rust type
matches the predicate: name, number of fields and field types.

### `Utf8`

**Produced by** clingo returning text that is not valid UTF-8, for example the file
name in a syntax tree's location after an `#include` of a file whose name is not
UTF-8 (`Ast::span`), a node holding invalid bytes (`Ast::try_to_string`), or
`Control::add_facts` given a symbol that holds such a string, which as text would
become a different fact.

A string symbol read from a file with invalid bytes is not an error: `as_string`
returns it with each invalid sequence replaced by U+FFFD, and displaying the symbol,
or a model that contains it, shows U+FFFD in the same places.

**Poisons** nothing; the check runs on the Rust side.

**What to do.** Fix the encoding of the input file or its name.

### `Nul`

**Produced by** a string with a NUL byte passed to clingo: program text, a part or
parameter name, a path, a string symbol, or a node text. C strings cannot hold one,
so clingox refuses the string before the call.

**Poisons** only when a grounding callback returned it.

**What to do.** Remove or escape the NUL byte.

### `Poisoned`

**Produced by** every call on a control after an error poisoned it. The message names
the error that did, as `the control was poisoned by an earlier error (...)`.

**What to do.** Drop the control and create a new one. Dropping is always safe.

### `Version`

**Produced by** the first call into clingo, and every later one, when the clingo the
program loads is not 5.8.1 or newer within 5.8. The build checks the header;
this check covers a shared library swapped after the build. It can only happen with
a clingo installed on the system.

**Poisons** nothing, but no call into clingo succeeds.

**What to do.** Install a clingo within 5.8 from 5.8.1 on, or build with the
`vendored` feature (see [Feature flags](feature-flags.md#vendored)).
`clingox::version()` reports the linked version.

### `Unsupported`

**Produced by** an operation that needs threads on a build of clingo without them,
such as the default WebAssembly build:

- `ControlBuilder::threads(n)` with `n` above 1;
- a timeout in `SolveOptions`, for `solve_with`, `solve_first_with`,
  `solve_optimal_with`, `solve_all_with` and `solve_with_events`;
- `Control::solve_async` and `solve_async_with_events`.

A timeout needs a second thread to stop the search. `solve_async` needs clingo's
asynchronous mode, which clingo would report as a logic error, which poisons.
clingox refuses both before calling clingo.

**Poisons** nothing. Nothing is solved.

**What to do.** Fall back to a method that works without threads, such as a
conflict limit (see [Set a time budget](../how-to/time-budget.md#without-threads)),
or build with the `threads` feature on a target that has threads.

### `InvalidInput`

**Produced by** input clingox refuses before calling clingo, because clingo would
mishandle it. Examples:

- a part name starting with `__clingox_facts_`, which `add_facts` reserves;
- a thread count outside 1 to 64;
- an index past the end, as in `Configuration::element`;
- a configuration entry of the wrong type, such as `len` of an entry that is not an
  array;
- an attribute a syntax tree node does not have, the wrong accessor for it, or an
  edit that would make the tree cyclic;
- a file path that is not valid UTF-8.

A NUL byte is `Nul`, not `InvalidInput`.

**Poisons** only when a grounding callback returned it.

**What to do.** The message names the value and the rule it broke.

### `GroundingLimit`

**Produced by** a grounding-size guard when the ground program exceeds its count of
atoms or rules: `Control::ground_with_limit`, or a registered `LimitedObserver`,
during `ground`. See
[the grounding-size guard](../concepts/observer.md#the-grounding-size-guard). The
guard counts only the size of the ground program; it does not bound the time or
memory grounding takes.

**Poisons** the control, always. clingo keeps whatever it ground before the guard
stopped it, and would answer a later solve from that truncated program without any
sign that it is incomplete.

**What to do.** Create a new control, and either reject the input or raise the limit.

## Errors your own code returns

A callback can return any `Error`: one from `Error::callback`, `Error::conversion` or
`Error::new(kind, ...)`, or one that a clingox call inside it returned. The error comes
back from the clingox call unchanged. Whether it poisons the control depends on where
the callback ran, not on its kind:

| Where your code ran | Poisons |
|---|---|
| the function of `Control::ground_with` | yes, it stops grounding partway |
| a `GroundProgramObserver` callback | yes, wherever clingo calls it |
| `Propagator::init` | yes |
| a `Script`'s `execute`, `callable` or `call` | yes |
| the closure of `Control::for_each_model` | no |
| a `SolveEventHandler` method | no |
| `Propagator::propagate`, `undo`, `check` or `decide` | no |

A panic in a callback follows the same rows: it resumes on your thread when the call
returns, and poisons the control only where an error would. A panic in the logger
does not poison. [Errors and panics](../concepts/errors-and-panics.md#panics-in-callbacks)
explains why.

## Not errors

An interrupt or a timeout is not an error. The search returns a result that says it
was interrupted (`SolveResult::is_interrupted`), and
[`Outcome::Unknown`](https://docs.rs/clingox/latest/clingox/enum.Outcome.html) when no
model was found. A search stopped by a conflict limit such as `--solve-limit` is
unknown and not interrupted. Neither poisons the control.

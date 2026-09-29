# Errors and panics

clingo is a C++ library behind a C interface. It reports failure through return
codes and a per-thread message, and it cannot let a Rust panic pass through its
frames. This chapter explains how clingox turns that into Rust errors, when an error
leaves a `Control` unusable, and what happens to errors and panics in your own
callbacks.

## One error type

Every fallible call returns `clingox::Result<T>`, whose error is `clingox::Error`. The
design follows `std::io::Error`:

- `kind()` returns an `ErrorKind` to match on;
- `Display` gives one line for people, naming the operation and its subject, then
  clingo's message:
  ``parsing program `base`: <block>:1:8-9: error: syntax error, unexpected <IDENTIFIER>``;
- `Debug` shows everything: the kind, the context, clingo's message, the messages it
  logged, and the source;
- `source()` returns the error of your own code that caused it, if any.

`Error` is `Send`, `Sync` and `'static`, so it moves across threads and fits into
`Box<dyn std::error::Error + Send + Sync>` and the error types built on it.

The kinds fall into two groups. Five mirror clingo's own error codes:

| Kind | Meaning |
|---|---|
| `Parse` | a program or term has a syntax error |
| `Runtime` | clingo reported a runtime error, such as a failed grounding |
| `Logic` | clingo reported wrong use of its API |
| `BadAlloc` | clingo ran out of memory |
| `Unknown` | clingo failed without saying why |

The others come from clingox's own checks: `Callback` (your callback failed),
`Conversion` (a value has no symbol, or a symbol does not fit the type), `Utf8`
(clingo returned text that is not valid UTF-8), `Nul` (a string with a NUL byte,
which C cannot take), `Poisoned`, `Version` (the linked clingo is not 5.8.1 or newer
within 5.8), `Unsupported` (the build lacks a feature, such as threads on
WebAssembly), `InvalidInput` (clingox refused a value before calling clingo, such
as a part name reserved for `add_facts` or a thread count outside 1 to 64) and
`GroundingLimit` (a [grounding-size guard](observer.md#the-grounding-size-guard)
was exceeded).
`ErrorKind` is non-exhaustive, so a `match` on it needs a wildcard arm; new kinds can
be added without breaking your code.

An interrupt or a timeout is not an error. A stopped search returns its result, which
says it was interrupted (`SolveResult::is_interrupted`, `Outcome::Unknown`).

## Messages with locations

While clingo parses and grounds, it logs messages: warnings, and the details of an
error. clingox captures the messages of each call. When the call fails, they are
attached to the error, each with its position in the program:

```rust
use clingox::{Control, ErrorKind};

let mut ctl = Control::new()?;
let err = ctl.add_base("a :- b c.").unwrap_err();
assert_eq!(err.kind(), ErrorKind::Parse);
for message in err.messages() {
    let at = message.location().expect("a parse error has a position");
    println!("line {}, column {}: {}", at.line(), at.column(), message.text());
}
# assert_eq!(err.messages()[0].location().map(|l| (l.line(), l.column())), Some((1, 8)));
# Ok::<(), clingox::Error>(())
```

Every message also goes to the logger set with `ControlBuilder::logger`, or, without
one, to the `log` crate under the target `clingox`.

## Poisoning

Some errors leave clingo's control object in a state it cannot recover from. After
such an error, the `Control` is **poisoned**: every later call returns an error of
kind `Poisoned`, which names the error that caused it, and the only thing left to do
is to drop the control. Dropping is always safe.

These errors poison:

- a `Parse` error from `add` or `add_base`, because clingo cannot ground a program
  after a failed parse;
- any `Logic`, `BadAlloc` or `Unknown` error, because clingo does not say how much of
  its state survived;
- a failure to close a search;
- **an error or panic that stops `ground` or `ground_with` partway**: a failed or
  panicking `ground_with` function, a failed or panicking
  [`GroundProgramObserver`](observer.md) callback, or `GroundingLimit`. clingo keeps
  whatever it already ground before the failure and answers a later solve from that
  truncated program silently, with nothing marking it incomplete, so this poisons
  whatever kind of error caused it.

These do not: errors that clingox detects before calling clingo (`Nul`, `Conversion`,
`Unsupported`, `InvalidInput`), `Runtime` errors that leave clingo intact (an unknown
configuration key, a failed `try_assign_external`), and errors and panics from a
callback that runs *after* grounding, such as `for_each_model`'s closure.

```rust
use clingox::{Control, ErrorKind, Symbol};

let mut ctl = Control::new()?;
// A conversion error is caught before clingo sees anything.
let err = ctl.add_facts([Symbol::number(1)]).unwrap_err();
assert_eq!(err.kind(), ErrorKind::Conversion);
ctl.add_base("a.")?;

// A parse error poisons.
ctl.add_base("a :- b c.").unwrap_err();
let err = ctl.add_base("b.").unwrap_err();
assert_eq!(err.kind(), ErrorKind::Poisoned);
# Ok::<(), clingox::Error>(())
```

Poisoning is explicit so that a program never continues on a control whose state is
unknown. A server that must survive bad input creates a new `Control` for each
request, or replaces a poisoned one.

A failed grounding callback poisons even though its own error, taken alone, looks
recoverable (here, a plain `Conversion` error your closure returned):

```rust
use clingox::prelude::*;
use clingox::ErrorKind;

let mut ctl = Control::new()?;
ctl.add_base("p(@f()).")?;
let err = ctl
    .ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
        call.push(Symbol::string("a\0b")?)
    })
    .unwrap_err();
assert_eq!(err.kind(), ErrorKind::Nul);
assert!(ctl.ground(&[Part::base()]).is_err(), "the control is poisoned");
# Ok::<(), clingox::Error>(())
```

## Errors from your callbacks

Several calls run your code while clingo works: the closure of `for_each_model`, the
function of `ground_with` that computes `@name(..)` terms, a
[`GroundProgramObserver`](observer.md) callback, and a logger. A closure
that returns `clingox::Result` can stop the work with an error. Wrap your own error
type with `Error::callback`, which keeps it whole:

```rust
use clingox::prelude::*;
use clingox::{Error, ErrorKind};

#[derive(Debug)]
struct TooMany(usize);

impl std::fmt::Display for TooMany {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "more than {} models", self.0)
    }
}

impl std::error::Error for TooMany {}

let mut ctl = Control::with_args(["--models=0"])?;
ctl.add_base("{ a; b; c }.")?;
ctl.ground(&[Part::base()])?;

let mut seen = 0;
let err = ctl
    .for_each_model(&[], |_| {
        seen += 1;
        if seen > 2 {
            return Err(Error::callback(TooMany(2)));
        }
        Ok(ControlFlow::Continue(()))
    })
    .unwrap_err();

assert_eq!(err.kind(), ErrorKind::Callback);
let cause = std::error::Error::source(&err).expect("the callback's error");
assert!(cause.downcast_ref::<TooMany>().is_some());
// The control is not poisoned.
assert!(ctl.solve(&[])?.is_sat());
# Ok::<(), clingox::Error>(())
```

The error comes back from the call unchanged, and `source()` followed by
`downcast_ref` recovers its type. Its `Display` names only the operation, as
`the callback failed`, preceded by what clingox was doing when the call adds it
(``grounding `base`: the callback failed``). It does not repeat your error's text,
which is one step down the chain of sources, so a reporter that prints the chain,
such as `anyhow`, shows that text once. `Debug` shows everything. An error your
closure returns directly, such as a `Conversion` error from `atoms` inside it, keeps
its own kind and text. For `ground_with`, the error also carries the messages clingo
logged while grounding, before your function failed.

In a hand-written `ToSymbol` or `FromSymbol`, report a value that does not convert
with `Error::conversion("...")`, which makes the same `Conversion` error the
derived code does. A `GroundProgramObserver` callback, or `ground_with`'s function,
can also raise a specific `ErrorKind` directly with `Error::new(kind, "...")`,
instead of wrapping a foreign error type with `Error::callback`.

## Panics in callbacks

A panic must not unwind into clingo's C++ frames: that would abort the process. So
clingox catches a panic in any callback, stops the work clingo is doing in an orderly
way, and resumes the panic on your thread when the clingox call returns. To you, the
panic looks as if it came straight from the closure, with its original payload. The
`Control` stays usable afterwards, as it does after a callback error, **except a panic
during grounding** (`ground`'s function or a `GroundProgramObserver` callback), which
poisons it for the same reason a returned error does (see Poisoning, above).

This holds even when clingo runs callbacks on its own solver threads: the panic is
carried back to the thread that made the call.

With `panic = "abort"` in your profile there is no unwinding, so a panic in a
callback aborts the process at the point where it happens. That is sound; it is what
`panic = "abort"` asks for.

## No panics on your input

clingox itself does not panic because of a value you pass it. Bad input is an `Err`:
a NUL byte in a name, a number out of range, a symbol that is not a fact, an unknown
option. A panic from inside clingox means a bug in clingox. The exceptions are the
testing helpers: `assert_models!` panics on a mismatch, because that is its job.

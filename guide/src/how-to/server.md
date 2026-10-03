# Use clingox in a server

A server that answers requests with clingo has to keep one request from harming the
others: a bad program must not break shared state, a hard one must not run forever,
and a large one must not exhaust memory. Inside one process, clingox can keep errors
to the request that caused them, stop a search after a time budget, and bound the
size of a ground program. It cannot bound the time or memory that grounding takes,
and it cannot catch a stack overflow. This page says what that means for program text
from clients, then shows how to structure, bound and answer the requests a process
handles.

## Programs from clients: use a separate process

If clients send rules, not only data, solve their programs in a separate process with
limits from the operating system on memory, CPU time and file access, and replace the
process from time to time. Within one process, a program can do all of this:

- **Use unbounded time and memory while grounding.** The grounding-size guard counts
  the atoms and rules of the ground program and nothing else. A 47-byte program,
  `p(1..8000). q :- #sum { X,Y : p(X), p(Y) } > 1.`, stays far below a limit of
  100,000 atoms and rules, and was killed at a 4 GB memory cap after 44 seconds of
  grounding (see [Set a time budget](time-budget.md#grounding-has-no-time-budget)).
  Grounding cannot be interrupted.
- **Overflow the stack.** clingo handles nested terms recursively, and a stack
  overflow aborts the process. In a release build, `Control::add_base` of a fact
  `p(f(f(...)))` nested 22,500 levels deep aborted on a thread with a 2 MiB stack
  (20,000 levels were added), and 90,000 levels on an 8 MiB stack (80,000 were added).
  Grounding such a fact aborted sooner: at 20,000 levels on 2 MiB and 70,000 on
  8 MiB.
- **Read files on the server.** An `#include "path".` directive reads that file,
  whether the text goes through `Control::add` or `ast::parse_string`, and its contents
  become part of the program and of the answer. An `#include` of a file that does not
  exist fails with `file could not be opened`, so the replies also tell which files
  exist.
- **Run your scripts.** A `Script` registered with `clingox::script::register` runs
  for every `#script (name)` block of its language, and answers `@f(...)` terms in
  any program on any control of the process.
- **Exhaust memory in other ways**, such as with a huge `#project` arity (see
  [Known issues](../reference/known-issues.md#behaviour-to-know-about)).
- **Crash a system clingo.** Without the `vendored` feature, a few inputs, such as some
  integer divisions in program text, end the process.
- **Grow the symbol table.** Every distinct symbol a request creates stays in clingo's
  global table until the process exits, so a process that sees ever new names and
  strings grows without bound.

Where you can, keep the rules on the server and accept data: `add_facts` with your own
types turns a request into facts that cannot contain directives. The rest of this page
is about the work inside one process, whether it runs your own programs or a client's
inside a sandbox.

## One control per request

The simplest structure creates a new `Control` for each request and drops it at the
end. Errors that poison a control then never outlive the request that caused them,
and requests share no state. Creating a control is cheap next to grounding and solving
a real program: measured in a release build for this guide, creating one took about
47 microseconds, and creating, grounding and solving a one-fact program and dropping
the control about 85 (median of 5 000, one core).

This handler takes program text from a request, bounds the size of its ground program
and the time of its search, and turns each outcome into a reply:

```rust
use std::time::Duration;

use clingox::observer::GroundingLimit;
use clingox::prelude::*;
use clingox::{ErrorKind, SolveOptions};
# if !clingox_sys::HAS_THREADS { return Ok(()); }

#[derive(Debug, PartialEq)]
enum Reply {
    Answer(Vec<String>),
    NoAnswer,
    TimedOut,
    Rejected(String),
}

fn handle(program: &str) -> clingox::Result<Reply> {
    let mut ctl = Control::new()?;

    // A syntax error or a NUL byte is the client's fault; a syntax error's
    // message has the position.
    if let Err(err) = ctl.add_base(program) {
        return match err.kind() {
            ErrorKind::Parse | ErrorKind::Nul => Ok(Reply::Rejected(err.to_string())),
            _ => Err(err),
        };
    }

    // Bounds the size of the ground program only, not grounding's time or memory.
    let limit = GroundingLimit::new(Some(100_000), Some(100_000));
    if let Err(err) = ctl.ground_with_limit(&[Part::base()], limit) {
        return match err.kind() {
            ErrorKind::GroundingLimit | ErrorKind::Runtime => Ok(Reply::Rejected(err.to_string())),
            _ => Err(err),
        };
    }

    let options = SolveOptions::new().timeout(Duration::from_secs(2));
    Ok(match ctl.solve_first_with(options)? {
        Outcome::Sat(model, _) => {
            Reply::Answer(model.symbols().iter().map(ToString::to_string).collect())
        }
        Outcome::Unsat => Reply::NoAnswer,
        Outcome::Unknown(_) => Reply::TimedOut,
    })
}

assert_eq!(handle("a. b :- a.")?, Reply::Answer(vec!["a".into(), "b".into()]));
assert_eq!(handle("a. :- a.")?, Reply::NoAnswer);
assert!(matches!(handle("a :- b c.")?, Reply::Rejected(_)));
assert!(matches!(handle("a.\0b.")?, Reply::Rejected(_)));
assert!(matches!(handle("p(1..1000000).")?, Reply::Rejected(_)));
# Ok::<(), clingox::Error>(())
```

The errors the handler passes on with `?` are the ones that are not the client's
fault, such as `ErrorKind::BadAlloc`; a server reports them as its own failure. The
grounding guard only rejects programs whose ground program is too large; it does not
keep this handler from grounding for a long time or using much memory.
[Error kinds](../reference/error-kinds.md) says what produces each kind, and
[Set a time budget](time-budget.md) compares the ways to bound a search.

## Ground once, answer many queries

When the rules and most of the data are the same for every request, grounding them for
each request repeats the same work. Ground them once, and let each request differ only
in its assumptions (see
[Solving step by step](../tutorial/solving-step-by-step.md)); here each request
assumes a different start node. A `Control` is `Send` but
not `Sync`: one thread at a time may use it, so threads share it behind a `Mutex`:

```rust
use std::sync::{Arc, Mutex};
use std::thread;

use clingox::prelude::*;
# if cfg!(all(target_family = "wasm", not(target_feature = "atomics"))) { return Ok(()); }

#[derive(FromSymbol)]
struct Reach(i32);

let mut ctl = Control::new()?;
ctl.add_base(
    "edge(1, 2). edge(2, 3). edge(3, 4).
     node(N) :- edge(N, _). node(N) :- edge(_, N).
     { start(N) : node(N) } 1.
     reach(N) :- start(N).
     reach(M) :- reach(N), edge(N, M).",
)?;
ctl.ground(&[Part::base()])?;
let shared = Arc::new(Mutex::new(ctl));

// Four requests at once, each asking how many nodes one node reaches.
let requests: Vec<_> = (1..=4)
    .map(|start| {
        let shared = Arc::clone(&shared);
        thread::spawn(move || -> clingox::Result<usize> {
            let start = Symbol::function("start", &[Symbol::number(start)])?;
            let options = SolveOptions::new().assumptions(&[(start, true).into()]);
            let mut ctl = shared.lock().expect("no request panics while it holds the lock");
            match ctl.solve_first_with(options)? {
                Outcome::Sat(model, _) => Ok(model.atoms::<Reach>()?.len()),
                _ => Ok(0),
            }
        })
    })
    .collect();

let counts: Vec<usize> = requests
    .into_iter()
    .map(|request| request.join().expect("the request does not panic"))
    .collect::<clingox::Result<_>>()?;
assert_eq!(counts, [4, 3, 2, 1]);
# Ok::<(), clingox::Error>(())
```

The lock serialises the searches. For more throughput, give each worker thread its
own grounded control. A shared control can be poisoned by one request, and then
answers every later one with `ErrorKind::Poisoned`; replace it under the lock when
that happens. Keep requests that bring their own program text away from a shared
control.

## Async runtimes

clingox has no `async` API: grounding and solving block the calling thread, and
`AsyncSolveHandle` is not a `Future`. In an async server, run each request on a thread
meant for blocking work, such as the blocking pool of your runtime, and keep the
`Control` on that thread for the length of the request. To give up on a request whose
client has gone away, keep an `InterruptHandle` where the async code can reach it:
it is `Send`, `Sync` and `'static`, and stops the running search
([Set a time budget](time-budget.md#stop-a-search-from-another-thread)).

By default each control solves with one thread, so a request occupies about one core
while it solves. `ControlBuilder::threads(n)` uses `n` solver threads for one search,
which multiplies that.

## Logging per request

Give each control a logger that knows the request it serves. The logger receives every
message clingo logs, may run on any thread, and must be `Send` and `'static`:

```rust
use clingox::prelude::*;

let request_id = 17;
let mut ctl = Control::builder()
    .logger(move |code, text| eprintln!("request {request_id}: {code:?}: {text}"))
    .build()?;
ctl.add_base("a :- b.")?; // clingo warns that `b` is never defined
ctl.ground(&[Part::base()])?;
# Ok::<(), clingox::Error>(())
```

With a logger set, the messages no longer go to the `log` crate. Either way, the
messages of a failed call are attached to its error.

## Related pages

- [Errors and panics](../concepts/errors-and-panics.md) explains poisoning and what
  happens to a panic in a callback.
- [Safety and threads](../concepts/safety-and-threads.md) lists which types can cross
  threads and where callbacks run.
- [Observing the ground program](../concepts/observer.md#the-grounding-size-guard)
  describes the grounding-size guard.

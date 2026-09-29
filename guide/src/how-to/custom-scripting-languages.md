# Custom scripting languages

clingo lets a program call out to a scripting language: a
`#script (name) ... #end.` block holds code, and a term such as `@f(1)` calls a
function while the program is grounded. clingo ships Python and Lua support. With
clingox you add your own language in Rust, by implementing the `Script` trait and
registering it once for the process.

## A small language

A script is a value with four methods. Only `execute` is required. It runs the
code of one block while the program is parsed. `callable` says which function
names the script owns, and `call` evaluates one call and returns the values, which
clingo treats as a pool:

```rust,standalone_crate
use std::collections::HashMap;
use std::sync::Mutex;

use clingox::ast::Span;
use clingox::prelude::*;
use clingox::script::{self, Script};

/// `#script (vars) x = 3 #end.` defines a variable, `@get(x)` reads it.
#[derive(Default)]
struct Vars(Mutex<HashMap<String, i32>>);

impl Script for Vars {
    fn execute(&self, _span: &Span, code: &str) -> clingox::Result<()> {
        for line in code.lines().filter(|l| !l.trim().is_empty()) {
            let (name, value) = line.split_once('=').ok_or_else(|| {
                clingox::Error::new(clingox::ErrorKind::InvalidInput, "expected `name = value`")
            })?;
            let value = value.trim().parse().map_err(clingox::Error::callback)?;
            self.0.lock().unwrap().insert(name.trim().to_owned(), value);
        }
        Ok(())
    }

    fn callable(&self, name: &str) -> clingox::Result<bool> {
        Ok(name == "get")
    }

    fn call(&self, _span: &Span, _name: &str, arguments: &[Symbol]) -> clingox::Result<Vec<Symbol>> {
        let variable = arguments.first().and_then(Symbol::name).unwrap_or_default();
        let value = self.0.lock().unwrap().get(variable).copied();
        // No symbol means the term has no value and the rule instance is dropped.
        Ok(value.map(Symbol::number).into_iter().collect())
    }
}

script::register("vars", "1.0", Vars::default())?;

let mut ctl = Control::new()?;
ctl.add_base("#script (vars) x = 3 #end. p(@get(x)). q(@get(y)).")?;
ctl.ground(&[Part::base()])?;
let (_, models) = ctl.solve_all()?;
clingox::testing::assert_models!(models, ["p(3)"]);
# Ok::<(), clingox::Error>(())
```

`q(@get(y))` has no value, so its rule instance is dropped and no error is
raised. Returning three symbols would make three instances.

## Register before anything else

`script::register` fails with `ErrorKind::InvalidInput` once the process has
created a `Control`, even one that failed to build or was dropped, or has started
an `Application`. clingo's script registry is not synchronised with grounding, so
adding to it while another thread grounds would be a data race. Register your
languages at the start of `main`, or in a `Once` that every entry point goes
through.

Other rules of registration:

- A name can be registered once; a second registration is refused, because clingo
  would run every block of both.
- A script is `Send + Sync + 'static` and takes `&self`, because every control in
  the process shares it and two threads that ground at the same time call it at
  the same time.
- A script is never dropped. It lives until the process exits, and its `Drop`
  does not run. clingo would call a `free` callback during static destruction,
  when Rust may already be gone, so clingox does not install one.
- `python` and `lua` are the names clingo's `--version` text looks for.
  Registering them makes it print `with Python <version>` and `with Lua
  <version>`; the languages themselves are not provided.
- Only lowercase identifiers can be written as `#script (name)`. Other names
  register, and `script::version` finds them, but a program cannot use them.

## What runs when, and where

`execute` runs while the program is added (`Control::add`, `Control::load`, a
program builder, or a run's parse), once per block, in source order, before
anything is grounded. `callable` and `call` run while grounding, for every `@`
term; `callable` is asked right before each `call`, never remembered, so keep it
cheap. All of them run on the thread that made the call, never on a solver
thread.

Every language that has run a block is asked about every `@` term for the rest
of the process, in registration order, and the first whose `callable` says yes
gets the call. Answer `callable` only for names of your own language.

A ground callback wins. `Control::ground_with` asks its callback first for every
`@` term and never falls through to a script, whatever the callback does. Use
`Control::ground` when a script should answer, or call your script from the
callback yourself.

## Errors

An error a script returns comes back from the call that ran it, with its kind
unchanged. It is not turned into a `Parse` error the way clingo's own syntax
errors are.

- An error from `execute` stops the parse and poisons the control, whatever its
  kind: clingo keeps the rest of the failed program queued and would parse it at
  the next `add`.
- An error from `callable` or `call` stops grounding and poisons the control.
- A panic in any of them is caught, poisons the control, and resumes on the
  calling thread when the call returns. It never unwinds through clingo.

A poisoned control is replaced with a new one; see
[Errors and panics](../concepts/errors-and-panics.md).

## A script `main`

`Script::main` runs instead of clingo's own ground-and-solve when you run
clingo's command line through `Application::run` with no `main` callback, and
the script answers `callable("main")` with true. It receives the same control an
`Application::main` callback does, with the same limits: it cannot be kept, and
it is finished when `main` returns. The files are already parsed when it starts.

```rust,standalone_crate
use std::sync::atomic::{AtomicBool, Ordering};

use clingox::application::{Application, exit_code};
use clingox::ast::Span;
use clingox::prelude::*;
use clingox::script::{self, Script};

struct Driver(AtomicBool);

impl Script for Driver {
    fn execute(&self, _span: &Span, _code: &str) -> clingox::Result<()> {
        Ok(())
    }

    fn callable(&self, name: &str) -> clingox::Result<bool> {
        Ok(name == "main" && self.0.load(Ordering::SeqCst))
    }

    fn main(&self, control: &mut ScopedControl<'_>) -> clingox::Result<()> {
        control.add_base("b.")?;
        control.ground(&[Part::base()])?;
        let (_, models) = control.solve_all()?;
        assert_eq!(models.len(), 1);
        Ok(())
    }
}

script::register("driver", "1", Driver(AtomicBool::new(true)))?;
let path = std::env::temp_dir().join(format!("clingox-guide-script-{}.lp", std::process::id()));
std::fs::write(&path, "#script (driver) #end. a.")?;
let file = path.to_string_lossy().into_owned();
let code = Application::new().run([file.as_str(), "--outf=3"])?;
// The script's main ran instead of clingo's and solved the control it
// was given: one model, all enumerated.
assert_eq!(code, exit_code::SATISFIABLE | exit_code::EXHAUSTED);
# std::fs::remove_file(&path)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

An `Err` from `main` makes `run` return that error with its kind unchanged
(clingo itself prints `*** ERROR: (clingo): <message>` and exits with 65), and a
panic resumes from `run`.

Two things to know. An `Application` with its own `main` callback always wins
over a script's. And a script that answers `callable("main")` with true takes
over **every** later default run in the process, also over files without a
`#script` block: clingo asks only the languages that have run a block and never
forgets that. A script that must not do this answers false unless it means to
run, as the switch above does.

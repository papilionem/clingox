# Clingo applications

Sometimes you want clingo's whole command line, not a `Control` you drive
yourself: the same files, options, output and exit codes as the `clingo`
executable, but inside your own program, with your own name and version and
your own handling of clingo's messages. That is what `python -m clingo` is,
and it is what [`Application`](https://docs.rs/clingox/latest/clingox/application/struct.Application.html) gives you in Rust.

This chapter covers the application layer: the default command line with a
program name, a version, a message limit and a logger, your own `main`
callback that drives a control under clingo's command line, your own
command-line options, and your own model printer. It ends with the
things to know before you use it in a program that does more than start
clingo.

## Run clingo's command line

Build an `Application`, set what you want to change, and call `run` with the
arguments you would type after `clingo`, without the program name:

```rust,standalone_crate
use clingox::application::{Application, exit_code};

let dir = std::env::temp_dir();
let path = dir.join(format!("clingox-guide-app-{}.lp", std::process::id()));
std::fs::write(&path, "a. {b}.")?;
let file = path.to_string_lossy().into_owned();

// `0` asks for every model, and `--outf=3` silences clingo's output so that
// the example prints nothing.
let code = Application::new().run([file.as_str(), "0", "--outf=3"])?;
assert_eq!(code, exit_code::SATISFIABLE | exit_code::EXHAUSTED);
# std::fs::remove_file(&path)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Without `--outf=3` clingo prints what the executable prints: the banner, the
models, the statistics. That output goes to the process's standard output
through C stdio. `run` flushes Rust's `stdout()` and `stderr()` and C stdio
before and after the run, so text you printed earlier and clingo's text stay
in order. The C stdio flush exists on Unix targets only (Linux, Android and
WebAssembly under Emscripten all count); on a target without it only Rust's
streams are flushed, and the order against clingo's text is not promised.

`run` returns clingo's exit code as an `i32`. The constants are in
`exit_code` and they are bit flags: 30 is `SATISFIABLE | EXHAUSTED`, a search
that found models and enumerated all of them; 10 is a model found with more
possible; 20 is unsatisfiable. A mistake on the command line is an exit code,
not a Rust error: an unknown option returns `Ok(1)` and a missing input file
`Ok(65)`, with clingo's explanation already on standard error. An `Err` is
reserved for what `run` itself refuses (see below).

The arguments can be anything that converts to `OsStr`: `&str`, `String`,
`Path`, `PathBuf`, `OsString`. A path needs no conversion, and text that is not
valid UTF-8 is refused with `ErrorKind::InvalidInput`:

```rust,standalone_crate
use std::path::PathBuf;

use clingox::application::Application;

let path: PathBuf = std::env::temp_dir().join(format!("clingox-guide-app3-{}.lp", std::process::id()));
std::fs::write(&path, "a.")?;
let arguments = vec![path.clone().into_os_string(), "--outf=3".into()];
assert_eq!(Application::new().run(arguments)?, 30);
# std::fs::remove_file(&path)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

To forward your own program's arguments, pass `std::env::args_os().skip(1)`:
skip the first one, which is the executable, or it becomes an input file.

With no file argument clingo reads the program from standard input, as the
executable does.

## Your own name, version and message limit

The name and version appear in `--help` and `--version` and in messages. The
message limit is how many messages clingo reports before it stops (20 by
default, 0 for none):

```rust,standalone_crate
use clingox::application::Application;

let dir = std::env::temp_dir();
let path = dir.join(format!("clingox-guide-app2-{}.lp", std::process::id()));
std::fs::write(&path, "a :- b. c :- d. e :- f.")?;
let file = path.to_string_lossy().into_owned();

let mut heard = 0;
let code = Application::new()
    .program_name("checker")
    .version("1.2.3")
    .message_limit(2)
    .logger(|_code, _text| heard += 1)
    .run([file.as_str(), "--outf=3"])?;

assert_eq!(code, 30);
assert_eq!(heard, 2, "three atoms are undefined, the limit lets two through");
# std::fs::remove_file(&path)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`checker --version` would now print `checker version 1.2.3` as its first line.

## Handle clingo's messages yourself

By default clingo writes its warnings and errors to standard error. With a
`logger`, they go to your closure instead, and nothing is written to standard
error for them. The closure receives a [`MessageCode`] and the text, with its
position (`file.lp:1:6-7: info: atom does not occur in any rule head:`) and
without trailing whitespace: the same values a `ControlBuilder` logger gets.

The closure may borrow from your program (it lives no longer than the `run`
call, which does not return until clingo and its threads are done), it must be
`Send`, and clingo calls it one message at a time. It runs on the thread that
emitted the message, which is usually yours but does not have to be. If it
panics, clingo finishes its run normally, no later message reaches the closure,
and `run` then resumes the panic on your thread with the original payload.

## Write your own main

Clingo's default main loads the files, grounds them and solves. With
`Application::main` you do that yourself, with clingo's command line already
applied to the control: options such as `--models` and `--opt-mode`, the
configuration, the statistics summary, the exit code. The closure receives the
control and the positional arguments (the files):

```rust,standalone_crate
use std::ops::ControlFlow;

use clingox::application::Application;
use clingox::{Part, ShowType};

let path = std::env::temp_dir().join(format!("clingox-guide-main-{}.lp", std::process::id()));
std::fs::write(&path, "1 {a; b} 1.")?;
let file = path.to_string_lossy().into_owned();

let mut models = Vec::new();
Application::new()
    .main(|ctl, files| {
        for file in files {
            ctl.load(file)?;
        }
        ctl.ground(&[Part::base()])?;
        let _result = ctl.for_each_model(&[], |model| {
            let atoms = model.symbols(ShowType::SHOWN)?;
            models.push(atoms.iter().map(ToString::to_string).collect::<Vec<_>>());
            Ok(ControlFlow::Continue(()))
        })?;
        Ok(())
    })
    .run([file.as_str(), "0", "--outf=3"])?;
models.sort();
assert_eq!(models, [["a"], ["b"]]);
# std::fs::remove_file(&path)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The closure runs once, on the thread that called `run`, and not at all for
`--help`, `--version` or an unknown option. A missing file does not stop clingo
before `main`: loading is the closure's job, so a bad path is the error your
`load` returns. The closure may borrow from your program, like the logger.

Some things differ from the control you create yourself:

- **The control is borrowed.** clingo owns it and frees it after your closure
  returns. Its type is `ScopedControl<'r>`, where the lifetime `'r` is a brand
  that exists only during the call. Your closure is written for every `'r`, so
  the compiler stops you from keeping the control, or anything you get from it
  (statistics, atoms, configuration, models, search handles), past the call. This
  includes `std::mem::replace(ctl, Control::new()?)`, which would put clingo's
  control in a place that outlives it; it does not compile. Anything you create
  yourself with `Control::new()` is a separate control and can leave the closure
  as usual.
- **Annotating the parameter.** Leave `ctl` unannotated, or write
  `&mut ScopedControl<'_>`. `&mut Control` means `&mut ScopedControl<'static>`,
  the kind you create yourself, and does not accept the callback's control. A
  helper function that should work on both takes `&mut ScopedControl<'_>`.
- **Errors.** If the closure returns `Err`, clingo prints `*** ERROR: (clingo):
  <message>` on standard error and would exit with 65. `run` returns that same
  error to you instead, so the exit code is visible only in that line. A panic in
  the closure is resumed by `run`.
- **Messages.** This control has no message capture, so `Error::messages` is empty
  on it; a syntax error's text goes to the application's logger, or to standard
  error.
- **Observers and propagators** registered in the closure live until clingo has
  finished with them, after the closure returns. A search you left open is
  closed for you before clingo takes the control down.

## Add your own options

`register_options` adds options to clingo's command line, next to `--models`
and the rest. Its closure receives an `Options` and registers each option
with `add`, which takes an `OptionSpec` (the help section, the option's name,
its description) and a closure that receives the option's value as a `&str`.
`validate_options` runs after everything is parsed and can reject the
combination. Both run on the thread that called `run`, as `main` does:

```rust,standalone_crate
use std::cell::RefCell;
use std::ops::ControlFlow;

use clingox::application::{Application, Flag, OptionSpec};
use clingox::{Error, ErrorKind, Part};

let path = std::env::temp_dir().join(format!("clingox-guide-opts-{}.lp", std::process::id()));
std::fs::write(&path, "#program base. a. #program extra. b.")?;
let file = path.to_string_lossy().into_owned();

let program = RefCell::new(String::from("base"));
let loud = Flag::new(false);
let mut grounded = Vec::new();
let code = Application::new()
    .register_options(|options| {
        let spec = OptionSpec::new("Example Options", "program", "The program part to ground")
            .argument("<prog>");
        options.add(spec, |value| {
            if value.is_empty() {
                return Err(Error::new(ErrorKind::InvalidInput, "no program"));
            }
            *program.borrow_mut() = value.to_owned();
            Ok(())
        })?;
        options.add_flag("Example Options", "loud", "Say more", &loud)
    })
    .validate_options(|| {
        // `loud` already holds its final value here.
        assert!(loud.get());
        Ok(())
    })
    .main(|ctl, files| {
        for file in files {
            ctl.load(file)?;
        }
        let name = program.borrow().clone();
        ctl.ground(&[Part::new(&name, &[])?])?;
        let _result = ctl.for_each_model(&[], |_model| Ok(ControlFlow::Continue(())))?;
        grounded.push(name);
        Ok(())
    })
    .run([file.as_str(), "--program=extra", "--loud", "--outf=3"])?;
assert_eq!(grounded, ["extra"]);
assert_eq!(code, 10, "one model was asked for, and it was found");
# std::fs::remove_file(&path)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

What to know:

- **The option key.** The second argument of `OptionSpec::new` is clingo's
  notation: `"name"`, `"name,p"` for a one-letter alias (`-p 3`), `"name,@1"`
  for an option that `--help` hides until `--help=2` (levels 0 to 5), and
  `"name,p,@1"` for both. A key clingo rejects is an `ErrorKind::Logic` error
  from `add` with clingo's message. A name registered twice in the run, or one
  that could never be typed (a leading `-`, an `=`), is `InvalidInput`. Only
  the names of options you register are checked: an option called `models`
  registers, and the run then ends with clingo's `duplicate option` message
  and exit code 1.
- **Values.** The closure receives the text after `=` or the next argument,
  never an empty string (`--name` and `--name=` are command-line errors that
  clingo reports itself). An option given twice is an error unless you call
  `.multi()`, and then the closure runs for each one in command-line order.
  `.argument("<n>")` sets the value's name in `--help`.
- **Refusing a value.** If the closure returns `Err`, parsing stops and `run`
  returns that error unchanged. clingo prints its own line for it (`'zz'
  invalid value for: 'name'`) and drops your message, so `run`'s `Err` is where
  the useful text is. Nothing else runs: not `validate_options`, not `main`.
- **Flags.** `add_flag` takes a `Flag`, a shared boolean. `--loud` sets it,
  `--no-loud` clears it, and it keeps its starting value when the option is
  absent. The value is copied into the `Flag` when parsing is done, before
  `validate_options`, so `validate_options`, `main` and the code after `run` see
  it, and an option's own closure does not. A run that ends earlier (`--help`,
  a bad command line) leaves the `Flag` alone. A `Flag` is `Clone`, `Send` and
  `Sync`, and one `Flag` can be registered once per run.
- **Sharing state.** Two closures of one application cannot borrow the same
  variable mutably, so a value that an option closure writes and `main` or
  `validate_options` reads goes in a `RefCell` or `Cell` declared before the
  application, as `program` does above.
- **Failures.** An `Err` from `register_options` or `validate_options`
  is returned by `run` unchanged, and a panic in any of them is resumed by
  `run`. clingo prints its own message meanwhile. Its exit code for a failed
  `validate_options` is 0, the code of success, which is a quirk of clingo;
  since `run` returns the error, the code does not matter.
- **Help.** Your options appear in `--help` under their section. Descriptions
  are shown literally, including a `%`.

## What a run does to your process

An `Application` is different from the rest of clingox in one way: clingo's
command line is a whole program, and `run` starts it inside your process.

- **Only one run at a time.** clingo keeps process-wide state for the run. A
  second `run`, started from a callback or from another thread while one is in
  progress, returns `ErrorKind::InvalidInput` immediately and touches nothing.
  It never waits, so it cannot deadlock. A `Control` used on another thread
  meanwhile is unaffected.
- **Signals.** While the run is active, clingo's application owns SIGINT,
  SIGTERM and seven other signals. Pressing Ctrl-C then interrupts the search,
  prints clingo's summary and **ends the process with exit code 1**: `run`
  does not return, no destructor runs, no panic hook fires. `run` saves the
  handlers before it starts and puts them back when it returns, so Ctrl-C
  works as before once the call is over. The handlers restored are the ones saved when
  the run began, so install your own before or after `run`, not while it is in
  progress (a handler installed meanwhile is overwritten). A signal your process ignores (a job started with `&` or `nohup` ignores SIGINT
  and SIGQUIT) stays ignored during the run. (Without that, clingo's handler would
  stay installed and crash the process on the next signal.)
- **Time limits end the process the same way.** `--time-limit=N` raises a
  signal after N seconds, and the result is the same: the summary, then exit
  code 1, without returning. A search that finishes in time returns normally.
  If your program has work to do after the solver, run the application in a
  child process instead of with `--time-limit`.
- **Options that end the process are refused.** Besides `--time-limit`, a few
  command lines make clingo leave the process from inside the run, so `run`
  looks for them first and returns `ErrorKind::InvalidInput` before anything
  happens. They are `--fast-exit` and its abbreviations (`--fa` and longer);
  `--pre` (bare, or `--pre=aspif` and `--pre=smodels`), which prints the
  program and leaves; `--print-portfolio` and its abbreviations (`--pri` and
  longer); `--text`, or `--output` (`-o`) with a valid format, together with
  `--mode=clingo` or `--mode=clasp`, and `--text` together with `--output`;
  a `--lemma-out=<file>` that cannot be opened for writing; and an
  `--out-atomf=<format>` that clasp rejects. The last two are found by clasp
  only while it sets up, and it leaves the process rather than reporting them.
  The check follows clasp's own rules for abbreviations and values, so
  `--pre=text` (an invalid value) and `--f` (ambiguous) still reach clingo and
  come back as `Ok(1)`. Other unknown or ambiguous options are clingo's
  business.
- **Platforms.** On Unix, including Android, the signal handling above applies.
  On WebAssembly under Node.js no signal is delivered, but the process-ending
  paths still end the Node process. In a browser, avoid `--time-limit`: ending
  the process there ends the module for good.

The handlers are put back the moment clingo returns, before any `Drop` of
yours runs. A signal aimed at another thread between clingo clearing its
application and that restore can still reach clingo's handler; this is a
defect in clingo that a wrapper cannot close entirely. Callbacks that run
inside the call (`main`, the printer, option callbacks) run with clingo's
handlers installed. The [known issues](../reference/known-issues.md) chapter
has the details.

Two more things to know about the command line. Short options in one argument
are read as clasp reads them, so with a flag registered as `"name,m"`, `-mo
text` means `-m -o text`; an exiting option hidden in such a group is refused
once your `register_options` has run, before clingo parses. And `--mode=clasp`
can be used once per process: clasp opens its input once, so a second run
would answer for the first run's file and `run` returns `InvalidInput`. In
that mode clasp reads a CNF or OPB file itself, `main` and `print_model` are
never called, and `run` returns clasp's exit code (10, 20 or 30).

Inside `main`, output you print with `println!` and clingo's own output use
different buffers; on a pipe your lines can appear before clingo's earlier
ones. Keep your own output out of `main`, or write it to standard error.
`message_limit(0)` does not silence errors: they still reach the logger.

## Print models yourself

`Application::print_model` replaces clingo's text output of each model. The
closure receives the model and a `DefaultPrinter`, clingo's own printer: call
`print()` to get the usual text, wrap it in your own lines, or leave it out to
print nothing for that model. It needs a `main`, because clingo crashes when a
model is read in the printer of an application that has none; without one,
`run` returns `ErrorKind::InvalidInput` before anything starts.

```rust,standalone_crate
use std::sync::Mutex;

use clingox::application::Application;
use clingox::{Part, ShowType};

let path = std::env::temp_dir().join(format!("clingox-guide-print-{}.lp", std::process::id()));
std::fs::write(&path, "1 {a; b} 1.")?;
let file = path.to_string_lossy().into_owned();

let seen = Mutex::new(Vec::new());
Application::new()
    .main(|ctl, files| {
        for file in files {
            ctl.load(file)?;
        }
        ctl.ground(&[Part::base()])?;
        ctl.solve(&[])?;
        Ok(())
    })
    .print_model(|model, _printer| {
        let atoms = model.symbols(ShowType::SHOWN)?;
        seen.lock().unwrap().push(atoms.iter().map(ToString::to_string).collect::<Vec<_>>());
        Ok(())
    })
    .run([file.as_str(), "0", "--verbose=0"])?;
let mut seen = seen.into_inner().unwrap();
seen.sort();
assert_eq!(seen, [["a"], ["b"]]);
# std::fs::remove_file(&path)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Things to know:

- **Threads.** The closure is `Fn + Send + Sync`. It runs on the thread that
  found the model, one call at a time, and it may run while `main` does: with
  several solver threads, or when `main` solves asynchronously. Keep mutable
  state in atomics or a `Mutex`, as above. It is not called at all for `-q`,
  `--outf=2`, `--outf=3`, `--mode=gringo` or `--text`, and with `--outf=1` or
  `--quiet=1` only for the last model.
- **Order of output.** clingo writes through C stdio and `println!` through
  Rust's own buffer. `print_model` flushes both around your closure and around
  `print()`, so what you print and what clingo prints come out in program
  order, also on a pipe or a file. This holds where C stdio can be flushed,
  which is every Unix target; elsewhere only Rust's streams are flushed and the
  order is not promised. Do not make the closure wait for another
  thread that prints through C stdio, and do not start another `run` in it
  (that is refused).
- **Holding stdout.** If `main` holds `std::io::stdout().lock()` while an
  asynchronous solve runs, the printer's flush on the solver thread waits for
  that lock and the run deadlocks, even when the closure prints nothing. Lock
  stdout only around a single write.
- **Failure.** If the closure returns an error or panics, or `print()` fails,
  the first failure is kept, nothing more is printed for later models, the
  search is interrupted through the control `main` received, and `run` returns
  the error unchanged (or resumes the panic). clingo itself is told the call
  succeeded.

[`MessageCode`]: https://docs.rs/clingox/latest/clingox/enum.MessageCode.html

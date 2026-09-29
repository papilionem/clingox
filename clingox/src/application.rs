//! Running clingo's own command line from Rust, with your own name, version
//! and message handling.
//!
//! An [`Application`] describes a program that behaves like the `clingo`
//! executable (the same thing `python -m clingo` is), and [`Application::run`]
//! starts clingo's command line with it. Everything clingo's command line does
//! is available: reading files, the many options, solving, printing models and
//! statistics, and the exit codes in [`exit_code`].
//!
//! ```standalone_crate
//! use clingox::application::{Application, exit_code};
//!
//! let path = std::env::temp_dir().join(format!("clingox-app-doc-{}.lp", std::process::id()));
//! std::fs::write(&path, "a. {b}.")?;
//! let file = path.to_string_lossy().into_owned();
//!
//! // `--outf=3` silences clingo's output; `0` asks for every model.
//! let code = Application::new().run([file.as_str(), "0", "--outf=3"])?;
//! assert_eq!(code, exit_code::SATISFIABLE | exit_code::EXHAUSTED);
//! # std::fs::remove_file(&path)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Process-wide effects
//!
//! Unlike the rest of this crate, a run is a whole program inside a call, and
//! it changes the process:
//!
//! - **One run at a time.** clingo keeps a process-wide singleton for the run.
//!   A second [`run`](Application::run) while one is in progress, on any
//!   thread (nested inside a callback, or concurrent), returns
//!   [`ErrorKind::InvalidInput`](crate::ErrorKind::InvalidInput) at once; it
//!   never waits. A [`Control`](crate::Control) used on another thread at the
//!   same time is not affected.
//! - **Signals.** While a run is active, clingo's application owns SIGINT,
//!   SIGTERM, SIGUSR1, SIGUSR2, SIGQUIT, SIGHUP, SIGXCPU, SIGXFSZ and SIGALRM. A signal
//!   the process ignores when the run starts stays ignored (a job started
//!   with `&` or `nohup` ignores SIGINT and SIGQUIT), so it does nothing. Any
//!   other signal in that time interrupts the search,
//!   prints clingo's summary and **ends the process** with exit code 1,
//!   without returning to Rust. The dispositions are saved before the run and
//!   put back after it, so a Ctrl-C after `run` returns behaves as before the
//!   call. The dispositions written back are the ones saved at entry, so a
//!   handler another thread installs for one of these signals while a run is
//!   in progress is overwritten: install your handlers before or after `run`,
//!   not during it. The dispositions go back the moment `clingo_main`
//!   returns, before any of your `Drop` code runs (observers, propagators and
//!   option closures the run held are dropped after that), so a signal during
//!   such a `Drop` behaves as it did before the run. Two windows stay open. A
//!   signal delivered to another thread between clingo clearing its
//!   application and the restore runs clingo's handler with a null instance
//!   and crashes. And every callback that runs inside the call (`main`, the
//!   model printer, option callbacks, the logger, script code) runs with
//!   clingo's handlers installed.
//! - **The process can end inside `run`.** Besides signals, `--time-limit`
//!   (which raises SIGALRM when the time is up) ends the process the same way.
//!   Rust destructors and panic hooks do not run on these paths. Pending Rust
//!   output, and on Unix targets C output too, is flushed before the call, so
//!   nothing written earlier is lost. `--time-limit` and signals cannot be told apart from a normal run
//!   before they happen, so they are only documented. Every other option that
//!   ends the process is refused by `run` with
//!   [`ErrorKind::InvalidInput`](crate::ErrorKind::InvalidInput) before clingo
//!   starts, and nothing runs. They are:
//!   - `--fast-exit` and its abbreviations from `--fa`, which leave with
//!     `_exit` after every run;
//!   - `--pre`, bare or with `aspif` or `smodels`, which prints the program and
//!     leaves (`--pre=<other>` is an error clingo reports itself);
//!   - `--print-portfolio` and its abbreviations from `--pri`;
//!   - `--text`, or `--output` (`-o`) with a valid format, together with
//!     `--mode=clingo` or `--mode=clasp`, and `--text` together with
//!     `--output`: both leave with exit code 128. Alone, or with
//!     `--mode=gringo`, they are fine;
//!   - `--lemma-out=<file>` when the file cannot be opened for writing, and
//!     `--out-atomf=<format>` when clasp rejects the format (a newline, a `%`
//!     that is not `%%`, `%s`, `%d` or `%0`, more than one of these, or a
//!     variable format that does not start with `-`): clasp finds both while
//!     it sets up and leaves without returning.
//!
//!   The refusals err on the safe side and are deliberately a little wider
//!   than the exits: `--out-atomf` is checked even with `--outf=2` or `3`,
//!   which never build the text output that rejects it; `--pre` is refused
//!   with `--text` or any `--mode`; an option is checked even when an earlier
//!   error on the line would have made clingo return before reaching it; and
//!   an empty `--lemma-out` value takes the next argument as its file, as
//!   clasp does. The `--lemma-out` check looks at the file system without
//!   opening or creating anything: a directory is refused, any other existing
//!   path is accepted if `access(2)` says this process may write it, and a
//!   missing one needs an existing parent directory it may write. A change
//!   between the check and clasp's open can still slip through.
//!
//!   The check reads the command line the way clasp's parser does (exact name
//!   or unambiguous prefix, `=value` or the next argument, values in any
//!   case). It is slightly wider than clingo: an argument after `--` is
//!   checked like any other, and the refusal does not wait for a later
//!   command-line error that would have masked the exit.
//!
//!   Short options are read as clasp reads them, including groups in one
//!   argument: with a flag registered through
//!   [`Options::add_flag`](Options::add_flag) under `"name,m"`, `-mo text`
//!   means `-m -o text`. A flag letter goes on to the next letter of the
//!   argument; a letter that takes a value (`-o`, `-t`, `-n`, or an alias of
//!   an [`Options::add`](Options::add) option) takes the rest of the argument,
//!   or the next argument if nothing is left. Your own aliases are known only
//!   once `register_options` has run, so a group that hides one of the exits
//!   above is refused then, before clingo parses anything: `run` returns
//!   [`ErrorKind::InvalidInput`](crate::ErrorKind::InvalidInput), and clingo
//!   has already printed its own error line for the failed registration.
//! - **`--mode=clasp` once.** clasp opens its input once per process, so a
//!   second run with `--mode=clasp` (spelled `--mode=clasp` or `--mode clasp`,
//!   in any case; `--mode` has no other valid abbreviation) would read the
//!   first run's file and could answer for the wrong problem. The first such
//!   run is allowed; every later one in the process returns
//!   [`ErrorKind::InvalidInput`](crate::ErrorKind::InvalidInput) before clingo
//!   starts, even if the first one failed after clingo began to parse (a run
//!   refused for an exiting option hidden in a short-option group does not
//!   count). In this mode clasp reads a CNF or
//!   OPB file itself: `main` and `print_model` are never called, and `run`
//!   returns clasp's exit code (10 satisfiable, 20 unsatisfiable, 30 when all
//!   models were found).
//! - **Standard output.** clingo writes through C stdio. Rust's `stdout()`
//!   and `stderr()` and C stdio are flushed before and after the run, so
//!   their output stays in order. The C stdio flush (`fflush(NULL)`) exists
//!   on Unix targets only, which include Android and Emscripten; elsewhere
//!   only Rust's streams are flushed and the order of clingo's own output
//!   against yours is not promised. With a logger set, messages no longer
//!   reach standard error. Inside `main`, Rust's `println!` and clingo's own
//!   output go through different buffers: on a pipe or a file, clingo's text
//!   waits in C's buffer until it fills or the run ends, while Rust's stdout
//!   is line-buffered, so your lines can appear before clingo's earlier ones.
//!   The flush at the end of the run does not reorder them. Either accept
//!   that, keep your own output out of `main` (print models with
//!   [`print_model`](Application::print_model)), or write it to standard
//!   error.
//!
//! # Platforms
//!
//! On Unix, including Android and Emscripten, the signal dispositions are
//! saved and restored. On WebAssembly under Node.js no signal is ever
//! delivered, but `_exit` still ends the Node process. In a browser, any
//! `_exit` path (`--time-limit`, and signals if the host has them) ends the
//! module's runtime for good, so avoid `--time-limit` there. WebAssembly
//! without atomics has no threads, so clingo solves on the calling thread.

use std::ffi::OsStr;
use std::fmt;

use crate::Model;
use crate::control::ScopedControl;
use crate::error::{MessageCode, Result};
use crate::raw;

mod options;
mod printer;

pub use options::{Flag, OptionSpec, Options};
pub use printer::DefaultPrinter;

/// clasp's exit codes, as [`Application::run`] returns them.
///
/// They are bit flags: 30 is `SATISFIABLE | EXHAUSTED`, the result of a search
/// that found models and enumerated all of them.
pub mod exit_code {
    /// The result is not known, or clingo did nothing (`--help`, `--version`).
    pub const UNKNOWN: i32 = 0;
    /// The search was interrupted.
    pub const INTERRUPTED: i32 = 1;
    /// A model was found.
    pub const SATISFIABLE: i32 = 10;
    /// The search space is exhausted. On its own it means unsatisfiable.
    pub const EXHAUSTED: i32 = 20;
    /// The process ran out of memory.
    pub const MEMORY: i32 = 33;
    /// An error, such as an input file that cannot be read.
    pub const ERROR: i32 = 65;
    /// The run did not start.
    pub const NO_RUN: i32 = 128;
}

/// A program built on clingo's command line.
///
/// Set what you want to change and call [`run`](Application::run). What you
/// leave unset behaves as in the `clingo` executable. Callbacks may borrow
/// from the caller for `'a`, because `run` blocks until clingo and every
/// thread it started are done with them.
///
/// ```standalone_crate
/// use clingox::application::Application;
///
/// let path = std::env::temp_dir().join(format!("clingox-app-doc2-{}.lp", std::process::id()));
/// std::fs::write(&path, "a :- b. c :- d.")?;
/// let file = path.to_string_lossy().into_owned();
///
/// let mut messages = Vec::new();
/// let code = Application::new()
///     .program_name("checker")
///     .message_limit(1)
///     .logger(|code, text| messages.push((code, text.to_owned())))
///     .run([file.as_str(), "--outf=3"])?;
///
/// assert_eq!(code, 30, "the only model, and the search is exhausted");
/// assert_eq!(messages.len(), 1, "the limit stops the second message");
/// assert!(messages[0].1.contains("atom does not occur in any rule head"));
/// # std::fs::remove_file(&path)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use = "an application does nothing until it is run"]
pub struct Application<'a> {
    program_name: Option<String>,
    version: Option<String>,
    message_limit: Option<u32>,
    logger: Option<raw::AppLogger<'a>>,
    main: Option<raw::AppMain<'a>>,
    register_options: Option<raw::AppRegister<'a>>,
    validate_options: Option<raw::AppValidate<'a>>,
    print_model: Option<raw::AppPrinter<'a>>,
}

impl<'a> Application<'a> {
    /// An application that behaves exactly like the `clingo` executable: name
    /// `clingo`, clingo's version text, a limit of 20 messages, messages on
    /// standard error.
    pub fn new() -> Application<'a> {
        Application {
            program_name: None,
            version: None,
            message_limit: None,
            logger: None,
            main: None,
            register_options: None,
            validate_options: None,
            print_model: None,
        }
    }

    /// The name shown by `--help`, `--version` and in messages. The default is
    /// `clingo`.
    ///
    /// A name with a NUL byte is reported by [`run`](Application::run) as
    /// [`ErrorKind::Nul`](crate::ErrorKind::Nul).
    pub fn program_name(mut self, name: &str) -> Self {
        self.program_name = Some(name.to_owned());
        self
    }

    /// The version shown by `--version` and `--help`, whose first line reads
    /// `<name> version <version>`. The default is clingo's own.
    ///
    /// A version with a NUL byte is reported by [`run`](Application::run) as
    /// [`ErrorKind::Nul`](crate::ErrorKind::Nul).
    pub fn version(mut self, version: &str) -> Self {
        self.version = Some(version.to_owned());
        self
    }

    /// How many messages clingo reports before it stops. The default is 20. The
    /// limit applies with and without a logger.
    ///
    /// A limit of 0 does not silence errors: clingo passes an error on
    /// whatever the limit is, so it still reaches the logger (or standard
    /// error). The limit counts the other messages.
    pub fn message_limit(mut self, limit: u32) -> Self {
        self.message_limit = Some(limit);
        self
    }

    /// Receives clingo's messages in place of standard error.
    ///
    /// The text is the message with its position and without trailing
    /// whitespace, as for [`ControlBuilder::logger`](crate::ControlBuilder::logger).
    /// clingo may call the logger from any thread that emits a message, one
    /// call at a time. If it panics, the panic is caught, later messages are
    /// not passed on, clingo finishes normally, and [`run`](Application::run)
    /// resumes the panic once the process-wide state is restored.
    pub fn logger<F>(mut self, logger: F) -> Self
    where
        F: FnMut(MessageCode, &str) + Send + 'a,
    {
        self.logger = Some(Box::new(logger));
        self
    }

    /// Replaces clingo's default main: `f` receives the application's control,
    /// with the command-line options already applied, and the positional
    /// arguments (the files), instead of clingo loading, grounding and solving
    /// them itself.
    ///
    /// The control is a [`ScopedControl<'r>`](ScopedControl) for a brand `'r`
    /// that exists only during the call. It has every method a
    /// [`Control`](crate::Control) has, but clingo owns it and frees it after
    /// `f` returns, so neither the control nor anything borrowed from it
    /// (statistics, atoms, configuration, solve handles, models) can be kept
    /// past `f`: the compiler refuses to. That is why `f` must be written for
    /// every brand (`for<'r>`), and why moving the control out with
    /// [`std::mem::replace`] or [`std::mem::swap`] does not compile.
    ///
    /// ```standalone_crate
    /// use std::ops::ControlFlow;
    ///
    /// use clingox::application::Application;
    /// use clingox::{Part, ShowType};
    ///
    /// let path = std::env::temp_dir().join(format!("clingox-main-doc-{}.lp", std::process::id()));
    /// std::fs::write(&path, "1 {a; b} 1.")?;
    /// let file = path.to_string_lossy().into_owned();
    ///
    /// let mut models = Vec::new();
    /// let code = Application::new()
    ///     .main(|ctl, files| {
    ///         for file in files {
    ///             ctl.load(file)?;
    ///         }
    ///         ctl.ground(&[Part::base()])?;
    ///         let _result = ctl.for_each_model(&[], |model| {
    ///             let atoms = model.symbols(ShowType::SHOWN)?;
    ///             models.push(atoms.iter().map(ToString::to_string).collect::<Vec<_>>());
    ///             Ok(ControlFlow::Continue(()))
    ///         })?;
    ///         Ok(())
    ///     })
    ///     .run([file.as_str(), "0", "--outf=3"])?;
    /// // clingo derives the exit code from the searches `main` ran.
    /// assert_eq!(code, 30);
    /// models.sort();
    /// assert_eq!(models, [["a"], ["b"]]);
    /// # std::fs::remove_file(&path)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// Moving the control out does not compile, because its brand cannot be
    /// named outside the call:
    ///
    /// ```compile_fail,E0521
    /// use clingox::Control;
    /// use clingox::application::Application;
    ///
    /// let mut stash = None;
    /// let _ = Application::new()
    ///     .main(|ctl, _files| {
    ///         stash = Some(std::mem::replace(ctl, Control::new()?));
    ///         Ok(())
    ///     })
    ///     .run(["--outf=3"]);
    /// ```
    ///
    /// **Annotating the parameter.** Leave `ctl` unannotated, or write
    /// `&mut ScopedControl<'_>`. `&mut Control` means `&mut
    /// ScopedControl<'static>`, the kind you create yourself, and does not
    /// accept the callback's control.
    ///
    /// `f` runs once, on the thread that called [`run`](Application::run), and
    /// not at all for `--help`, `--version`, an unknown option, or a run with
    /// `--mode=clasp` (clasp then reads a CNF or OPB file itself and `run`
    /// returns its exit code). It is called
    /// even if a file is missing: loading is `f`'s job. Calling `main` again
    /// replaces `f`.
    ///
    /// If `f` returns an error, clingo prints `*** ERROR: (clingo): <message>`
    /// on standard error and would exit with 65; [`run`](Application::run)
    /// returns that error unchanged instead, so the code is visible only in
    /// that line. If `f` panics, the panic is resumed by `run`. Observers and
    /// propagators registered on the control are kept alive until clingo has
    /// finished, and a search left open when `f` returns is closed first.
    ///
    /// The control has no capture logger, so [`Error::messages`](crate::Error::messages)
    /// is empty on it; the text of a syntax error goes to the application's
    /// [`logger`](Application::logger) or to standard error.
    pub fn main<F>(mut self, f: F) -> Self
    where
        F: for<'r> FnOnce(&mut ScopedControl<'r>, &[&str]) -> Result<()> + 'a,
    {
        self.main = Some(Box::new(f));
        self
    }

    /// Adds your own options to clingo's command line.
    ///
    /// `f` receives the [`Options`] to register them with. It runs once per
    /// [`run`](Application::run), on the thread that called `run`, before the
    /// arguments are parsed, and also for `--help`, `--version` and a command
    /// line that turns out to be wrong, so the options appear in the help. The
    /// value of each option reaches the closure you give to
    /// [`Options::add`]; a [`Flag`] carries a yes or no.
    ///
    /// Calling `register_options` again replaces `f`. If `f` returns an error,
    /// clingo prints `*** ERROR: (clingo): <message>` and stops, and
    /// [`run`](Application::run) returns that error unchanged. If it panics,
    /// `run` resumes the panic.
    ///
    /// ```standalone_crate
    /// use std::cell::RefCell;
    ///
    /// use clingox::application::{Application, OptionSpec};
    ///
    /// let path = std::env::temp_dir().join(format!("clingox-opt-doc-{}.lp", std::process::id()));
    /// std::fs::write(&path, "a. {b}.")?;
    /// let file = path.to_string_lossy().into_owned();
    ///
    /// let level = RefCell::new(String::new());
    /// let code = Application::new()
    ///     .register_options(|options| {
    ///         let spec = OptionSpec::new("Example Options", "level", "How careful to be")
    ///             .argument("<n>");
    ///         options.add(spec, |value| {
    ///             *level.borrow_mut() = value.to_owned();
    ///             Ok(())
    ///         })
    ///     })
    ///     .validate_options(|| {
    ///         assert_eq!(*level.borrow(), "3", "validation sees the parsed value");
    ///         Ok(())
    ///     })
    ///     .run([file.as_str(), "--level=3", "0", "--outf=3"])?;
    /// assert_eq!(code, 30);
    /// # std::fs::remove_file(&path)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn register_options<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut Options<'a>) -> Result<()> + 'a,
    {
        self.register_options = Some(Box::new(f));
        self
    }

    /// Checks the options once they are all parsed.
    ///
    /// `f` runs once, on the thread that called [`run`](Application::run),
    /// after every option was parsed and before [`main`](Application::main)
    /// (also when an input file is missing). It does not run for `--help`,
    /// `--version`, an unknown option or a value a `parse` closure refused.
    /// A [`Flag`] already holds its final value when `f` runs.
    ///
    /// If `f` returns an error, clingo prints `*** ERROR: (clingo):
    /// <message>` followed by `Try '--help' for usage information`, and
    /// [`run`](Application::run) returns that error unchanged. clingo itself
    /// reports the exit code 0 in this case, the code of success (an upstream
    /// quirk), which is why `run` returns the error and not that code. If `f`
    /// panics, `run` resumes the panic. Calling `validate_options` again
    /// replaces `f`.
    ///
    /// Two closures of one application cannot both borrow the same variable
    /// mutably. To share what `parse` wrote with `validate_options` or
    /// `main`, keep it in a [`RefCell`](std::cell::RefCell) or a
    /// [`Cell`](std::cell::Cell) that outlives the application, as in the
    /// example of [`register_options`](Application::register_options).
    pub fn validate_options<F>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Result<()> + 'a,
    {
        self.validate_options = Some(Box::new(f));
        self
    }

    /// Replaces clingo's text output of each model: `f` receives the model and
    /// a [`DefaultPrinter`] that prints it the way clingo would. It needs
    /// [`main`](Application::main); [`run`](Application::run) returns
    /// [`ErrorKind::InvalidInput`](crate::ErrorKind::InvalidInput) without one
    /// (clingo crashes when a model is read in the printer of an application
    /// that has no `main`), before anything runs, also for `--help`.
    ///
    /// The bound is written with its higher-ranked lifetime, so it is visible
    /// that the printer cannot outlive the call:
    ///
    /// ```text
    /// F: for<'p> Fn(&Model, &mut DefaultPrinter<'p>) -> Result<()> + Send + Sync + 'a
    /// ```
    ///
    /// `f` runs once per model, on the thread that found it, one call at a
    /// time, possibly while `main` runs (when `main` solves asynchronously, or
    /// with several solver threads). It is `Fn`, so mutable state goes in
    /// atomics or a `Mutex`. The `&Model` is the same borrowed view
    /// [`Control::for_each_model`](crate::Control::for_each_model) lends and
    /// cannot be kept. `f` is not called at all with `-q`, `--outf=2`,
    /// `--outf=3`, `--mode=gringo` or `--text`, and with `--outf=1` and
    /// `--quiet=1` only for the last model.
    ///
    /// ```standalone_crate
    /// use std::sync::Mutex;
    ///
    /// use clingox::application::Application;
    /// use clingox::{Part, ShowType};
    ///
    /// let path = std::env::temp_dir().join(format!("clingox-print-doc-{}.lp", std::process::id()));
    /// std::fs::write(&path, "1 {a; b} 1.")?;
    /// let file = path.to_string_lossy().into_owned();
    ///
    /// let seen = Mutex::new(Vec::new());
    /// let code = Application::new()
    ///     .main(|ctl, files| {
    ///         for file in files {
    ///             ctl.load(file)?;
    ///         }
    ///         ctl.ground(&[Part::base()])?;
    ///         ctl.solve(&[])?;
    ///         Ok(())
    ///     })
    ///     .print_model(|model, printer| {
    ///         let atoms = model.symbols(ShowType::SHOWN)?;
    ///         seen.lock().unwrap().push(atoms.iter().map(ToString::to_string).collect::<Vec<_>>());
    ///         // Not calling `printer.print()` prints nothing for this model.
    ///         Ok(())
    ///     })
    ///     .run([file.as_str(), "0", "--verbose=0"])?;
    /// assert_eq!(code, 30);
    /// let mut seen = seen.into_inner().unwrap();
    /// seen.sort();
    /// assert_eq!(seen, [["a"], ["b"]]);
    /// # std::fs::remove_file(&path)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Output order
    ///
    /// clingo writes through C stdio and Rust's `println!` through its own
    /// buffer. This section holds on Unix targets, where clingox can flush C
    /// stdio; on other targets only Rust's standard output is flushed and the
    /// order against clingo's text is not promised. Before `f` is called C
    /// stdio is flushed; [`DefaultPrinter::print`]
    /// flushes Rust's standard output before and C stdio after clingo's
    /// printer; after `f` returns, on every path, both are flushed. So what
    /// `f` prints with `println!` or `print!` appears in program order with
    /// clingo's text, on a terminal, a pipe or a file. The closure must not
    /// wait for another thread that prints through C stdio (clasp holds the C
    /// `stdout` lock during the print), and a [`run`](Application::run) inside
    /// it is refused with `InvalidInput`, like any nested run.
    ///
    /// **Do not hold `std::io::stdout().lock()` in `main` while a solve runs
    /// asynchronously.** The flushes above take Rust's standard output lock on
    /// the solver thread, so the printer waits for a lock `main` holds while it
    /// waits for the search: a deadlock, even if `f` prints nothing. Take the
    /// lock only around a single write.
    ///
    /// # Failure
    ///
    /// If `f` returns an error, panics, or [`DefaultPrinter::print`] fails and
    /// `f` returns that error, the first failure is kept. clingo is told the
    /// call succeeded, `f` and the default printer are skipped for every later
    /// model (nothing more is printed), and the search is interrupted through
    /// the control `main` received, so it ends early. `run` then returns the
    /// error unchanged, or resumes the panic, whatever `main` did with its
    /// solve call. A search that cannot be interrupted, such as a blocking
    /// solve on a build without threads, runs to its end without printing.
    ///
    /// With `--mode=clasp` clasp writes the models itself and `f` is never
    /// called.
    ///
    /// Calling `print_model` again replaces `f`; the order relative to the
    /// other setters does not matter.
    pub fn print_model<F>(mut self, f: F) -> Self
    where
        F: for<'p> Fn(&Model, &mut DefaultPrinter<'p>) -> Result<()> + Send + Sync + 'a,
    {
        self.print_model = Some(Box::new(f));
        self
    }

    /// Runs clingo's command line and returns its exit code.
    ///
    /// `arguments` are the command line without the program name: files,
    /// numbers and options, as after `clingo` in a shell. Do not pass
    /// `std::env::args()` unchanged, whose first item is the executable, or it
    /// becomes an input file. With no file argument clingo reads the program
    /// from standard input. The call blocks until clingo and every thread it
    /// started are done.
    ///
    /// The exit code is clingo's, see [`exit_code`]. clingo's own command-line
    /// errors are exit codes, not errors: an unknown option is `Ok(1)` and a
    /// missing input file is `Ok(65)`, with clingo's text on standard error.
    ///
    /// # Process-wide effects
    ///
    /// See the [module documentation](self#process-wide-effects). In short: a
    /// signal or `--time-limit` expiring during the run **ends the process**
    /// and `run` never returns; the signal dispositions are restored on every
    /// path that returns.
    ///
    /// ```no_run
    /// use clingox::application::Application;
    ///
    /// // After about a second this ends the process with exit code 1.
    /// // Nothing after `run` executes and no destructor runs.
    /// Application::new().run(["hard.lp", "--time-limit=1"])?;
    /// println!("not reached when the limit expires");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// - whatever the [`main`](Application::main) callback returned as `Err`,
    ///   or the first error of the [`print_model`](Application::print_model)
    ///   closure (which comes first if both fail);
    /// - [`ErrorKind::Nul`](crate::ErrorKind::Nul) for a NUL byte in the name,
    ///   the version or an argument;
    /// - [`ErrorKind::InvalidInput`](crate::ErrorKind::InvalidInput) for an
    ///   argument that is not UTF-8, for `--fast-exit` and its abbreviations
    ///   (`--fa` and longer, with or without `=value`), for the other options
    ///   that end the process (`--pre`, `--print-portfolio`, `--text` or
    ///   `--output` with a conflicting `--mode`, an unwritable `--lemma-out`, an
    ///   invalid `--out-atomf`; the [module documentation](self#process-wide-effects)
    ///   lists them), and for a run while
    ///   another is in progress in the process, on any thread, for a second
    ///   run with `--mode=clasp` in the process, for a short-option group that
    ///   hides one of those options once your `register_options` has run, and for
    ///   [`print_model`](Application::print_model) without
    ///   [`main`](Application::main). Nothing runs,
    ///   is printed or is changed in these cases;
    /// - [`ErrorKind::Unknown`](crate::ErrorKind::Unknown) if the signal
    ///   dispositions cannot be read (nothing ran), or cannot be restored after
    ///   the run (the message names the signal and the exit code). POSIX allows
    ///   neither for these valid signals, so this is not expected;
    /// - [`ErrorKind::Version`](crate::ErrorKind::Version) if the linked clingo
    ///   is outside the supported range.
    ///
    /// # Panics
    ///
    /// Resumes a panic of the logger, of `main` or of the model printer, after clingo has finished and the
    /// process-wide state is restored.
    pub fn run<I>(self, arguments: I) -> Result<i32>
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        let owned: Vec<I::Item> = arguments.into_iter().collect();
        let refs: Vec<&OsStr> = owned.iter().map(AsRef::as_ref).collect();
        raw::run_application(
            raw::Settings {
                program_name: self.program_name,
                version: self.version,
                message_limit: self.message_limit,
                logger: self.logger,
                main: self.main,
                register_options: self.register_options,
                validate_options: self.validate_options,
                print_model: self.print_model,
            },
            &refs,
        )
    }
}

impl Default for Application<'_> {
    fn default() -> Self {
        Application::new()
    }
}

impl fmt::Debug for Application<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Application")
            .field("program_name", &self.program_name.as_deref())
            .field("version", &self.version.as_deref())
            .field("message_limit", &self.message_limit)
            .field("logger", &self.logger.is_some())
            .field("register_options", &self.register_options.is_some())
            .field("validate_options", &self.validate_options.is_some())
            .field("print_model", &self.print_model.is_some())
            .finish_non_exhaustive()
    }
}

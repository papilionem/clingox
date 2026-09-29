//! Custom scripting languages for `#script (name) ... #end.` blocks and
//! `@name(..)` terms.
//!
//! A [`Script`] is a language the program can call into while it is parsed and
//! grounded, the way clingo's own Python and Lua support work. Register it once
//! with [`register`], before the process creates a [`Control`] or runs an
//! [`Application`](crate::application::Application). From then on every
//! control in the process, and every run of clingo's command line, sees it.
//!
//! ```standalone_crate
//! use std::collections::HashMap;
//! use std::sync::Mutex;
//!
//! use clingox::ast::Span;
//! use clingox::prelude::*;
//! use clingox::script::{self, Script};
//!
//! /// `#script (vars) x = 3 #end.` defines a variable, `@get(x)` reads it.
//! #[derive(Default)]
//! struct Vars(Mutex<HashMap<String, i32>>);
//!
//! impl Script for Vars {
//!     fn execute(&self, _span: &Span, code: &str) -> clingox::Result<()> {
//!         for line in code.lines().filter(|l| !l.trim().is_empty()) {
//!             let (name, value) = line.split_once('=').ok_or_else(|| {
//!                 clingox::Error::new(clingox::ErrorKind::InvalidInput, "expected `name = value`")
//!             })?;
//!             let value = value.trim().parse().map_err(clingox::Error::callback)?;
//!             self.0.lock().unwrap().insert(name.trim().to_owned(), value);
//!         }
//!         Ok(())
//!     }
//!
//!     fn callable(&self, name: &str) -> clingox::Result<bool> {
//!         Ok(name == "get")
//!     }
//!
//!     fn call(&self, _span: &Span, _name: &str, arguments: &[Symbol]) -> clingox::Result<Vec<Symbol>> {
//!         let variable = arguments.first().and_then(Symbol::name).unwrap_or_default();
//!         let value = self.0.lock().unwrap().get(variable).copied();
//!         // No symbol means the term has no value and the rule instance is dropped.
//!         Ok(value.map(Symbol::number).into_iter().collect())
//!     }
//! }
//!
//! script::register("vars", "1.0", Vars::default())?;
//! assert_eq!(script::version("vars").as_deref(), Some("1.0"));
//!
//! let mut ctl = Control::new()?;
//! ctl.add_base("#script (vars) x = 3 #end. p(@get(x)). q(@get(y)).")?;
//! ctl.ground(&[Part::base()])?;
//! let (_, models) = ctl.solve_all()?;
//! clingox::testing::assert_models!(models, ["p(3)"]);
//! # Ok::<(), clingox::Error>(())
//! ```
//!
//! # What runs where
//!
//! - [`Script::execute`] runs on the thread that added the program
//!   ([`Control::add`], [`Control::load`], a program builder, or the parsing
//!   of a run), once per block in source order, before anything is grounded.
//! - [`Script::callable`] and [`Script::call`] run on the thread that grounds,
//!   for every `@` term. `callable` is asked immediately before each `call`
//!   and never memoised, so keep it cheap.
//! - [`Script::main`] runs on the thread that called
//!   [`Application::run`](crate::application::Application::run).
//! - Script code never runs on a solver thread, so a search with many threads
//!   does not call the script. Two threads that ground two controls at once do
//!   call the same script at the same time, which is why a script is
//!   `Send + Sync` and takes `&self`.
//!
//! # Rules worth knowing
//!
//! - **One upstream race remains.** clingo sets a "this script has run a
//!   block" flag without a lock, and reads it the same way. Two controls on
//!   two threads that both execute `#script` blocks of one language race on
//!   that flag (U41), which `ThreadSanitizer` reports. Run the blocks of such a
//!   language from one thread, or accept the race.
//! - **Registration comes first.** [`register`] fails with
//!   [`ErrorKind::InvalidInput`] once the process has created a control (even
//!   one that failed to build or was dropped) or started an application:
//!   clingo's script registry is not synchronised with grounding, so adding
//!   to it while another thread grounds would race. An argument check that
//!   rejects a run before clingo is reached does not count.
//! - **One language per name.** A name registered twice is refused, because
//!   clingo would run every block of both.
//! - **Scripts are never dropped.** A script lives until the process exits and
//!   its `Drop` never runs. clingo would run a `free` callback during static
//!   destruction, when Rust may already be gone, and that crashes hosts, so
//!   none is installed. A script that is refused is dropped at once.
//! - **Only names the lexer reads are reachable.** Any name registers, but a
//!   program can use `#script (name)` only for a lowercase identifier
//!   (`foo`, `foo_bar`, `python`, `lua`); `Foo`, `1x` and non-ASCII names can
//!   be looked up with [`version`] and nothing else.
//! - **Every language that has run a block is asked** about every `@` term for
//!   the rest of the process, in registration order, and the first whose
//!   `callable` says yes gets the `call`. Answer `callable` only for names of
//!   your own language.
//! - **A ground callback wins.** [`Control::ground_with`] asks its callback
//!   first for every `@` term and never falls through to a script, whatever
//!   the callback does, even for a name it does not know. Use
//!   [`Control::ground`] to reach a script, or call your script from the
//!   callback yourself.
//! - **`python` and `lua`.** The names clingo's own `--version` text
//!   mentions: registering them makes it print `with Python <version>` and
//!   `with Lua <version>`. The languages themselves are not provided.
//!
//! # Errors and panics
//!
//! An error a script returns comes back from the call that ran it, with its
//! [`kind`](crate::Error::kind) and [`source`](std::error::Error::source)
//! unchanged and the operation added to the message.
//!
//! - From `execute`, through [`Control::add`] or [`Control::load`]: the parse
//!   stops at the failing block, so later blocks in the same program do not
//!   run, and the error is not turned into [`ErrorKind::Parse`] the way
//!   clingo's own syntax errors are. It poisons the control **whatever its
//!   kind**: clingo keeps the rest of the failed program queued in its parser
//!   and the next `add` or `load` would silently resume it, so a control that
//!   stayed usable would ground text nobody was told had been accepted.
//!   Recovery means building a new [`Control`].
//! - From `callable` or `call`, through [`Control::ground`]: grounding stops
//!   and the control is poisoned whatever the kind, because clingo keeps what
//!   it already ground and would answer from it silently.
//! - A panic in any of them is caught, the control is poisoned, and the panic
//!   resumes on the calling thread when the call returns. It never unwinds
//!   through clingo.
//! - After the first failure of a call, later script callbacks of that call
//!   return without running your code, so at most one error or panic is kept.
//! - Through [`Application::run`](crate::application::Application::run) the
//!   same errors come back as `Err` (clingo itself exits with 65 and prints
//!   `*** ERROR: (clingo): <message>`), never as an exit code.
//!
//! [`Control`]: crate::Control
//! [`Control::add`]: crate::Control::add
//! [`Control::load`]: crate::Control::load
//! [`Control::ground`]: crate::Control::ground
//! [`Control::ground_with`]: crate::Control::ground_with
//! [`ErrorKind::InvalidInput`]: crate::ErrorKind::InvalidInput
//! [`ErrorKind::Parse`]: crate::ErrorKind::Parse

use crate::ast::Span;
use crate::control::ScopedControl;
use crate::error::{Error, ErrorKind, Result};
use crate::raw;
use crate::symbol::Symbol;

/// A scripting language for `#script (name) ... #end.` blocks and
/// `@name(..)` terms.
///
/// One value serves every control in the process, from whichever thread adds
/// or grounds, possibly at the same time, so its methods take `&self` and the
/// value is `Send + Sync + 'static` (it moves to a process-global registry and
/// is never dropped). Only [`execute`](Script::execute) is required.
///
/// See the [module documentation](self) for the rules that apply to all
/// scripts, and [`register`] for how to install one.
pub trait Script: Send + Sync + 'static {
    /// Runs the code of one block while the program is parsed ([`Control::add`],
    /// [`Control::load`], a program builder, or a run's parse).
    ///
    /// `span` is the block from the `#` of `#script` to one past the `.` of
    /// `#end.`; its file is `<block>` for [`Control::add`], the path as given
    /// for [`Control::load`] and `<string>` for a statement added through a
    /// program builder. `code` is the text between the delimiters; blanks next
    /// to the delimiters on the same line are dropped, newlines and inner
    /// indentation are kept. Blocks run in source order, before anything of
    /// the program is grounded.
    ///
    /// An error stops the parse: later blocks in the same program do not run,
    /// and the control is poisoned whatever the error's kind (clingo keeps the
    /// rest of the program queued).
    ///
    /// [`Control::add`]: crate::Control::add
    /// [`Control::load`]: crate::Control::load
    ///
    /// # Errors
    ///
    /// Whatever the implementation returns; see the
    /// [module documentation](self#errors-and-panics).
    fn execute(&self, span: &Span, code: &str) -> Result<()>;

    /// Whether `name` is a function of this script.
    ///
    /// Asked immediately before every [`call`](Script::call), for every `@`
    /// term, and never memoised; it must be cheap. Only languages that have
    /// run a block are asked, in registration order, and the first to answer
    /// `true` gets the call. The default answers `false`.
    ///
    /// Also asked for `"main"`, twice per run, when an
    /// [`Application`](crate::application::Application) has no `main`
    /// callback: see [`Script::main`].
    ///
    /// # Errors
    ///
    /// An error stops grounding and poisons the control.
    fn callable(&self, name: &str) -> Result<bool> {
        let _ = name;
        Ok(false)
    }

    /// Calls `name(arguments)` for a term whose [`callable`](Script::callable)
    /// answered `true`. The symbols returned form a pool: none drops the rule
    /// instance (the term has no value), three make three instances.
    ///
    /// `span` covers the term from the `@` to one past its `)`. The default
    /// returns an [`ErrorKind::InvalidInput`] error saying the script has no
    /// such function.
    ///
    /// # Errors
    ///
    /// An error stops grounding and poisons the control.
    fn call(&self, span: &Span, name: &str, arguments: &[Symbol]) -> Result<Vec<Symbol>> {
        let _ = (span, arguments);
        Err(Error::new(
            ErrorKind::InvalidInput,
            format!("the script has no function `{name}`"),
        ))
    }

    /// Runs instead of clingo's default main, which reads the files and then
    /// grounds and solves, when the application has no
    /// [`main`](crate::application::Application::main) callback and
    /// [`callable("main")`](Script::callable) is true. The files are already
    /// parsed. It receives the same control an
    /// [`Application::main`](crate::application::Application::main) callback
    /// does, with the same limits: the control cannot be kept, and it is
    /// finished when `main` returns.
    ///
    /// `Ok(())` ends the run with exit code 0 and clingo's `UNKNOWN` summary
    /// unless the method solved something. An `Err` makes
    /// [`Application::run`](crate::application::Application::run) return that
    /// error with its kind unchanged (clingo prints `*** ERROR: (clingo):
    /// <message>` and exits with 65); a panic resumes from `run` after the
    /// run's cleanup. The default does nothing.
    ///
    /// **Takeover.** clingo asks only languages that have run a block, and
    /// never resets that. A script that answers `callable("main")` with `true`
    /// therefore takes over **every** later default-main run in the process,
    /// also over files that have no `#script` block, and also runs that
    /// belong to other code. A script that must not do this answers `false`
    /// unless it means to run.
    ///
    /// # Errors
    ///
    /// Whatever the implementation returns.
    ///
    /// # Examples
    ///
    /// ```standalone_crate
    /// use std::sync::atomic::{AtomicBool, Ordering};
    ///
    /// use clingox::application::{Application, exit_code};
    /// use clingox::ast::Span;
    /// use clingox::prelude::*;
    /// use clingox::script::{self, Script};
    ///
    /// struct Driver(AtomicBool);
    ///
    /// impl Script for Driver {
    ///     fn execute(&self, _span: &Span, _code: &str) -> clingox::Result<()> {
    ///         Ok(())
    ///     }
    ///
    ///     fn callable(&self, name: &str) -> clingox::Result<bool> {
    ///         Ok(name == "main" && self.0.load(Ordering::SeqCst))
    ///     }
    ///
    ///     fn main(&self, control: &mut ScopedControl<'_>) -> clingox::Result<()> {
    ///         control.add_base("b.")?;
    ///         control.ground(&[Part::base()])?;
    ///         let (_, models) = control.solve_all()?;
    ///         assert_eq!(models.len(), 1);
    ///         Ok(())
    ///     }
    /// }
    ///
    /// script::register("driver", "1", Driver(AtomicBool::new(true)))?;
    /// let path = std::env::temp_dir().join(format!("clingox-script-main-{}.lp", std::process::id()));
    /// std::fs::write(&path, "#script (driver) #end. a.")?;
    /// let file = path.to_string_lossy().into_owned();
    /// // `--outf=3` silences clingo's own output.
    /// let code = Application::new().run([file.as_str(), "--outf=3"])?;
    /// // The script's main ran instead of clingo's and solved the control it
    /// // was given: one model, all enumerated.
    /// assert_eq!(code, exit_code::SATISFIABLE | exit_code::EXHAUSTED);
    /// # std::fs::remove_file(&path)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    fn main(&self, control: &mut ScopedControl<'_>) -> Result<()> {
        let _ = control;
        Ok(())
    }
}

/// Registers `script` as the language `name`, for the rest of the process.
///
/// `version` is what [`version`] returns, and what clingo's `--version` text
/// prints for the names `python` and `lua`. The script is leaked on purpose
/// and never dropped once accepted; see the [module documentation](self).
///
/// # Errors
///
/// In this order:
///
/// - [`ErrorKind::Nul`] if `name` or `version` contains a NUL byte;
/// - [`ErrorKind::InvalidInput`] if the process has already created a
///   [`Control`](crate::Control) (also one that failed to build, or that was
///   dropped) or started an application. An argument check that rejects a run
///   before clingo is reached does not count;
/// - [`ErrorKind::InvalidInput`] if `name` is registered already;
/// - clingo's error if it refuses the registration, for example
///   [`ErrorKind::BadAlloc`], and [`ErrorKind::Version`] if the linked clingo
///   is not one clingox accepts.
///
/// A script that is refused is dropped before this function returns.
///
/// # Examples
///
/// ```standalone_crate
/// use clingox::ast::Span;
/// use clingox::script::{self, Script};
/// use clingox::{Control, ErrorKind};
///
/// struct Silent;
///
/// impl Script for Silent {
///     fn execute(&self, _span: &Span, _code: &str) -> clingox::Result<()> {
///         Ok(())
///     }
/// }
///
/// script::register("silent", "0.1", Silent)?;
/// // The same name again, and any name once a control exists, are refused.
/// let again = script::register("silent", "0.2", Silent).unwrap_err();
/// assert_eq!(again.kind(), ErrorKind::InvalidInput);
/// let _ctl = Control::new()?;
/// let late = script::register("late", "1", Silent).unwrap_err();
/// assert_eq!(late.kind(), ErrorKind::InvalidInput);
/// assert_eq!(script::version("silent").as_deref(), Some("0.1"));
/// assert_eq!(script::version("late"), None);
/// # Ok::<(), clingox::Error>(())
/// ```
pub fn register(name: &str, version: &str, script: impl Script) -> Result<()> {
    raw::register_script(name, version, Box::new(script))
}

/// The version text a language was registered with, or `None` if `name` is
/// not registered (also for a name with a NUL byte).
///
/// Case sensitive, and an owned copy.
///
/// # Examples
///
/// ```
/// use clingox::script;
///
/// assert_eq!(script::version("nothing-registered"), None);
/// ```
#[must_use]
pub fn version(name: &str) -> Option<String> {
    raw::script_version(name)
}

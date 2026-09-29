//! Command-line options of an [`Application`](super::Application): the
//! registry lent to [`register_options`](super::Application::register_options),
//! the description of one option, and the boolean a flag sets.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{Error, ErrorKind, Result};
use crate::raw::{self, OptionsHandle};

/// The command line being extended.
///
/// It is only ever lent, as `&mut`, to the closure of
/// [`Application::register_options`](super::Application::register_options),
/// so it cannot be kept past registration. It is neither `Send` nor `Sync`.
///
/// # The option key
///
/// The second argument of [`OptionSpec::new`] and of [`Options::add_flag`]
/// names the option in clingo's notation:
///
/// - `"name"` is the long option `--name`;
/// - `"name,p"` adds the one-character alias `-p` (given as `-p v`, `-pv`; a
///   value after `-p=` starts with the `=`);
/// - `"name,@l"` sets the help level `l` from 0 to 5: level 0 is shown by
///   `--help`, level 1 by `--help=2`, level 2 by `--help=3`, and levels 3 to
///   5 are never listed;
/// - `"name,p,@l"` combines both, in that order.
///
/// Anything else that clingo rejects (`"a,b,c"`, `"x,@9"`, an empty name) makes
/// the registration fail with [`ErrorKind::Logic`] and clingo's message, and
/// registers nothing. A name that begins with `-` or contains `=` could never be
/// typed on a command line and is refused with [`ErrorKind::InvalidInput`], as
/// is a name or alias this run registered before. clingo's own options are
/// not checked: an option named like one of them (`models`, or the alias `t`,
/// which exists only on builds of clingo with threads and so is free on
/// WebAssembly) registers, and the run ends with clingo's `duplicate option`
/// message and exit code 1 before anything else happens.
///
/// Options and flags share one namespace of long names, and aliases are a
/// second one.
///
/// Some keys clingo accepts can never match a command line, and `add` does not
/// refuse them: a space in the name, and the aliases `-` and `=`. Such an option
/// is registered and never given. Also, the `--no-<name>` form of a flag
/// shadows an option that is named `no-<name>`, which then cannot be reached.
pub struct Options<'o> {
    handle: OptionsHandle<'o>,
    names: HashSet<String>,
    aliases: HashSet<String>,
    flags: Vec<Flag>,
}

impl<'o> Options<'o> {
    pub(crate) fn from_handle(handle: OptionsHandle<'o>) -> Self {
        Options {
            handle,
            names: HashSet::new(),
            aliases: HashSet::new(),
            flags: Vec::new(),
        }
    }

    /// Registers an option that takes a value.
    ///
    /// `parse` receives the text after `=` (or the next argument), once for
    /// each time the option is given, in command-line order. The text is never
    /// empty: `--name` without a value and `--name=` are syntax errors that
    /// clingo reports itself, and `parse` is not called. A value may begin
    /// with `-`. `parse` may return an error to refuse the value: parsing
    /// stops, and [`run`](super::Application::run) returns that error
    /// unchanged. clingo prints its own line for it, `'<value>' invalid value
    /// for: '<name>'`, and drops the message the closure produced.
    ///
    /// `parse` runs on the thread that called `run`, only while clingo parses
    /// the command line, so it can capture what the caller of `run` owns. To
    /// read what it wrote from [`validate_options`](super::Application::validate_options)
    /// or [`main`](super::Application::main), write to a
    /// [`RefCell`](std::cell::RefCell) or [`Cell`](std::cell::Cell) that lives
    /// outside the application, since two closures cannot borrow one
    /// variable mutably.
    ///
    /// An option given twice without [`OptionSpec::multi`] ends the run with
    /// exit code 1 (`multiple occurrences`) after `parse` saw the first value.
    ///
    /// ```
    /// use std::cell::RefCell;
    ///
    /// use clingox::application::{Application, OptionSpec};
    ///
    /// let path = std::env::temp_dir().join(format!("clingox-add-doc-{}.lp", std::process::id()));
    /// std::fs::write(&path, "a.")?;
    /// let file = path.to_string_lossy().into_owned();
    ///
    /// let ports = RefCell::new(Vec::new());
    /// let code = Application::new()
    ///     .register_options(|options| {
    ///         let spec = OptionSpec::new("Server Options", "port,p", "A port to listen on")
    ///             .multi()
    ///             .argument("<n>");
    ///         options.add(spec, |value| {
    ///             let port: u16 = value.parse().map_err(|_| {
    ///                 clingox::Error::new(clingox::ErrorKind::InvalidInput, "not a port")
    ///             })?;
    ///             ports.borrow_mut().push(port);
    ///             Ok(())
    ///         })
    ///     })
    ///     .run([file.as_str(), "--port=80", "-p", "8080", "--outf=3"])?;
    /// assert_eq!(*ports.borrow(), [80, 8080]);
    /// assert_eq!(code, 30);
    /// # std::fs::remove_file(&path)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Nul`] for a NUL byte in any string;
    /// - [`ErrorKind::InvalidInput`] for a name or alias registered before in
    ///   this run, and for a name that begins with `-` or contains `=`;
    /// - [`ErrorKind::Logic`], with clingo's message, for a key clingo does not
    ///   accept.
    pub fn add<F>(&mut self, spec: OptionSpec<'_>, parse: F) -> Result<()>
    where
        F: FnMut(&str) -> Result<()> + 'o,
    {
        let strings = [
            spec.group,
            spec.option,
            spec.description,
            spec.argument.unwrap_or(""),
        ];
        check_nul(&strings)?;
        let key = self.check_key(spec.option, None)?;
        self.handle.add(
            spec.group,
            spec.option,
            spec.description,
            spec.multi,
            spec.argument,
            Box::new(parse),
        )?;
        self.record(key);
        Ok(())
    }

    /// Registers a boolean option: `--name` sets `flag` to true and
    /// `--no-name` to false.
    ///
    /// The value clingo starts from is what `flag` holds now, so it stays
    /// when the option is not given. `--name=` is accepted and means true;
    /// `--name=x` is an error clingo reports (exit code 1), and so is giving
    /// the flag twice.
    ///
    /// The result reaches `flag` when parsing is done, right before
    /// [`validate_options`](super::Application::validate_options), so
    /// `validate_options`, [`main`](super::Application::main) and the code
    /// after [`run`](super::Application::run) see it, and an option's `parse`
    /// closure does not. If the run ends earlier, for `--help` or a command
    /// line clingo refuses, `flag` keeps its value. A flag used for a second
    /// run starts from what the first run left.
    ///
    /// The description is shown literally, `%` included.
    ///
    /// ```
    /// use clingox::application::{Application, Flag};
    ///
    /// let path = std::env::temp_dir().join(format!("clingox-flag-doc-{}.lp", std::process::id()));
    /// std::fs::write(&path, "a.")?;
    /// let file = path.to_string_lossy().into_owned();
    ///
    /// let chatty = Flag::new(false);
    /// let seen = chatty.clone();
    /// let code = Application::new()
    ///     .register_options(|options| {
    ///         options.add_flag("Example Options", "chatty", "Talk more", &chatty)
    ///     })
    ///     .validate_options(|| {
    ///         assert!(seen.get());
    ///         Ok(())
    ///     })
    ///     .run([file.as_str(), "--chatty", "--outf=3"])?;
    /// assert_eq!(code, 30);
    /// assert!(chatty.get());
    /// # std::fs::remove_file(&path)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// As for [`add`](Options::add); in addition, registering the same `Flag`
    /// twice in one run is [`ErrorKind::InvalidInput`].
    pub fn add_flag(
        &mut self,
        group: &str,
        option: &str,
        description: &str,
        flag: &Flag,
    ) -> Result<()> {
        check_nul(&[group, option, description])?;
        let key = self.check_key(option, Some(flag))?;
        self.handle.add_flag(group, option, description, flag)?;
        self.record(key);
        self.flags.push(flag.clone());
        Ok(())
    }

    /// What this run knows about the key, checked before clingo sees it.
    fn check_key(&self, option: &str, flag: Option<&Flag>) -> Result<Key> {
        let mut parts = option.split(',');
        let name = parts.next().unwrap_or_default();
        let alias = parts.next().filter(|part| !part.starts_with('@'));
        if name.starts_with('-') || name.contains('=') {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("the option name {name:?} could never be given on a command line"),
            ));
        }
        if self.names.contains(name) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("the option {name:?} is already registered in this run"),
            ));
        }
        if let Some(alias) = alias
            && self.aliases.contains(alias)
        {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("the alias {alias:?} is already registered in this run"),
            ));
        }
        if flag.is_some_and(|flag| self.flags.iter().any(|known| known.same(flag))) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "this Flag is already registered in this run",
            ));
        }
        Ok(Key {
            name: name.to_owned(),
            alias: alias.map(str::to_owned),
        })
    }

    fn record(&mut self, key: Key) {
        self.names.insert(key.name);
        if let Some(alias) = key.alias {
            self.aliases.insert(alias);
        }
    }
}

impl fmt::Debug for Options<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("names", &self.names.len())
            .field("aliases", &self.aliases.len())
            .finish_non_exhaustive()
    }
}

/// The long name and alias one registration takes.
struct Key {
    name: String,
    alias: Option<String>,
}

fn check_nul(strings: &[&str]) -> Result<()> {
    strings
        .iter()
        .try_for_each(|text| raw::c_str(text).map(drop))
}

/// The description of one option, for [`Options::add`].
///
/// ```
/// use clingox::application::OptionSpec;
///
/// // `--seed=<n>` in the help, hidden until `--help=2`, also usable as `-s`.
/// let spec = OptionSpec::new("Example Options", "seed,s,@1", "The seed for the run")
///     .argument("<n>");
/// # let _ = spec;
/// ```
#[must_use]
#[derive(Clone, Copy, Debug)]
pub struct OptionSpec<'s> {
    group: &'s str,
    option: &'s str,
    description: &'s str,
    multi: bool,
    argument: Option<&'s str>,
}

impl<'s> OptionSpec<'s> {
    /// An option in the help section `group`, named by `option` (see the
    /// notation on [`Options`]), with `description` in the help text.
    ///
    /// The description is shown literally, `%` included. `group` becomes a
    /// section of `--help` (clingo appends `Options` to a name that does not
    /// end with it); an empty group lists the option under the previous one.
    pub fn new(group: &'s str, option: &'s str, description: &'s str) -> Self {
        OptionSpec {
            group,
            option,
            description,
            multi: false,
            argument: None,
        }
    }

    /// The option may be given more than once; the closure runs for each.
    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    /// The name of the value in the help text, as in `--level=<n>`. The default
    /// is `<arg>`.
    pub fn argument(mut self, name: &'s str) -> Self {
        self.argument = Some(name);
        self
    }
}

/// A boolean that a flag option of the command line sets.
///
/// Register it with [`Options::add_flag`]; read it with [`get`](Flag::get)
/// from [`validate_options`](super::Application::validate_options),
/// [`main`](super::Application::main) or after [`run`](super::Application::run).
/// Clones share one value, and the type is `Send` and `Sync`.
#[derive(Clone, Debug, Default)]
pub struct Flag(Arc<AtomicBool>);

impl Flag {
    /// A flag that reads `initial` until a run sets it.
    pub fn new(initial: bool) -> Self {
        Flag(Arc::new(AtomicBool::new(initial)))
    }

    /// The current value.
    pub fn get(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn store(&self, value: bool) {
        self.0.store(value, Ordering::Release);
    }

    /// Whether `other` is a clone of this flag.
    fn same(&self, other: &Flag) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

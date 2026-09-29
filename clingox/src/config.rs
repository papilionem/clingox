//! Reading and changing clingo's configuration by path.

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::fmt;
use std::marker::PhantomData;

use crate::control::ControlCore;
use crate::error::{Error, ErrorKind, Result};
use crate::raw;

/// A view of a control's configuration, read and written by path.
///
/// A path is clingo's own key syntax: names separated by dots, where a number
/// selects an array element (`solve.models`, `solver.0.seed`). clingo also
/// resolves a name on an array through its first element (`solver.seed`). The
/// empty path is the root. Values are text, as clingo prints and parses them.
///
/// The view holds the control mutably (DESIGN S5), so the control cannot be
/// used while it is alive. [`Control::configuration`] makes one.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("{a;b}.")?;
/// ctl.ground(&[Part::base()])?;
/// let mut conf = ctl.configuration();
/// assert_eq!(conf.get("solve.models")?.as_deref(), Some("-1"));
/// conf.set("solve.models", "0")?;
/// let (_, models) = ctl.solve_all()?;
/// assert_eq!(models.len(), 4);
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct Configuration<'c> {
    control: &'c mut ControlCore,
    /// Keeps the view on the thread that made it (DESIGN S12). Only the
    /// control and its solve handles move between threads; a view that
    /// became `Send` through `&mut Control` alone would widen that promise
    /// without a reason.
    _not_send: PhantomData<*const ()>,
}

/// What a configuration entry is, as [`Configuration::kind`] reports it.
///
/// Mirrors clingo's `clingo_configuration_type`, a set of three bits.
/// `#[non_exhaustive]`, as [`StatKind`](crate::StatKind) and the other enums
/// that mirror a C one are.
///
/// # Examples
///
/// ```
/// use clingox::ConfigKind;
///
/// assert_ne!(ConfigKind::Map, ConfigKind::ArrayMap);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfigKind {
    /// A value, read with [`Configuration::get`] and written with
    /// [`Configuration::set`].
    Value,
    /// An array, reached by index: [`Configuration::len`] and
    /// [`Configuration::element`].
    Array,
    /// A map, reached by name: [`Configuration::keys`] and
    /// [`Configuration::has_key`].
    Map,
    /// Both an array and a map. clingo's `solver` entry is one: an array with
    /// one map of options per thread, that also answers to the names of the
    /// first element (`solver.seed`).
    ArrayMap,
}

impl ScopedControl<'_> {
    /// A view of the control's configuration.
    ///
    /// A search left open by a forgotten handle is closed first (DESIGN S4).
    /// If that fails, the control is poisoned and every method of the view
    /// returns [`ErrorKind::Poisoned`](crate::ErrorKind::Poisoned).
    ///
    /// It does not return `Result` itself: every operation of the view does,
    /// and the error of closing a leftover search surfaces there, so a
    /// fallible constructor would only add a second place to handle it.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// let keys = ctl.configuration().keys("")?;
    /// assert!(keys.iter().any(|k| k == "solve"));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn configuration(&mut self) -> Configuration<'_> {
        // A failure poisons the control, and the view's methods report that.
        drop(self.core.finish_search());
        Configuration {
            control: &mut self.core,
            _not_send: PhantomData,
        }
    }
}

impl Configuration<'_> {
    /// The value at `path` as clingo prints it, or `None` for an entry
    /// without a value: a map or array such as `solve`, or an option clingo
    /// leaves unassigned, such as `tester.solver.heuristic`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Runtime`](crate::ErrorKind::Runtime) for an unknown
    ///   path;
    /// - [`ErrorKind::Nul`](crate::ErrorKind::Nul) if the path contains a NUL
    ///   byte;
    /// - [`ErrorKind::Poisoned`](crate::ErrorKind::Poisoned) if an earlier
    ///   error poisoned the control.
    ///
    /// None of them poisons the control. The message names the path.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::with_args(["--opt-mode=optN"])?;
    /// let conf = ctl.configuration();
    /// assert_eq!(conf.get("solve.opt_mode")?.as_deref(), Some("optN"));
    /// assert_eq!(conf.get("solve")?, None);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn get(&self, path: &str) -> Result<Option<String>> {
        self.control.observed(
            || format!("reading configuration `{path}`"),
            |handle| raw::with_c_str(path, |path| handle.config_get(path)),
        )
    }

    /// Sets the value at `path`, as text. It takes effect from the next solve
    /// call.
    ///
    /// **After a rejected value.** clingo may change an option even when it
    /// rejects the new value (on wasm32 a rejected `solve.models` is left at
    /// 0). So when the option had a value before, clingox sets that value
    /// again, and the option reads as it did. This cannot work for an option
    /// that had no value: clingo has no way to unassign one. The options of
    /// the `tester` configuration start unassigned, and the first `set` of
    /// any of them, rejected or not, makes clingo create that configuration
    /// with every option assigned its default. For example,
    /// `tester.solver.opt_strategy` reads `None` at first, and `Some("bb,lin")`
    /// after a rejected `"abc"`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Runtime`](crate::ErrorKind::Runtime) for an unknown
    ///   path, a map or array, or a value clingo rejects;
    /// - [`ErrorKind::Nul`](crate::ErrorKind::Nul) if the path or the value
    ///   contains a NUL byte;
    /// - [`ErrorKind::Poisoned`](crate::ErrorKind::Poisoned) if an earlier
    ///   error poisoned the control.
    ///
    /// None of them poisons the control. The message names the path.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ErrorKind;
    ///
    /// let mut ctl = clingox::Control::new()?;
    /// let mut conf = ctl.configuration();
    /// conf.set("solve.models", "0")?;
    /// let err = conf.set("solve.models", "abc").unwrap_err();
    /// assert_eq!(err.kind(), ErrorKind::Runtime);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn set(&mut self, path: &str, value: &str) -> Result<()> {
        self.control.guarded(
            || format!("setting configuration `{path}` to {value:?}"),
            |handle| handle.config_set(&raw::c_str(path)?, &raw::c_str(value)?),
        )
    }

    /// The names under the map at `path`, in clingo's order. A value has no
    /// names.
    ///
    /// # Errors
    ///
    /// As [`Configuration::get`].
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// let keys = ctl.configuration().keys("solve")?;
    /// assert!(keys.iter().any(|k| k == "models"));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn keys(&self, path: &str) -> Result<Vec<String>> {
        self.control.observed(
            || format!("listing configuration `{path}`"),
            |handle| raw::with_c_str(path, |path| handle.config_keys(path)),
        )
    }
}

impl Configuration<'_> {
    /// What kind of entry `path` names, so that a caller can walk the tree
    /// without guessing: [`keys`](Configuration::keys) for a map,
    /// [`len`](Configuration::len) and [`element`](Configuration::element) for
    /// an array, [`get`](Configuration::get) for a value.
    ///
    /// # Errors
    ///
    /// As [`Configuration::get`] for an unknown path, and
    /// [`ErrorKind::Unknown`] if clingo reports a combination of types this
    /// version does not know.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ConfigKind;
    ///
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// assert_eq!(conf.kind("solve")?, ConfigKind::Map);
    /// assert_eq!(conf.kind("solve.models")?, ConfigKind::Value);
    /// assert_eq!(conf.kind("solver")?, ConfigKind::ArrayMap);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn kind(&self, path: &str) -> Result<ConfigKind> {
        self.control.observed(
            || format!("classifying configuration `{path}`"),
            |handle| {
                let bits = raw::with_c_str(path, |path| handle.config_kind(path))?;
                match bits {
                    1 => Ok(ConfigKind::Value),
                    2 => Ok(ConfigKind::Array),
                    4 => Ok(ConfigKind::Map),
                    6 => Ok(ConfigKind::ArrayMap),
                    other => Err(Error::new(
                        ErrorKind::Unknown,
                        format!("clingo reports the unknown configuration type {other}"),
                    )),
                }
            },
        )
    }

    /// The help text of the entry at `path`, exactly as clingo has it: it is
    /// not trimmed, and a `%A` stands for the option's argument. The empty
    /// path is the root and reads `Options`.
    ///
    /// # Errors
    ///
    /// As [`Configuration::get`].
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// assert_eq!(conf.description("solve")?, "Solve Options");
    /// assert_eq!(
    ///     conf.description("solver.seed")?,
    ///     "Set random number generator's seed to %A",
    /// );
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn description(&self, path: &str) -> Result<String> {
        self.control.observed(
            || format!("describing configuration `{path}`"),
            |handle| raw::with_c_str(path, |path| handle.config_description(path)),
        )
    }

    /// The number of elements of the array entry at `path`, such as `solver`,
    /// which has one element per thread. An entry that is not an array is
    /// [`ErrorKind::InvalidInput`](crate::ErrorKind::InvalidInput).
    ///
    /// The size grows when an element past the end is set:
    /// `set("solver.5.seed", "9")` on two threads makes it 6.
    ///
    /// # Errors
    ///
    /// As [`Configuration::get`], and `InvalidInput` for an entry that is not
    /// an array.
    ///
    /// # Examples
    ///
    /// ```
    /// // One solver by default; `-t 3` would make it 3 where threads exist.
    /// let mut ctl = clingox::Control::new()?;
    /// assert_eq!(ctl.configuration().len("solver")?, 1);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn len(&self, path: &str) -> Result<usize> {
        self.control.observed(
            || format!("sizing configuration `{path}`"),
            |handle| raw::with_c_str(path, |path| handle.config_len(path)),
        )
    }

    /// The path of element `index` of the array entry at `path`: `solver` and
    /// `2` give `solver.2`.
    ///
    /// The index must be below [`Configuration::len`]. clingo itself accepts
    /// an offset past the end, so this is checked here; to reach a new
    /// element, [`Configuration::set`] it.
    ///
    /// The path is checked against clingo: it must resolve from the root to
    /// the key clingo gives for the element. A path clingo reads differently,
    /// such as `solver..`, is `InvalidInput` rather than a path that fails
    /// later.
    ///
    /// # Errors
    ///
    /// As [`Configuration::len`], and `InvalidInput` for an index that is not
    /// below the size or a path that does not lead to the element.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ErrorKind;
    ///
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// assert_eq!(conf.element("solver", 0)?, "solver.0");
    /// let err = conf.element("solver", 1).unwrap_err();
    /// assert_eq!(err.kind(), ErrorKind::InvalidInput);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn element(&self, path: &str, index: usize) -> Result<String> {
        self.control.observed(
            || format!("reading element {index} of configuration `{path}`"),
            |handle| raw::with_c_str(path, |path| handle.config_element(path, index)),
        )
    }

    /// Whether the map entry at `path` has a sub-entry `key`. The key may be
    /// dotted, and case matters. It is not bounded by [`Configuration::len`]:
    /// with three solvers, the root still has `solver.5.seed`.
    ///
    /// # Errors
    ///
    /// As [`Configuration::get`], and `InvalidInput` for an entry that is not
    /// a map.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// assert!(conf.has_key("solve", "models")?);
    /// assert!(!conf.has_key("solve", "nosuch")?);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn has_key(&self, path: &str, key: &str) -> Result<bool> {
        self.control.observed(
            || format!("looking up `{key}` in configuration `{path}`"),
            |handle| {
                raw::with_c_str(path, |path| {
                    raw::with_c_str(key, |key| handle.config_has_key(path, key))
                })
            },
        )
    }
}

impl fmt::Debug for Configuration<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Configuration").finish_non_exhaustive()
    }
}

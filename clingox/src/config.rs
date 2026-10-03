//! Reading and changing clingo's configuration by path.

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::fmt;
use std::marker::PhantomData;

use crate::control::ControlCore;
use crate::error::{Error, ErrorKind, Result};
use crate::raw;
use crate::segment::PathSegment;
use crate::walk::Walk;

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
/// **Paths or entries.** Each path method resolves its path from the root, so
/// it suits a single read or write. To walk the tree, or to read many entries,
/// take an entry with [`root`](Configuration::root) or
/// [`entry`](Configuration::entry) and step from it with
/// [`ConfigEntry::children`]: a step is one or two clingo calls and no path is
/// built. An entry only reads; [`set`](Configuration::set) stays here, by
/// path.
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
            |handle| raw::with_c_str(path, |path| handle.config_kind(path)).and_then(kind_of_bits),
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

/// The kind clingo's type bits stand for.
fn kind_of_bits(bits: u32) -> Result<ConfigKind> {
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
}

impl fmt::Debug for Configuration<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Configuration").finish_non_exhaustive()
    }
}

impl Configuration<'_> {
    /// The root entry of the tree, to walk it with [`ConfigEntry::children`].
    ///
    /// The entry borrows the view, not the control, so [`Configuration::set`]
    /// cannot run while an entry is alive, and nothing can use the control
    /// either, which the view holds mutably. Bind the view first, then take
    /// the entry: `ctl.configuration().root()?` does not compile when the
    /// entry is kept, because the temporary view is dropped at the end of the
    /// statement.
    ///
    /// # Errors
    ///
    /// Only [`ErrorKind::Poisoned`] for a poisoned control, and the error of
    /// closing a leftover search, as the other methods of the view; see
    /// [`Control::configuration`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ConfigKind;
    ///
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// let root = conf.root()?;
    /// assert_eq!(root.kind()?, ConfigKind::Map);
    /// assert_eq!(root.description()?, "Options");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    ///
    /// Keeping the entry past its view does not compile:
    ///
    /// ```compile_fail,E0716
    /// let mut ctl = clingox::Control::new()?;
    /// let root = ctl.configuration().root()?;
    /// root.kind()?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn root(&self) -> Result<ConfigEntry<'_>> {
        let control: &ControlCore = self.control;
        control
            .observed(
                || "reading the configuration".to_owned(),
                raw::ControlHandle::config_root,
            )
            .map(|key| ConfigEntry { control, key })
    }

    /// The entry at `path`, resolved once: it is
    /// [`root`](Configuration::root) followed by [`ConfigEntry::entry`], with
    /// one clingo call for the path. Read it as often as needed without
    /// resolving the path again.
    ///
    /// # Errors
    ///
    /// As [`Configuration::get`]: [`ErrorKind::Runtime`] for an unknown path,
    /// [`ErrorKind::Nul`] for a NUL byte, [`ErrorKind::Poisoned`] for a
    /// poisoned control. None of them poisons it.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// let models = conf.entry("solve.models")?;
    /// assert_eq!(models.value()?.as_deref(), Some("-1"));
    /// assert_eq!(models.value()?, conf.get("solve.models")?);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn entry(&self, path: &str) -> Result<ConfigEntry<'_>> {
        let control: &ControlCore = self.control;
        control
            .observed(
                || format!("resolving configuration `{path}`"),
                |handle| raw::with_c_str(path, |path| handle.config_entry(path)),
            )
            .map(|key| ConfigEntry { control, key })
    }
}

/// One entry of the configuration, held by clingo's key for it.
///
/// An entry is a small `Copy` value that reads its own kind, value,
/// description and names, and steps to a child by name, index or relative
/// path. A step costs one or two clingo calls (the name, then the entry) and
/// builds no path, which is what makes a walk through entries cheaper than the
/// same walk through the path methods of [`Configuration`]. An entry does not
/// know its own path: the iterator [`ConfigEntry::children`] hands each child
/// over with its [`PathSegment`].
///
/// An entry is read-only. It borrows the view that made it, so
/// [`Configuration::set`] cannot run while one is alive; to change a value
/// found by a walk, collect the paths first and set them afterwards (see
/// [`ConfigEntry::children`]). An entry is not `Send`, like the view.
///
/// Every method gives the result of the path method of [`Configuration`] with
/// the same name on the same entry, and its errors do not poison the control.
///
/// # Examples
///
/// ```
/// let mut ctl = clingox::Control::new()?;
/// let conf = ctl.configuration();
/// let solve = conf.entry("solve")?;
/// let models = solve.entry("models")?;
/// assert_eq!(models.value()?.as_deref(), Some("-1"));
/// assert_eq!(solve.len().unwrap_err().kind(), clingox::ErrorKind::InvalidInput);
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy)]
pub struct ConfigEntry<'a> {
    control: &'a ControlCore,
    key: raw::ConfigKey<'a>,
}

impl<'a> ConfigEntry<'a> {
    /// Runs one read of the entry. As a step of a children walk it skips
    /// [`ControlCore::observed`]'s search closing: the view closed any leftover
    /// search when it was made, and no search can open while the borrow lives,
    /// so `refusal` and `note` are all that is left to do per call. A
    /// poisoning while an entry lives cannot be produced through the public
    /// API (every way to poison needs the control mutably), so `refusal` is a
    /// check that nothing reaches today.
    fn read<T>(
        &self,
        context: impl FnOnce() -> String,
        f: impl FnOnce(raw::ConfigKey<'a>) -> Result<T>,
    ) -> Result<T> {
        self.control
            .refusal()
            .and_then(|()| f(self.key))
            .map_err(|err| self.control.note(err.context(context())))
    }

    /// What kind of entry this is.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unknown`], which poisons the control, if clingo reports a
    /// combination of types this version does not know, as
    /// [`Configuration::kind`] does.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ConfigKind;
    ///
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// assert_eq!(conf.entry("solve")?.kind()?, ConfigKind::Map);
    /// assert_eq!(conf.entry("solver")?.kind()?, ConfigKind::ArrayMap);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn kind(&self) -> Result<ConfigKind> {
        self.read(
            || "reading the kind of a configuration entry".to_owned(),
            |key| key.kind().and_then(kind_of_bits),
        )
    }

    /// The value as clingo prints it, or `None` for a map or array and for an
    /// option clingo leaves unassigned, such as `tester.solver.heuristic`.
    /// This is [`Configuration::get`] on the entry.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Utf8`] if the value is not UTF-8, and
    /// [`ErrorKind::Poisoned`] for a poisoned control.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// assert_eq!(conf.entry("solve.models")?.value()?.as_deref(), Some("-1"));
    /// assert_eq!(conf.entry("solve")?.value()?, None);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn value(&self) -> Result<Option<String>> {
        self.read(
            || "reading a configuration entry".to_owned(),
            raw::ConfigKey::value,
        )
    }

    /// The help text, exactly as clingo has it: it is not trimmed, and a `%A`
    /// stands for the option's argument. This is [`Configuration::description`]
    /// on the entry.
    ///
    /// # Errors
    ///
    /// As [`ConfigEntry::value`].
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// assert_eq!(conf.entry("solve")?.description()?, "Solve Options");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn description(&self) -> Result<String> {
        self.read(
            || "describing a configuration entry".to_owned(),
            raw::ConfigKey::description,
        )
    }

    /// The number of elements of an array entry, such as `solver`, which has
    /// one element per thread. This is [`Configuration::len`] on the entry, so
    /// an entry that is not an array is [`ErrorKind::InvalidInput`].
    ///
    /// It differs from [`StatsEntry::len`](crate::StatsEntry::len), which
    /// counts the children of any entry and is 0 for a value.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for an entry that is not an array, and
    /// [`ErrorKind::Poisoned`] for a poisoned control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ErrorKind;
    ///
    /// // One solver by default; `-t 3` would make it 3 where threads exist.
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// assert_eq!(conf.entry("solver")?.len()?, 1);
    /// assert_eq!(conf.entry("solve")?.len().unwrap_err().kind(), ErrorKind::InvalidInput);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    #[allow(
        clippy::len_without_is_empty,
        reason = "len is an error for an entry that is not an array, so an is_empty has no \
                  honest meaning for every entry"
    )]
    pub fn len(&self) -> Result<usize> {
        self.read(
            || "sizing a configuration entry".to_owned(),
            raw::ConfigKey::array_size,
        )
    }

    /// Element `index` of an array entry. The index must be below
    /// [`ConfigEntry::len`]: clingo accepts an offset past the end, so this is
    /// checked here, strictly, as [`Configuration::element`] does. To reach a
    /// new element, [`Configuration::set`] it.
    ///
    /// The child outlives this entry, so a walk can drop the parent.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for an entry that is not an array or an
    /// index that is not below the size, and [`ErrorKind::Poisoned`] for a
    /// poisoned control. Neither poisons it.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ErrorKind;
    ///
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// let first = conf.entry("solver")?.element(0)?;
    /// assert_eq!(first.entry("seed")?.value()?.as_deref(), Some("1"));
    /// let err = conf.entry("solver")?.element(1).unwrap_err();
    /// assert_eq!(err.kind(), ErrorKind::InvalidInput);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn element(&self, index: usize) -> Result<ConfigEntry<'a>> {
        let control = self.control;
        self.read(
            || format!("reading element {index} of a configuration entry"),
            |key| key.element(index),
        )
        .map(|key| ConfigEntry { control, key })
    }

    /// The names under a map entry, in clingo's order. A value or a plain
    /// array has none. This is [`Configuration::keys`] on the entry.
    ///
    /// For `solver`, which is both an array and a map, these are the names of
    /// its map side, the options of element 0, while
    /// [`children`](ConfigEntry::children) yields its elements.
    ///
    /// # Errors
    ///
    /// As [`ConfigEntry::value`].
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// let keys = conf.entry("solve")?.keys()?;
    /// assert!(keys.iter().any(|k| k == "models"));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn keys(&self) -> Result<Vec<String>> {
        self.read(
            || "listing a configuration entry".to_owned(),
            raw::ConfigKey::keys,
        )
    }

    /// The entry at `path`, relative to this one, resolved with one clingo
    /// call. It resolves a path exactly as [`Configuration::entry`] resolves
    /// one from the root: names separated by dots, a number selecting an array
    /// element, a name on `solver` going through element 0. The empty path is
    /// the entry itself, without a clingo call. The path is relative: `solve`
    /// below the entry `solve` is an error.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Runtime`] for an unknown path, including a step below a
    /// value, and [`ErrorKind::Nul`] for a NUL byte. Neither poisons the
    /// control. The message names the path.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut ctl = clingox::Control::new()?;
    /// let conf = ctl.configuration();
    /// let solve = conf.entry("solve")?;
    /// assert_eq!(solve.entry("models")?.value()?, conf.get("solve.models")?);
    /// assert_eq!(solve.entry("")?.description()?, solve.description()?);
    /// assert!(solve.entry("nosuch").is_err());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn entry(&self, path: &str) -> Result<ConfigEntry<'a>> {
        let control = self.control;
        self.read(
            || format!("resolving `{path}` from a configuration entry"),
            |key| raw::with_c_str(path, |path| key.lookup(path)),
        )
        .map(|key| ConfigEntry { control, key })
    }

    /// The children of the entry, each with its [`PathSegment`]: one clingo
    /// call per element of an array, two per entry of a map (its name, then
    /// the entry).
    ///
    /// - An array or array-map yields its elements as `Index(0..len)`. The
    ///   `solver` entry is both an array and a map, and is walked as an array:
    ///   its map side names the options of element 0, which element 0 yields
    ///   already, so each node of the tree is visited once. The map side stays
    ///   reachable through [`keys`](ConfigEntry::keys) and
    ///   [`entry`](ConfigEntry::entry).
    /// - A map yields its names in clingo's order as `Name`.
    /// - A value yields nothing.
    ///
    /// The iterator returns an error once and then ends.
    ///
    /// A walk cannot change a value, because the entries borrow the view. To
    /// set what a walk finds, collect the paths and set them afterwards.
    ///
    /// # Errors
    ///
    /// As [`ConfigEntry::kind`], for the call that reads the entry's kind and
    /// size.
    ///
    /// # Examples
    ///
    /// Collecting the path of every value, then changing one of them once the
    /// walk is over:
    ///
    /// ```
    /// use clingox::{ConfigEntry, ConfigKind};
    ///
    /// fn collect(
    ///     entry: ConfigEntry<'_>,
    ///     path: &mut Vec<String>,
    ///     out: &mut Vec<String>,
    /// ) -> clingox::Result<()> {
    ///     match entry.kind()? {
    ///         ConfigKind::Value => out.push(path.join(".")),
    ///         _ => {
    ///             for child in entry.children()? {
    ///                 let (segment, child) = child?;
    ///                 path.push(segment.to_string());
    ///                 collect(child, path, out)?;
    ///                 path.pop();
    ///             }
    ///         }
    ///     }
    ///     Ok(())
    /// }
    ///
    /// let mut ctl = clingox::Control::new()?;
    /// let mut conf = ctl.configuration();
    /// let mut paths = Vec::new();
    /// collect(conf.root()?, &mut Vec::new(), &mut paths)?;
    /// assert!(paths.contains(&"solve.models".to_owned()));
    /// assert!(paths.contains(&"solver.0.seed".to_owned()));
    /// conf.set("solve.models", "0")?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn children(&self) -> Result<ConfigChildren<'a>> {
        let (side, walk) = self.read(
            || "listing the children of a configuration entry".to_owned(),
            |key| {
                Ok(match kind_of_bits(key.kind()?)? {
                    ConfigKind::Array | ConfigKind::ArrayMap => {
                        let len = key.array_size_of_array()?;
                        (Side::Elements(len), Walk::new(len))
                    }
                    ConfigKind::Map => (Side::Names, Walk::new(key.map_size()?)),
                    ConfigKind::Value => (Side::Names, Walk::finished()),
                })
            },
        )?;
        Ok(ConfigChildren {
            entry: *self,
            side,
            walk,
        })
    }
}

impl fmt::Debug for ConfigEntry<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// A value, or the error that kept clingo from reporting it.
        struct Read<T>(Result<T>);
        impl<T: fmt::Debug> fmt::Debug for Read<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match &self.0 {
                    Ok(value) => value.fmt(f),
                    Err(err) => write!(f, "<{err}>"),
                }
            }
        }
        let mut debug = f.debug_struct("ConfigEntry");
        let kind = self.kind();
        let is_value = matches!(kind, Ok(ConfigKind::Value));
        debug.field("kind", &Read(kind));
        if is_value {
            debug.field("value", &Read(self.value()));
        }
        debug.finish()
    }
}

/// Which children a [`ConfigChildren`] reads, and how many elements an array
/// has, so that each step needs no size read.
#[derive(Clone, Copy)]
enum Side {
    Elements(usize),
    Names,
}

/// The children of a configuration entry, made by [`ConfigEntry::children`].
///
/// It yields each child with its [`PathSegment`]: `Index` for the elements of
/// an array, `Name` for the entries of a map. After an error it returns that
/// error once and then `None`, and it is fused.
///
/// # Examples
///
/// ```
/// use clingox::PathSegment;
///
/// let mut ctl = clingox::Control::new()?;
/// let conf = ctl.configuration();
/// let mut children = conf.entry("solver")?.children()?;
/// let (segment, first) = children.next().expect("one solver")?;
/// assert_eq!(segment, PathSegment::Index(0));
/// assert_eq!(first.entry("seed")?.value()?.as_deref(), Some("1"));
/// assert!(children.next().is_none());
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct ConfigChildren<'a> {
    entry: ConfigEntry<'a>,
    side: Side,
    walk: Walk,
}

impl<'a> Iterator for ConfigChildren<'a> {
    type Item = Result<(PathSegment, ConfigEntry<'a>)>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entry;
        let side = self.side;
        self.walk.step(|position| {
            let control = entry.control;
            entry
                .read(
                    || "listing the children of a configuration entry".to_owned(),
                    |key| match side {
                        Side::Elements(len) => Ok((
                            PathSegment::Index(position),
                            key.element_below(position, len)?,
                        )),
                        Side::Names => {
                            let (name, child) = key.map_entry(position)?;
                            Ok((PathSegment::Name(name), child))
                        }
                    },
                )
                .map(|(segment, key)| (segment, ConfigEntry { control, key }))
        })
    }
}

impl std::iter::FusedIterator for ConfigChildren<'_> {}

impl fmt::Debug for ConfigChildren<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigChildren").finish_non_exhaustive()
    }
}

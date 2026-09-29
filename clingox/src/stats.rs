//! Reading clingo's statistics by path, and owned snapshots of them.

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::fmt;

use crate::control::ControlCore;
use crate::error::Result;
use crate::raw;

/// A view of the statistics of a control's last solve call, read by path.
///
/// A path names entries separated by dots: map entries by name and array
/// elements by index (`summary.times.total`, `summary.costs.0`). The empty
/// path is the root. What clingo 5.8.2 reports with default options:
///
/// - the root has `problem`, `solving` and `summary`; after the first solve
///   call also `user_step` and `user_accu`, and with `--stats` also `accu`;
/// - `summary.call` counts solve calls from 0, `summary.models.enumerated`
///   and `summary.models.optimal` count the models of the last call, and
///   `summary.costs` has one entry per priority level;
/// - `problem.lp.atoms` and `problem.lp.rules` describe the ground program.
///
/// Values read before the first solve call are not meaningful:
/// `summary.times.total` holds a wall-clock timestamp until then. On
/// WebAssembly, `summary.times.cpu` is 0, because Emscripten's `getrusage`
/// returns a constant.
///
/// The view borrows the control (DESIGN S5), so the control cannot ground or
/// solve while it is alive. [`Statistics::snapshot`] copies the tree into a
/// [`StatsTree`] that can outlive it.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::with_args(["--models=0"])?;
/// ctl.add_base("{a;b}.")?;
/// ctl.ground(&[Part::base()])?;
/// ctl.solve(&[])?;
/// let stats = ctl.statistics()?;
/// assert_eq!(stats.value("summary.models.enumerated")?, 4.0);
/// assert!(stats.keys("summary")?.iter().any(|k| k == "times"));
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct Statistics<'c> {
    control: &'c ControlCore,
    stats: raw::Stats<'c>,
}

impl ScopedControl<'_> {
    /// A view of the statistics of the last solve call.
    ///
    /// A search left open by a forgotten handle is closed first (DESIGN S4):
    /// clingo forbids reading statistics during a search.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`](crate::ErrorKind::Poisoned) if an earlier
    ///   error poisoned the control;
    /// - [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs
    ///   out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// let ctl = clingox::Control::new()?;
    /// let stats = ctl.statistics()?;
    /// assert!(stats.keys("")?.iter().any(|k| k == "summary"));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn statistics(&self) -> Result<Statistics<'_>> {
        let stats = self.core.observed(
            || "reading statistics".to_owned(),
            raw::ControlHandle::statistics,
        )?;
        Ok(Statistics {
            control: &self.core,
            stats,
        })
    }
}

impl<'c> Statistics<'c> {
    /// The number at the value entry `path`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Runtime`](crate::ErrorKind::Runtime) for an unknown path,
    ///   an array index out of range, or a map or array at `path`. The message
    ///   names the path. clingo itself would raise logic errors here, which
    ///   poison; clingox checks the path first, so these do not;
    /// - [`ErrorKind::Poisoned`](crate::ErrorKind::Poisoned) if an earlier
    ///   error poisoned the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ErrorKind;
    ///
    /// let ctl = clingox::Control::new()?;
    /// let stats = ctl.statistics()?;
    /// assert_eq!(stats.value("summary.models.enumerated")?, 0.0);
    /// assert_eq!(stats.value("summary").unwrap_err().kind(), ErrorKind::Runtime);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn value(&self, path: &str) -> Result<f64> {
        self.read(path, |stats| stats.value(path))
    }

    /// The names of the map at `path` in clingo's order, or the indices of the
    /// array there as text (`0`, `1`, ...), so that each key extends the path.
    /// A value has no keys.
    ///
    /// # Errors
    ///
    /// As [`Statistics::value`], except that any entry type is accepted.
    ///
    /// # Examples
    ///
    /// ```
    /// let ctl = clingox::Control::new()?;
    /// let stats = ctl.statistics()?;
    /// assert_eq!(stats.keys("summary.models")?, ["enumerated", "optimal"]);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn keys(&self, path: &str) -> Result<Vec<String>> {
        self.read(path, |stats| stats.keys(path))
    }

    /// Copies the whole tree. Maps keep clingo's order.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`](crate::ErrorKind::Poisoned) if an earlier
    ///   error poisoned the control;
    /// - [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs
    ///   out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// let ctl = clingox::Control::new()?;
    /// let tree = ctl.statistics()?.snapshot()?;
    /// assert_eq!(tree.value("summary.models.enumerated"), Some(0.0));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn snapshot(&self) -> Result<StatsTree> {
        self.read("", raw::Stats::snapshot)
    }

    fn read<T>(&self, path: &str, f: impl FnOnce(raw::Stats<'c>) -> Result<T>) -> Result<T> {
        self.control
            .observed(|| format!("reading statistics `{path}`"), |_| f(self.stats))
    }
}

impl fmt::Debug for Statistics<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Statistics").finish_non_exhaustive()
    }
}

/// The kind of a statistics entry a caller may ask
/// [`MutableStatistics::push_array`] or [`MutableStatistics::add_map_key`]
/// to create (clingo.h:2100-2106).
///
/// `#[non_exhaustive]`, as clingox's other public enums that mirror a C one
/// are (RULES §4): clingo's statistics types are `value`, `array`, `map`
/// and `empty`, but `empty` is never a kind to *create* (there is no
/// `clingo_statistics_type_empty` you would ever pass to `push_array` or
/// `add_map_key`; an entry becomes `empty` only implicitly), so it is not a
/// variant here.
///
/// # Examples
///
/// ```
/// use clingox::StatKind;
///
/// assert_eq!(StatKind::Value, StatKind::Value);
/// assert_ne!(StatKind::Value, StatKind::Array);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatKind {
    /// A single number.
    Value,
    /// A list of entries, reached by index.
    Array,
    /// Named entries.
    Map,
}

/// A writable view of one solve's statistics, lent to
/// [`SolveEventHandler::on_statistics`](crate::SolveEventHandler::on_statistics).
///
/// The read side ([`value`](MutableStatistics::value),
/// [`keys`](MutableStatistics::keys)) is identical to [`Statistics`]'s own,
/// reusing the same C functions through this object's own, genuinely non-const
/// pointer (C narrows it to `const` implicitly, exactly as [`Statistics`]
/// itself is built on the `const *` `clingo_control_statistics` returns):
/// behaviour, error kinds and path syntax (dot-separated map names and array
/// indices) are exactly [`Statistics`]'s, not a second implementation. The
/// write methods are new: [`set_value`](MutableStatistics::set_value),
/// [`push_array`](MutableStatistics::push_array) and
/// [`add_map_key`](MutableStatistics::add_map_key). The C API has no "create
/// this whole path" call, only "add one subkey to an existing map or array
/// entry," so building a tree from scratch takes one call per new node (see the
/// guide's chapter on solve events and user statistics for a worked example).
///
/// Each write method checks the resolved entry's own type before calling
/// clingo, exactly as the read side already does before every descent and every
/// value read (`clingo` itself would raise a *logic* error for the wrong kind,
/// which would poison the control unconditionally, DESIGN S3); a mismatch here
/// is [`ErrorKind::Runtime`](crate::ErrorKind::Runtime) instead, and does not
/// poison.
///
/// **Nothing here can outlive the callback that received it**, the same
/// lifetime rule [`ExtendableModel`](crate::ExtendableModel) follows;
/// `tests/ui/mutable_statistics_escapes_the_callback.rs` pins the compile
/// error.
///
/// **U19.** Before patch U19 (`docs/dev/UPSTREAM-ISSUES.md`), clasp's
/// statistics registry was unsafe to extend concurrently from more than one
/// solver thread; U19's original note covered clasp's own statistics kinds and
/// statistics *reading*, and is extended here to *user*-defined statistics too,
/// since `push_array`/`add_map_key` call the very registration path U19 patches
/// (`clingo_statistics_map_add_subkey`/`clingo_statistics_array_push`). On the
/// vendored build this is safe under multiple solver threads (a system clingo
/// keeps the unpatched race).
///
/// # Examples
///
/// ```
/// use std::ops::ControlFlow;
///
/// use clingox::{Control, MutableStatistics, Part, SolveEventHandler, SolveOptions, StatKind};
///
/// struct RecordACount;
///
/// impl SolveEventHandler for RecordACount {
///     fn on_statistics(
///         &mut self,
///         step: &mut MutableStatistics<'_>,
///         _accumulated: &mut MutableStatistics<'_>,
///     ) -> clingox::Result<ControlFlow<()>> {
///         step.add_map_key("", "mine", StatKind::Value)?;
///         step.set_value("mine", 1.0)?;
///         Ok(ControlFlow::Continue(()))
///     }
/// }
///
/// let mut ctl = Control::with_args(["--stats=2"])?;
/// ctl.add_base("a.")?;
/// ctl.ground(&[Part::base()])?;
/// ctl.solve_with_events(SolveOptions::new(), RecordACount)?;
/// assert_eq!(ctl.statistics()?.value("user_step.mine")?, 1.0);
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct MutableStatistics<'a> {
    stats: raw::MutableStats<'a>,
}

impl<'a> MutableStatistics<'a> {
    /// Wraps an already-validated, non-const statistics object. The
    /// `unsafe` cast from clingo's raw pointer happens once, in
    /// `raw::MutableStats::new`, called only from the composed solve-event
    /// trampoline (`raw::events`); this constructor itself needs no
    /// `unsafe`.
    pub(crate) fn from_raw(stats: raw::MutableStats<'a>) -> MutableStatistics<'a> {
        MutableStatistics { stats }
    }

    /// The number at the value entry `path`, as [`Statistics::value`].
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Runtime`](crate::ErrorKind::Runtime) for an unknown
    /// path, an array index out of range, or a map or array at `path`.
    pub fn value(&self, path: &str) -> Result<f64> {
        self.stats
            .value(path)
            .map_err(|e| e.context(format!("reading statistics `{path}`")))
    }

    /// The keys at `path`, as [`Statistics::keys`].
    ///
    /// # Errors
    ///
    /// As [`MutableStatistics::value`], except that any entry type is
    /// accepted.
    pub fn keys(&self, path: &str) -> Result<Vec<String>> {
        self.stats
            .keys(path)
            .map_err(|e| e.context(format!("reading statistics `{path}`")))
    }

    /// Sets the number at the value entry `path`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Runtime`](crate::ErrorKind::Runtime) for an unknown
    /// path or an entry that is not a value (a map or an array); it does
    /// not poison the control.
    pub fn set_value(&mut self, path: &str, value: f64) -> Result<()> {
        self.stats
            .set_value(path, value)
            .map_err(|e| e.context(format!("writing statistics `{path}`")))
    }

    /// Creates a new entry at the end of the array at `path`, of the given
    /// kind, and returns its index: the array's own size just before the
    /// push, which the caller needs to address the new element (there is
    /// no separate "get me the last element" call).
    ///
    /// **clingo does not require every element of an array to share one
    /// kind**: nothing here checks the kind
    /// of an array's existing elements against `kind`, because clingo
    /// itself does not either; a `push_array` call with a different `kind`
    /// each time builds a mixed-kind array without error.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Runtime`](crate::ErrorKind::Runtime) for an unknown
    /// path or an entry that is not an array; it does not poison the
    /// control.
    pub fn push_array(&mut self, path: &str, kind: StatKind) -> Result<usize> {
        self.stats
            .push_array(path, kind)
            .map_err(|e| e.context(format!("writing statistics `{path}`")))
    }

    /// Adds a subkey named `name` to the map at `path`, of the given kind.
    ///
    /// # Errors
    ///
    /// Every kind that occurs here is listed, not only the two most common
    /// ones.
    ///
    /// - [`ErrorKind::Runtime`](crate::ErrorKind::Runtime) for an unknown path
    ///   or an entry that is not a map;
    /// - [`ErrorKind::InvalidInput`](crate::ErrorKind::InvalidInput) if `name`
    ///   is empty or contains `.`, the path separator this type's own lookups
    ///   use, since no later `value`/`keys`/`snapshot` call could ever address
    ///   such a subkey again;
    /// - [`ErrorKind::Nul`](crate::ErrorKind::Nul) if `name` contains a NUL
    ///   byte;
    /// - [`ErrorKind::Logic`](crate::ErrorKind::Logic), which poisons the
    ///   control, if clingo itself refuses the subkey (for instance, adding one
    ///   after the statistics have already been finalized for this step).
    ///
    /// None of the first three poisons the control.
    pub fn add_map_key(&mut self, path: &str, name: &str, kind: StatKind) -> Result<()> {
        self.stats
            .add_map_key(path, name, kind)
            .map_err(|e| e.context(format!("writing statistics `{path}.{name}`")))
    }
}

impl fmt::Debug for MutableStatistics<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutableStatistics").finish_non_exhaustive()
    }
}

/// An owned copy of clingo's statistics, made by [`Statistics::snapshot`].
///
/// It is `Send + Sync + 'static` (DESIGN S12). Paths are those of
/// [`Statistics`]; where [`Statistics`] returns an error, [`StatsTree::get`]
/// and [`StatsTree::value`] return `None`.
///
/// # Examples
///
/// ```
/// use clingox::StatsTree;
///
/// let ctl = clingox::Control::new()?;
/// let tree = ctl.statistics()?.snapshot()?;
/// match tree.get("summary.models") {
///     Some(StatsTree::Map(entries)) => assert_eq!(entries[0].0, "enumerated"),
///     other => panic!("summary.models is a map: {other:?}"),
/// }
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StatsTree {
    /// A number.
    Value(f64),
    /// A list of entries, reached by index.
    Array(Vec<StatsTree>),
    /// Named entries in clingo's order. An entry of clingo's type `empty`,
    /// which is neither a value, an array nor a map, is an empty map.
    Map(Vec<(String, StatsTree)>),
}

impl StatsTree {
    /// The entry at `path`, or `None` if there is none. The empty path is the
    /// tree itself.
    pub fn get(&self, path: &str) -> Option<&StatsTree> {
        if path.is_empty() {
            return Some(self);
        }
        path.split('.').try_fold(self, |tree, part| match tree {
            StatsTree::Map(entries) => entries
                .iter()
                .find(|(name, _)| name == part)
                .map(|(_, entry)| entry),
            StatsTree::Array(elements) => elements.get(part.parse::<usize>().ok()?),
            StatsTree::Value(_) => None,
        })
    }

    /// The number at `path`, or `None` if there is no value there.
    pub fn value(&self, path: &str) -> Option<f64> {
        match self.get(path)? {
            StatsTree::Value(value) => Some(*value),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> StatsTree {
        StatsTree::Map(vec![
            ("b".to_owned(), StatsTree::Value(1.0)),
            (
                "a".to_owned(),
                StatsTree::Array(vec![StatsTree::Value(2.0), StatsTree::Map(Vec::new())]),
            ),
        ])
    }

    #[test]
    fn paths_reach_map_entries_and_array_elements() {
        let tree = tree();
        assert_eq!(tree.value("b"), Some(1.0));
        assert_eq!(tree.value("a.0"), Some(2.0));
        assert_eq!(tree.get("a.1"), Some(&StatsTree::Map(Vec::new())));
        assert_eq!(tree.get(""), Some(&tree));
    }

    #[test]
    fn bad_paths_give_none() {
        let tree = tree();
        for path in ["c", "a.2", "a.x", "b.0", "a..0", "."] {
            assert_eq!(tree.get(path), None, "{path}");
        }
        assert_eq!(tree.value("a"), None, "an array is not a value");
    }

    #[test]
    fn stats_trees_are_send_and_sync() {
        fn traits<T: Send + Sync + 'static>() {}
        traits::<StatsTree>();
    }
}

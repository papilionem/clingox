//! Wrappers for the statistics (clingo.h:2100-2239, 3205).
//!
//! clingo reports an unknown key and reading a map as a value as logic errors
//! (`StatsMap::at`, `type error`), which would poison the control (DESIGN S3).
//! So every path is walked here one level at a time, checking each key and
//! the type of each entry before clingo is asked, and a bad path is a runtime
//! error.

use std::ffi::{CStr, CString, c_char};
use std::marker::PhantomData;

use clingox_sys as ffi;

use super::control::ControlHandle;
use super::{borrowed_str, call, query, with_c_str};
use crate::StatsTree;
use crate::error::{Error, ErrorKind};
use crate::stats::StatKind;

/// The type of a statistics entry (clingo.h:2100-2106).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatsKind {
    /// clingo's `empty`: an entry that is neither of the others.
    Empty,
    Value,
    Array,
    Map,
}

/// A control's statistics object and its root key, valid while the control is
/// borrowed shared.
///
/// Nothing can ground or solve through `&ControlHandle`, and no search is open
/// while one is borrowed: `ControlHandle::statistics` is only called after a
/// leftover search was closed, and starting one takes `&mut`. clingo.h only
/// forbids reading statistics during a search (clingo.h:3195-3200).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Stats<'c> {
    object: *const ffi::clingo_statistics_t,
    root: u64,
    _control: PhantomData<&'c ControlHandle>,
}

impl ControlHandle {
    /// The statistics of the last solve call (clingo.h:3205, 2119). The caller
    /// must have closed any leftover search.
    pub(crate) fn statistics(&self) -> Result<Stats<'_>, Error> {
        let mut stats: *const ffi::clingo_statistics_t = std::ptr::null();
        let mut root = 0;
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, no search
            // is open (`logged` closed it), and `stats` is a valid out-pointer.
            // The object belongs to the control (clingo.h:3205).
            unsafe { ffi::clingo_control_statistics(control, &raw mut stats) }
        })?;
        // SAFETY: `stats` was just returned by clingo, and `root` is a valid
        // out-pointer (clingo.h:2119).
        query(|| unsafe { ffi::clingo_statistics_root(stats, &raw mut root) })?;
        Ok(Stats {
            object: stats,
            root,
            _control: PhantomData,
        })
    }
}

impl<'c> Stats<'c> {
    /// The root entry.
    pub(crate) fn root_key(self) -> StatsKey<'c> {
        StatsKey {
            stats: self,
            key: self.root,
        }
    }

    /// The number at `path`.
    pub(crate) fn value(self, path: &str) -> Result<f64, Error> {
        self.root_key().lookup(path)?.value()
    }

    /// The names of the map at `path`, or the indices of the array there as
    /// text; none for a value.
    pub(crate) fn keys(self, path: &str) -> Result<Vec<String>, Error> {
        self.root_key().lookup(path)?.keys()
    }

    /// A copy of the whole tree, in clingo's order.
    pub(crate) fn snapshot(self) -> Result<StatsTree, Error> {
        self.root_key().copy()
    }
}

/// What an entry holds, as a walk needs it: nothing below it, or elements or
/// names with their count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    /// A value, or clingo's `empty`: no children.
    Leaf,
    /// An array of this many elements.
    Array(usize),
    /// A map of this many entries.
    Map(usize),
}

impl Shape {
    /// The number of children.
    pub(crate) fn len(self) -> usize {
        match self {
            Shape::Leaf => 0,
            Shape::Array(len) | Shape::Map(len) => len,
        }
    }
}

/// An entry of the statistics: the object and the entry's key.
///
/// `'c` is the borrow of the control (or of the statistics event) the object
/// was made under. While it lives nothing can ground, solve or write the
/// statistics, so a key stays what clingo returned it for. Only this module
/// builds one, from the root or from a value clingo returned for this object
/// (`map_at`, `array_at`), so safe code can neither invent a key nor pair it
/// with another object's (DESIGN S5). The raw pointer in `Stats` keeps
/// it `!Send` and `!Sync`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StatsKey<'c> {
    stats: Stats<'c>,
    key: u64,
}

impl<'c> StatsKey<'c> {
    /// The type of the entry (clingo.h:2126).
    fn kind(self) -> Result<StatsKind, Error> {
        let mut kind = 0;
        // SAFETY: `stats.object` is valid for `'c` (see `Stats`), `key` was
        // returned by clingo for it, and `kind` is a valid out-pointer
        // (clingo.h:2126).
        query(|| unsafe {
            ffi::clingo_statistics_type(self.stats.object, self.key, &raw mut kind)
        })?;
        Ok(match u32::try_from(kind) {
            Ok(ffi::clingo_statistics_type_value) => StatsKind::Value,
            Ok(ffi::clingo_statistics_type_array) => StatsKind::Array,
            Ok(ffi::clingo_statistics_type_map) => StatsKind::Map,
            _ => StatsKind::Empty,
        })
    }

    /// The kind of the entry as [`StatKind`] has it: clingo's `empty` is a
    /// map, as the snapshot copies it.
    pub(crate) fn stat_kind(self) -> Result<StatKind, Error> {
        Ok(match self.kind()? {
            StatsKind::Value => StatKind::Value,
            StatsKind::Array => StatKind::Array,
            StatsKind::Map | StatsKind::Empty => StatKind::Map,
        })
    }

    /// What the entry holds, from one type read and at most one size read.
    pub(crate) fn shape(self) -> Result<Shape, Error> {
        Ok(match self.kind()? {
            StatsKind::Array => Shape::Array(self.array_size()?),
            StatsKind::Map => Shape::Map(self.map_size()?),
            StatsKind::Value | StatsKind::Empty => Shape::Leaf,
        })
    }

    /// The number of elements of the array (clingo.h:2139).
    fn array_size(self) -> Result<usize, Error> {
        let mut size = 0;
        // SAFETY: as in `kind`; the caller checked that the entry is an array,
        // as clingo.h requires (clingo.h:2139).
        query(|| unsafe {
            ffi::clingo_statistics_array_size(self.stats.object, self.key, &raw mut size)
        })?;
        Ok(size)
    }

    /// Element `index` of the array (clingo.h:2149).
    ///
    /// The caller checked that the entry is an array with more than `index`
    /// elements: a children walk holds the size from [`StatsKey::shape`] and
    /// stops at it, and a path walk compares against [`StatsKey::array_size`].
    pub(crate) fn array_at(self, index: usize) -> Result<StatsKey<'c>, Error> {
        let mut element = 0;
        // SAFETY: as in `kind`; the caller checked that the entry is an array
        // with more than `index` elements (clingo.h:2149).
        query(|| unsafe {
            ffi::clingo_statistics_array_at(self.stats.object, self.key, index, &raw mut element)
        })?;
        Ok(StatsKey {
            key: element,
            ..self
        })
    }

    /// The number of entries of the map (clingo.h:2173).
    fn map_size(self) -> Result<usize, Error> {
        let mut size = 0;
        // SAFETY: as in `kind`; the caller checked that the entry is a map
        // (clingo.h:2173).
        query(|| unsafe {
            ffi::clingo_statistics_map_size(self.stats.object, self.key, &raw mut size)
        })?;
        Ok(size)
    }

    /// The name of entry `index` of the map, copied (clingo.h:2193).
    fn map_name(self, index: usize) -> Result<String, Error> {
        let mut name: *const c_char = std::ptr::null();
        // SAFETY: as in `kind`; the caller checked that the entry is a map with
        // more than `index` entries (clingo.h:2193).
        query(|| unsafe {
            ffi::clingo_statistics_map_subkey_name(
                self.stats.object,
                self.key,
                index,
                &raw mut name,
            )
        })?;
        // SAFETY: clingo returns a NUL-terminated name owned by the statistics,
        // which do not change while the control is borrowed; it is copied at
        // once.
        unsafe { borrowed_str(name) }.map(str::to_owned)
    }

    /// Whether the map has an entry `name` (clingo.h:2183).
    fn has_subkey(self, name: &CStr) -> Result<bool, Error> {
        let mut present = false;
        // SAFETY: as in `kind`; the caller checked that the entry is a map,
        // `name` is NUL-terminated and outlives the call, and `present` is a
        // valid out-pointer (clingo.h:2183).
        query(|| unsafe {
            ffi::clingo_statistics_map_has_subkey(
                self.stats.object,
                self.key,
                name.as_ptr(),
                &raw mut present,
            )
        })?;
        Ok(present)
    }

    /// The entry `name` of the map, which [`StatsKey::has_subkey`] confirmed
    /// (clingo.h:2204).
    fn map_at_present(self, name: &CStr) -> Result<StatsKey<'c>, Error> {
        let mut entry = 0;
        // SAFETY: as in `kind`; the entry is a map that has `name`, so clingo
        // raises no logic error, `name` is NUL-terminated and outlives the
        // call, and `entry` is a valid out-pointer (clingo.h:2204).
        query(|| unsafe {
            ffi::clingo_statistics_map_at(
                self.stats.object,
                self.key,
                name.as_ptr(),
                &raw mut entry,
            )
        })?;
        Ok(StatsKey { key: entry, ..self })
    }

    /// The entry `name` of the map, or `None` if there is no such entry. The
    /// presence check stays for every name, listed by clingo or not, so that
    /// no clingo logic error can come from here.
    fn map_at(self, name: &str) -> Result<Option<StatsKey<'c>>, Error> {
        // A name with a NUL byte cannot be a key.
        if name.contains('\0') {
            return Ok(None);
        }
        with_c_str(name, |name| {
            if !self.has_subkey(name)? {
                return Ok(None);
            }
            self.map_at_present(name).map(Some)
        })
    }

    /// Entry `index` of a map with more than `index` entries, with its name.
    ///
    /// A name with a `.` could not be addressed by a path, and clasp would
    /// read it as one in `map_at`. None occurs in clingo 5.8.2's statistics
    /// (checked in three configurations), and a user statistic cannot get one through clingox, so this is
    /// a check that nothing reaches; it keeps a name from another binding from
    /// turning into a different lookup.
    pub(crate) fn map_entry(self, index: usize) -> Result<(String, StatsKey<'c>), Error> {
        let name = self.map_name(index)?;
        if name.contains('.') {
            return Err(Error::new(
                ErrorKind::Runtime,
                format!("clingo lists the name `{name}`, which contains the path separator"),
            ));
        }
        match self.map_at(&name)? {
            Some(entry) => Ok((name, entry)),
            None => Err(Error::new(
                ErrorKind::Runtime,
                format!("clingo lists `{name}` but has no entry for it"),
            )),
        }
    }

    /// The number at the value entry (clingo.h:2230).
    fn value_at(self) -> Result<f64, Error> {
        let mut value = 0.0;
        // SAFETY: as in `kind`; the caller checked that the entry is a value
        // (clingo.h:2230).
        query(|| unsafe {
            ffi::clingo_statistics_value_get(self.stats.object, self.key, &raw mut value)
        })?;
        Ok(value)
    }

    /// The entry one path part below this one, checked at this level.
    fn step(self, part: &str) -> Result<StatsKey<'c>, Error> {
        match self.kind()? {
            StatsKind::Map => self.map_at(part)?,
            StatsKind::Array => match part.parse::<usize>() {
                Ok(index) if index < self.array_size()? => Some(self.array_at(index)?),
                _ => None,
            },
            StatsKind::Value | StatsKind::Empty => None,
        }
        .ok_or_else(|| Error::new(ErrorKind::Runtime, format!("no entry `{part}`")))
    }

    /// The entry at `path` below this one, walked one checked level at a
    /// time. The empty path is this entry.
    pub(crate) fn lookup(self, path: &str) -> Result<StatsKey<'c>, Error> {
        if path.is_empty() {
            return Ok(self);
        }
        path.split('.').try_fold(self, StatsKey::step)
    }

    /// The number at the entry. A map, array or `empty` is a runtime error.
    pub(crate) fn value(self) -> Result<f64, Error> {
        match self.kind()? {
            StatsKind::Value => self.value_at(),
            kind => Err(Error::new(
                ErrorKind::Runtime,
                format!("the entry is {}, not a value", kind_name(kind)),
            )),
        }
    }

    /// The names of the map, or the indices of the array as text; none for a
    /// value.
    pub(crate) fn keys(self) -> Result<Vec<String>, Error> {
        match self.kind()? {
            StatsKind::Map => (0..self.map_size()?)
                .map(|index| self.map_name(index))
                .collect(),
            StatsKind::Array => Ok((0..self.array_size()?).map(|i| i.to_string()).collect()),
            StatsKind::Value | StatsKind::Empty => Ok(Vec::new()),
        }
    }

    /// A copy of the tree below the entry. An entry of clingo's type `empty`
    /// is copied as an empty map.
    fn copy(self) -> Result<StatsTree, Error> {
        Ok(match self.kind()? {
            StatsKind::Value => StatsTree::Value(self.value_at()?),
            StatsKind::Array => StatsTree::Array(
                (0..self.array_size()?)
                    .map(|index| self.array_at(index)?.copy())
                    .collect::<Result<_, Error>>()?,
            ),
            StatsKind::Map => StatsTree::Map(
                (0..self.map_size()?)
                    .map(|index| {
                        let name = self.map_name(index)?;
                        let entry = self.map_at(&name)?.ok_or_else(|| {
                            Error::new(
                                ErrorKind::Unknown,
                                format!("clingo lists `{name}` but has no entry for it"),
                            )
                        })?;
                        Ok((name, entry.copy()?))
                    })
                    .collect::<Result<_, Error>>()?,
            ),
            StatsKind::Empty => StatsTree::Map(Vec::new()),
        })
    }
}

fn kind_name(kind: StatsKind) -> &'static str {
    match kind {
        StatsKind::Empty => "empty",
        StatsKind::Value => "a value",
        StatsKind::Array => "an array",
        StatsKind::Map => "a map",
    }
}

/// clingo's raw type constant for a [`StatKind`] (clingo.h:2100-2106), the
/// reverse of `StatsKind`'s own read-side mapping in [`Stats::kind`]: the
/// write functions take the type of the *new* entry to create, and
/// [`StatKind`] only names the three kinds a caller may ask to create
/// (`Empty` is never a target).
fn raw_kind(kind: StatKind) -> ffi::clingo_statistics_type_t {
    super::c_int_of(match kind {
        StatKind::Value => ffi::clingo_statistics_type_value,
        StatKind::Array => ffi::clingo_statistics_type_array,
        StatKind::Map => ffi::clingo_statistics_type_map,
    })
}

/// A statistics object reached from a solve event's statistics event: unlike
/// [`Stats`], which [`ControlHandle::statistics`] builds on the `const *`
/// `clingo_control_statistics` returns, this one wraps a genuinely non-const
/// `clingo_statistics_t *` (H:2159, 2215, 2239), because the statistics event
/// hands the callback the per-step and accumulated trees clingo is still
/// building, which only the write functions can reach.
///
/// The read side (`value`, `keys`) reuses [`Stats`] itself, through
/// [`MutableStats::as_const`]: C narrows a non-const pointer to `const`
/// implicitly, so the read-only functions clingo exposes work unchanged on this
/// object's own pointer, and there is no reason to duplicate their logic here.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MutableStats<'a> {
    object: *mut ffi::clingo_statistics_t,
    root: u64,
    _marker: PhantomData<&'a mut ffi::clingo_statistics_t>,
}

impl<'a> MutableStats<'a> {
    /// Wraps a non-const statistics object delivered by a solve event's
    /// statistics event, reading its root key.
    ///
    /// # Safety
    ///
    /// `object` must be a live, non-const `clingo_statistics_t *`, valid for
    /// `'a` (the duration of the statistics event that delivered it,
    /// `control.cc:2009-2010`).
    pub(crate) unsafe fn new(object: *mut ffi::clingo_statistics_t) -> Result<Self, Error> {
        let mut root = 0;
        // SAFETY: `object` is live for `'a` (the caller's contract above); C
        // narrows the non-const pointer to `const` implicitly, exactly as
        // `Stats`'s own construction does with the pointer
        // `clingo_control_statistics` returns. `root` is a valid out-pointer
        // (clingo.h:2119).
        query(|| unsafe { ffi::clingo_statistics_root(object.cast_const(), &raw mut root) })?;
        Ok(MutableStats {
            object,
            root,
            _marker: PhantomData,
        })
    }

    /// Builds one directly from its parts, skipping the real, FFI-touching
    /// root lookup [`MutableStats::new`] does. Only for the `raw::events`
    /// unit tests, which pin the statistics event's dispatch and outer
    /// return-value/`goon` mapping without a live clingo statistics object
    /// (Miri cannot call `clingo_statistics_root`); those tests never read
    /// through the resulting object, only that the handler was called and
    /// what it returned.
    #[cfg(test)]
    pub(crate) fn for_test(object: *mut ffi::clingo_statistics_t, root: u64) -> Self {
        MutableStats {
            object,
            root,
            _marker: PhantomData,
        }
    }

    /// A read-only view of the same object and root, for the methods
    /// [`Stats`] already implements.
    fn as_const(self) -> Stats<'a> {
        Stats {
            object: self.object.cast_const(),
            root: self.root,
            _control: PhantomData,
        }
    }

    /// The root entry, for a read cursor lent for as long as `self` is.
    pub(crate) fn root_key(self) -> StatsKey<'a> {
        self.as_const().root_key()
    }

    /// The number at `path`, as [`Stats::value`].
    pub(crate) fn value(self, path: &str) -> Result<f64, Error> {
        self.as_const().value(path)
    }

    /// The keys at `path`, as [`Stats::keys`].
    pub(crate) fn keys(self, path: &str) -> Result<Vec<String>, Error> {
        self.as_const().keys(path)
    }

    /// Sets the number at the value entry `path` (clingo.h:2239). The
    /// resolved entry's own type is checked first, exactly as the read side
    /// already checks it before every read (module doc): a mismatch is a
    /// runtime error here rather than the logic error clingo itself would
    /// raise, which would poison unconditionally (DESIGN S3).
    pub(crate) fn set_value(self, path: &str, value: f64) -> Result<(), Error> {
        let entry = self.root_key().lookup(path)?;
        match entry.kind()? {
            StatsKind::Value => {
                // SAFETY: `self.object` is live and non-const for `'a`
                // (the constructor's contract); `entry.key` was resolved
                // against it and just checked to be a value entry (clingo.h:2239).
                call(|| unsafe { ffi::clingo_statistics_value_set(self.object, entry.key, value) })
            }
            other => Err(Error::new(
                ErrorKind::Runtime,
                format!("the entry is {}, not a value", kind_name(other)),
            )),
        }
    }

    /// Creates the subkey at the end of the array entry `path`, of the given
    /// kind, and returns its index (clingo.h:2159): the array's own size
    /// just before the push, which the C API itself also reports through
    /// `subkey` but as an opaque key rather than an index, so it is computed
    /// here instead of decoded from clingo's value.
    pub(crate) fn push_array(self, path: &str, kind: StatKind) -> Result<usize, Error> {
        let entry = self.root_key().lookup(path)?;
        match entry.kind()? {
            StatsKind::Array => {
                let index = entry.array_size()?;
                let mut subkey = 0;
                // SAFETY: as in `set_value`; `entry` is checked as an array
                // above, and `kind` is one of clingo's own type constants
                // (clingo.h:2159). `subkey` is a valid out-pointer.
                call(|| unsafe {
                    ffi::clingo_statistics_array_push(
                        self.object,
                        entry.key,
                        raw_kind(kind),
                        &raw mut subkey,
                    )
                })?;
                Ok(index)
            }
            other => Err(Error::new(
                ErrorKind::Runtime,
                format!("the entry is {}, not an array", kind_name(other)),
            )),
        }
    }

    /// Adds a subkey named `name` to the map entry `path`, of the given kind
    /// (clingo.h:2215).
    ///
    /// `name` is rejected up front, before clingo ever sees it, if it is empty
    /// or contains `.`, [`Statistics`](crate::Statistics)'s own dot-separated
    /// path syntax: clingo happily creates such a subkey, but no path through
    /// `value`/`keys`/`snapshot` can ever address it again afterwards, so
    /// building one silently makes part of the tree unreachable instead of
    /// failing where the mistake was made.
    pub(crate) fn add_map_key(self, path: &str, name: &str, kind: StatKind) -> Result<(), Error> {
        let entry = self.root_key().lookup(path)?;
        match entry.kind()? {
            StatsKind::Map => {
                if name.is_empty() || name.contains('.') {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "{name:?} is empty or contains `.`, the path separator, so no \
                             lookup could ever address it again"
                        ),
                    ));
                }
                let name = CString::new(name).map_err(|e| {
                    Error::new(
                        ErrorKind::Nul,
                        format!(
                            "{name:?} contains a NUL byte at position {}",
                            e.nul_position()
                        ),
                    )
                })?;
                let mut subkey = 0;
                // SAFETY: as in `set_value`; `entry` is checked as a map
                // above, `name` is NUL-terminated and outlives the call, and
                // `kind` is one of clingo's own type constants
                // (clingo.h:2215). `subkey` is a valid out-pointer.
                call(|| unsafe {
                    ffi::clingo_statistics_map_add_subkey(
                        self.object,
                        entry.key,
                        name.as_ptr(),
                        raw_kind(kind),
                        &raw mut subkey,
                    )
                })
            }
            other => Err(Error::new(
                ErrorKind::Runtime,
                format!("the entry is {}, not a map", kind_name(other)),
            )),
        }
    }
}

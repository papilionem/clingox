//! Wrappers for the statistics (clingo.h:2100-2239, 3205).
//!
//! clingo reports an unknown key and reading a map as a value as logic errors
//! (`StatsMap::at`, `type error`), which would poison the control (DESIGN S3).
//! So every path is walked here one level at a time, checking each key and
//! the type of each entry before clingo is asked, and a bad path is a runtime
//! error.

use std::ffi::{CString, c_char};
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

impl Stats<'_> {
    /// The type of the entry at `key` (clingo.h:2126).
    fn kind(self, key: u64) -> Result<StatsKind, Error> {
        let mut kind = 0;
        // SAFETY: `self.object` is valid for `'c` (see `Stats`), `key` was
        // returned by clingo for it, and `kind` is a valid out-pointer
        // (clingo.h:2126).
        query(|| unsafe { ffi::clingo_statistics_type(self.object, key, &raw mut kind) })?;
        Ok(match u32::try_from(kind) {
            Ok(ffi::clingo_statistics_type_value) => StatsKind::Value,
            Ok(ffi::clingo_statistics_type_array) => StatsKind::Array,
            Ok(ffi::clingo_statistics_type_map) => StatsKind::Map,
            _ => StatsKind::Empty,
        })
    }

    /// The number of elements of the array at `key` (clingo.h:2139).
    fn array_size(self, key: u64) -> Result<usize, Error> {
        let mut size = 0;
        // SAFETY: as in `kind`; the caller checked that `key` is an array, as
        // clingo.h requires (clingo.h:2139).
        query(|| unsafe { ffi::clingo_statistics_array_size(self.object, key, &raw mut size) })?;
        Ok(size)
    }

    /// The key of element `index` of the array at `key` (clingo.h:2149).
    fn array_at(self, key: u64, index: usize) -> Result<u64, Error> {
        let mut element = 0;
        // SAFETY: as in `kind`; the caller checked that `key` is an array with
        // more than `index` elements (clingo.h:2149).
        query(|| unsafe {
            ffi::clingo_statistics_array_at(self.object, key, index, &raw mut element)
        })?;
        Ok(element)
    }

    /// The number of entries of the map at `key` (clingo.h:2173).
    fn map_size(self, key: u64) -> Result<usize, Error> {
        let mut size = 0;
        // SAFETY: as in `kind`; the caller checked that `key` is a map
        // (clingo.h:2173).
        query(|| unsafe { ffi::clingo_statistics_map_size(self.object, key, &raw mut size) })?;
        Ok(size)
    }

    /// The name of entry `index` of the map at `key`, copied (clingo.h:2193).
    fn map_name(self, key: u64, index: usize) -> Result<String, Error> {
        let mut name: *const c_char = std::ptr::null();
        // SAFETY: as in `kind`; the caller checked that `key` is a map with
        // more than `index` entries (clingo.h:2193).
        query(|| unsafe {
            ffi::clingo_statistics_map_subkey_name(self.object, key, index, &raw mut name)
        })?;
        // SAFETY: clingo returns a NUL-terminated name owned by the statistics,
        // which do not change while the control is borrowed; it is copied at
        // once.
        unsafe { borrowed_str(name) }.map(str::to_owned)
    }

    /// The key of the entry `name` of the map at `key`, or `None` if there is
    /// no such entry (clingo.h:2183, 2204).
    fn map_at(self, key: u64, name: &str) -> Result<Option<u64>, Error> {
        // A name with a NUL byte cannot be a key.
        if name.contains('\0') {
            return Ok(None);
        }
        with_c_str(name, |name| {
            let mut present = false;
            // SAFETY: as in `kind`; the caller checked that `key` is a map,
            // `name` is NUL-terminated and outlives the call, and `present` is
            // a valid out-pointer (clingo.h:2183).
            query(|| unsafe {
                ffi::clingo_statistics_map_has_subkey(
                    self.object,
                    key,
                    name.as_ptr(),
                    &raw mut present,
                )
            })?;
            if !present {
                return Ok(None);
            }
            let mut entry = 0;
            // SAFETY: as above; the entry exists, so clingo raises no logic
            // error (clingo.h:2204).
            query(|| unsafe {
                ffi::clingo_statistics_map_at(self.object, key, name.as_ptr(), &raw mut entry)
            })?;
            Ok(Some(entry))
        })
    }

    /// The number at the value entry `key` (clingo.h:2230).
    fn value_at(self, key: u64) -> Result<f64, Error> {
        let mut value = 0.0;
        // SAFETY: as in `kind`; the caller checked that `key` is a value
        // (clingo.h:2230).
        query(|| unsafe { ffi::clingo_statistics_value_get(self.object, key, &raw mut value) })?;
        Ok(value)
    }

    /// The key of the entry at `path`, walked one checked level at a time.
    /// The empty path is the root.
    fn resolve(self, path: &str) -> Result<u64, Error> {
        let mut key = self.root;
        if path.is_empty() {
            return Ok(key);
        }
        for part in path.split('.') {
            key = match self.kind(key)? {
                StatsKind::Map => self.map_at(key, part)?,
                StatsKind::Array => match part.parse::<usize>() {
                    Ok(index) if index < self.array_size(key)? => Some(self.array_at(key, index)?),
                    _ => None,
                },
                StatsKind::Value | StatsKind::Empty => None,
            }
            .ok_or_else(|| Error::new(ErrorKind::Runtime, format!("no entry `{part}`")))?;
        }
        Ok(key)
    }

    /// The number at `path`.
    pub(crate) fn value(self, path: &str) -> Result<f64, Error> {
        let key = self.resolve(path)?;
        match self.kind(key)? {
            StatsKind::Value => self.value_at(key),
            kind => Err(Error::new(
                ErrorKind::Runtime,
                format!("the entry is {}, not a value", kind_name(kind)),
            )),
        }
    }

    /// The names of the map at `path`, or the indices of the array there as
    /// text; none for a value.
    pub(crate) fn keys(self, path: &str) -> Result<Vec<String>, Error> {
        let key = self.resolve(path)?;
        match self.kind(key)? {
            StatsKind::Map => (0..self.map_size(key)?)
                .map(|index| self.map_name(key, index))
                .collect(),
            StatsKind::Array => Ok((0..self.array_size(key)?).map(|i| i.to_string()).collect()),
            StatsKind::Value | StatsKind::Empty => Ok(Vec::new()),
        }
    }

    /// A copy of the whole tree, in clingo's order.
    pub(crate) fn snapshot(self) -> Result<StatsTree, Error> {
        self.copy(self.root)
    }

    /// A copy of the tree below `key`. An entry of clingo's type `empty` is
    /// copied as an empty map.
    fn copy(self, key: u64) -> Result<StatsTree, Error> {
        Ok(match self.kind(key)? {
            StatsKind::Value => StatsTree::Value(self.value_at(key)?),
            StatsKind::Array => StatsTree::Array(
                (0..self.array_size(key)?)
                    .map(|index| self.copy(self.array_at(key, index)?))
                    .collect::<Result<_, Error>>()?,
            ),
            StatsKind::Map => StatsTree::Map(
                (0..self.map_size(key)?)
                    .map(|index| {
                        let name = self.map_name(key, index)?;
                        let entry = self.map_at(key, &name)?.ok_or_else(|| {
                            Error::new(
                                ErrorKind::Unknown,
                                format!("clingo lists `{name}` but has no entry for it"),
                            )
                        })?;
                        Ok((name, self.copy(entry)?))
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
        let key = self.as_const().resolve(path)?;
        match self.as_const().kind(key)? {
            StatsKind::Value => {
                // SAFETY: `self.object` is live and non-const for `'a`
                // (the constructor's contract); `key` was resolved against
                // it and just checked to be a value entry (clingo.h:2239).
                call(|| unsafe { ffi::clingo_statistics_value_set(self.object, key, value) })
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
        let key = self.as_const().resolve(path)?;
        match self.as_const().kind(key)? {
            StatsKind::Array => {
                let index = self.as_const().array_size(key)?;
                let mut subkey = 0;
                // SAFETY: as in `set_value`; `key` is checked as an array
                // above, and `kind` is one of clingo's own type constants
                // (clingo.h:2159). `subkey` is a valid out-pointer.
                call(|| unsafe {
                    ffi::clingo_statistics_array_push(
                        self.object,
                        key,
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
        let key = self.as_const().resolve(path)?;
        match self.as_const().kind(key)? {
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
                // SAFETY: as in `set_value`; `key` is checked as a map
                // above, `name` is NUL-terminated and outlives the call, and
                // `kind` is one of clingo's own type constants
                // (clingo.h:2215). `subkey` is a valid out-pointer.
                call(|| unsafe {
                    ffi::clingo_statistics_map_add_subkey(
                        self.object,
                        key,
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

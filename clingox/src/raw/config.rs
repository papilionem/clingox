//! Wrappers for the configuration (clingo.h:1900-2053, 3236).
//!
//! A path is clingo's own key syntax, which clasp resolves in one
//! `clingo_configuration_map_at` call: names separated by dots, where a number
//! selects an array element (`solver.0.seed`), and a name on an array is
//! looked up in its first element (`solver.seed`).

use std::ffi::{CStr, CString, c_char};
use std::marker::PhantomData;

use clingox_sys as ffi;

use super::control::ControlHandle;
use super::{borrowed_str, c_str, call, fill_string, query, with_c_str};
use crate::error::{Error, ErrorKind};

/// An entry of the configuration: the object and the entry's key.
///
/// The object belongs to the control, and `'a` is the borrow of the control
/// it was made under. While that borrow lives nothing can ground, solve or
/// write the configuration, so a key stays what clingo returned it for. Only
/// this module builds one, and only from a value clingo returned for this
/// object (the root, `map_at`, `array_at`), so safe code can neither invent a
/// key nor pair it with another control's object (DESIGN S5). The
/// raw pointer keeps it `!Send` and `!Sync`.
#[derive(Clone, Copy)]
pub(crate) struct ConfigKey<'a> {
    config: *mut ffi::clingo_configuration_t,
    key: ffi::clingo_id_t,
    _control: PhantomData<&'a ControlHandle>,
}

impl<'a> ConfigKey<'a> {
    /// The entry's type bits: value, array, map (clingo.h:1900-1912, 1931).
    pub(crate) fn kind(self) -> Result<u32, Error> {
        let mut kind = 0;
        // SAFETY: `config` and `key` come from `ControlHandle::config_root` or
        // from clingo for this object, within the current borrow of the
        // control, and `kind` is a valid out-pointer (clingo.h:1931).
        query(|| unsafe { ffi::clingo_configuration_type(self.config, self.key, &raw mut kind) })?;
        Ok(kind)
    }

    /// The size of an array entry, after the type check clingo.h requires
    /// (clingo.h:1952). Anything else is `InvalidInput`.
    pub(crate) fn array_size(self) -> Result<usize, Error> {
        if self.kind()? & ffi::clingo_configuration_type_array == 0 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the entry is not an array",
            ));
        }
        self.array_size_of_array()
    }

    /// The size of an entry the caller already knows to be an array, so that
    /// a walk reads the type once.
    pub(crate) fn array_size_of_array(self) -> Result<usize, Error> {
        let mut size = 0;
        // SAFETY: the caller checked that the entry is an array, and `size`
        // is a valid out-pointer (clingo.h:1952).
        query(|| unsafe {
            ffi::clingo_configuration_array_size(self.config, self.key, &raw mut size)
        })?;
        Ok(size)
    }

    fn is_value(self) -> Result<bool, Error> {
        Ok(self.kind()? & ffi::clingo_configuration_type_value != 0)
    }

    /// Sets a value entry from text (clingo.h:2053).
    fn set(self, value: &CStr) -> Result<(), Error> {
        // SAFETY: the caller checked that the entry holds a value, as clingo.h
        // requires, and `value` is NUL-terminated and outlives the call; clingo
        // parses it (clingo.h:2049-2053).
        call(|| unsafe {
            ffi::clingo_configuration_value_set(self.config, self.key, value.as_ptr())
        })
    }

    /// The entry at `path` below this one; the empty path is this entry
    /// (clingo.h:2010). An unknown path is a runtime error ("invalid key").
    pub(crate) fn lookup(self, path: &CStr) -> Result<ConfigKey<'a>, Error> {
        if path.is_empty() {
            return Ok(self);
        }
        let mut key = 0;
        // SAFETY: `config` is valid as above, `self.key` is one of its keys,
        // `path` is NUL-terminated and outlives the call, and `key` is a valid
        // out-pointer. An unknown path, and a step below a value, is a runtime
        // error, not undefined behaviour (clingocontrol.cc:396-402).
        query(|| unsafe {
            ffi::clingo_configuration_map_at(self.config, self.key, path.as_ptr(), &raw mut key)
        })?;
        Ok(ConfigKey { key, ..self })
    }

    /// The value as clingo prints it, or `None` for a map or array, or for an
    /// option clingo leaves unassigned (clingo.h:2024, 2033, 2044).
    pub(crate) fn value(self) -> Result<Option<String>, Error> {
        if !self.is_value()? {
            return Ok(None);
        }
        let mut assigned = false;
        // SAFETY: the entry holds a value (checked above), and `assigned` is a
        // valid out-pointer (clingo.h:2024).
        query(|| unsafe {
            ffi::clingo_configuration_value_is_assigned(self.config, self.key, &raw mut assigned)
        })?;
        if !assigned {
            return Ok(None);
        }
        fill_string(
            |size| {
                // SAFETY: the entry holds a value, and `size` is a valid
                // out-pointer (clingo.h:2033).
                query(|| unsafe {
                    ffi::clingo_configuration_value_get_size(self.config, self.key, size)
                })
            },
            |buffer, size| {
                // SAFETY: as above; `buffer` has room for `size` characters, the
                // size clingo just reported, NUL included (control.cc:1067-1087).
                query(|| unsafe {
                    ffi::clingo_configuration_value_get(self.config, self.key, buffer, size)
                })
            },
        )
        .map(Some)
    }

    /// The number of names under a map entry the caller already knows to be a
    /// map (clingo.h:1978).
    pub(crate) fn map_size(self) -> Result<usize, Error> {
        let mut size = 0;
        // SAFETY: the caller checked that the entry is a map, and `size` is a
        // valid out-pointer (clingo.h:1978).
        query(|| unsafe {
            ffi::clingo_configuration_map_size(self.config, self.key, &raw mut size)
        })?;
        Ok(size)
    }

    /// The name at `index` of a map entry with more than `index` names,
    /// copied (clingo.h:1999).
    fn name_at(self, index: usize) -> Result<String, Error> {
        let mut name: *const c_char = std::ptr::null();
        // SAFETY: the caller checked that the entry is a map with more than
        // `index` names, and `name` is a valid out-pointer (clingo.h:1999).
        query(|| unsafe {
            ffi::clingo_configuration_map_subkey_name(self.config, self.key, index, &raw mut name)
        })?;
        // SAFETY: clingo returns a NUL-terminated name owned by the
        // configuration, which does not change while the control is borrowed;
        // it is copied at once.
        unsafe { borrowed_str(name) }.map(str::to_owned)
    }

    /// Child `index` of a map entry with more than `index` names, with its
    /// name.
    ///
    /// A name with a `.` would be read as a path by `map_at`. None occurs in
    /// clingo 5.8.2's configuration (274 names checked, with one and three
    /// solvers), so this check is one nothing
    /// reaches; it keeps a name from another version from turning into a
    /// different lookup.
    pub(crate) fn map_entry(self, index: usize) -> Result<(String, ConfigKey<'a>), Error> {
        let name = self.name_at(index)?;
        if name.contains('.') {
            return Err(Error::new(
                ErrorKind::Runtime,
                format!("clingo lists the name `{name}`, which contains the path separator"),
            ));
        }
        let child = with_c_str(&name, |path| self.lookup(path))?;
        Ok((name, child))
    }

    /// The names under the map, in clingo's order; none for a value or an
    /// array (clingo.h:1978, 1999).
    pub(crate) fn keys(self) -> Result<Vec<String>, Error> {
        if self.kind()? & ffi::clingo_configuration_type_map == 0 {
            return Ok(Vec::new());
        }
        (0..self.map_size()?)
            .map(|index| self.name_at(index))
            .collect()
    }

    /// The help text, copied at once (clingo.h:1939).
    pub(crate) fn description(self) -> Result<String, Error> {
        let mut text: *const c_char = std::ptr::null();
        // SAFETY: the entry is a key of the current configuration, and `text`
        // is a valid out-pointer (clingo.h:1939).
        query(|| unsafe {
            ffi::clingo_configuration_description(self.config, self.key, &raw mut text)
        })?;
        // SAFETY: clingo returns a NUL-terminated text owned by the
        // configuration, which does not change while the control is borrowed;
        // it is copied at once.
        unsafe { borrowed_str(text) }.map(str::to_owned)
    }

    /// Element `index` of an array entry.
    ///
    /// clingo does not check the offset (`array_at` answers for `size` and
    /// `size + 1`, and truncates a huge offset), so `index < size` is checked
    /// here, strictly (clingo.h:1964).
    pub(crate) fn element(self, index: usize) -> Result<ConfigKey<'a>, Error> {
        let size = self.array_size()?;
        self.element_below(index, size)
    }

    /// Element `index` of an array entry of `size` elements, which the caller
    /// read from [`ConfigKey::array_size`] or [`ConfigKey::array_size_of_array`].
    pub(crate) fn element_below(self, index: usize, size: usize) -> Result<ConfigKey<'a>, Error> {
        if index >= size {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("index {index} is out of range for an array of {size} elements"),
            ));
        }
        let mut key = 0;
        // SAFETY: the entry is an array of `size` elements (the caller's
        // contract), `index < size`, and `key` is a valid out-pointer
        // (clingo.h:1964).
        query(|| unsafe {
            ffi::clingo_configuration_array_at(self.config, self.key, index, &raw mut key)
        })?;
        Ok(ConfigKey { key, ..self })
    }

    /// Whether the map entry has the sub-entry `name` (clingo.h:1989).
    pub(crate) fn has_key(self, name: &CStr) -> Result<bool, Error> {
        if self.kind()? & ffi::clingo_configuration_type_map == 0 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the entry is not a map",
            ));
        }
        let mut result = false;
        // SAFETY: the entry is a map (checked above), `name` is NUL-terminated
        // and outlives the call, and `result` is a valid out-pointer
        // (clingo.h:1989).
        query(|| unsafe {
            ffi::clingo_configuration_map_has_subkey(
                self.config,
                self.key,
                name.as_ptr(),
                &raw mut result,
            )
        })?;
        Ok(result)
    }
}

impl ControlHandle {
    /// The root entry (clingo.h:3236, 1922).
    pub(crate) fn config_root(&self) -> Result<ConfigKey<'_>, Error> {
        let mut config: *mut ffi::clingo_configuration_t = std::ptr::null_mut();
        let mut key = 0;
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and
            // `config` a valid out-pointer. The configuration belongs to the
            // control and is used only within the current borrow of it; no
            // search is open (`logged` closed it), which clingo.h requires
            // (clingo.h:3236).
            unsafe { ffi::clingo_control_configuration(control, &raw mut config) }
        })?;
        // SAFETY: `config` was just returned by clingo, and `key` is a valid
        // out-pointer (clingo.h:1922).
        query(|| unsafe { ffi::clingo_configuration_root(config, &raw mut key) })?;
        Ok(ConfigKey {
            config,
            key,
            _control: PhantomData,
        })
    }

    /// The entry at `path`; the empty path is the root. An unknown path is a
    /// runtime error ("invalid key").
    pub(crate) fn config_entry(&self, path: &CStr) -> Result<ConfigKey<'_>, Error> {
        self.config_root()?.lookup(path)
    }

    /// The value at `path` as clingo prints it, or `None` for a map or array,
    /// or for an option clingo leaves unassigned.
    pub(crate) fn config_get(&self, path: &CStr) -> Result<Option<String>, Error> {
        self.config_entry(path)?.value()
    }

    /// Sets the value at `path` (clingo.h:2053). It takes effect from the next
    /// solve call. A value the option rejects is a runtime error ("could not
    /// set option value"), and an option that had a value keeps it.
    ///
    /// clasp does not guarantee that by itself: it parses some numbers in
    /// place (`parseSigned` writes `strtoll`'s result before checking it,
    /// libpotassco/src/string_convert.cpp:109-131, 220-223), and where
    /// `int64_t` is `long long`, as on wasm32, a rejected `solve.models` is
    /// left at 0. So the previous value is put back after a failure. An
    /// option without a value cannot be restored: the configuration API has
    /// no way to unassign one, and the first set of a `tester` option assigns
    /// every option of the tester configuration (`ClaspCliConfig::setValue`
    /// creates it before parsing the value, clasp_options.cpp:984-992).
    pub(crate) fn config_set(&mut self, path: &CStr, value: &CStr) -> Result<(), Error> {
        let entry = self.config_entry(path)?;
        // clingo.h requires a value entry (clingo.h:2049).
        if !entry.is_value()? {
            return Err(Error::new(
                ErrorKind::Runtime,
                "a map or array takes no value",
            ));
        }
        let previous = self.config_get(path)?;
        let result = entry.set(value);
        if result.is_err()
            && let Some(previous) = previous.and_then(|p| CString::new(p).ok())
        {
            // The value clingo printed is one it accepts; if putting it back
            // fails all the same, the first error is the one to report.
            drop(entry.set(&previous));
        }
        result
    }

    /// The names under the map at `path`, in clingo's order; none for a value
    /// or an array (clingo.h:1978, 1999).
    pub(crate) fn config_keys(&self, path: &CStr) -> Result<Vec<String>, Error> {
        self.config_entry(path)?.keys()
    }

    /// The help text of the entry at `path`, copied at once (clingo.h:1939).
    pub(crate) fn config_description(&self, path: &CStr) -> Result<String, Error> {
        self.config_entry(path)?.description()
    }

    /// The type bits of the entry at `path` (clingo.h:1931).
    pub(crate) fn config_kind(&self, path: &CStr) -> Result<u32, Error> {
        self.config_entry(path)?.kind()
    }

    /// The size of the array entry at `path` (clingo.h:1952).
    pub(crate) fn config_len(&self, path: &CStr) -> Result<usize, Error> {
        self.config_entry(path)?.array_size()
    }

    /// The path of element `index` of the array entry at `path`.
    ///
    /// clingo does not check the offset, so `index < size` is checked in
    /// [`ConfigKey::element`], strictly.
    pub(crate) fn config_element(&self, path: &CStr, index: usize) -> Result<String, Error> {
        let subkey = self.config_entry(path)?.element(index)?;
        // clingo hands out a key, not a name. The element's path is the
        // array's path and its number, and it must lead to the same key when
        // clingo resolves it from the root; a base clingo reads differently
        // (`solver..`) is refused here rather than returned as a broken path.
        let element = format!("{}.{index}", path.to_string_lossy());
        let unresolved = |found: String| {
            Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "`{element}` does not lead to element {index} of the array ({found}, not key {})",
                    subkey.key
                ),
            )
        };
        // An unknown path is a runtime error of clingo's, a plain "no".
        let resolved = match self.config_entry(&c_str(&element)?) {
            Ok(resolved) => resolved,
            Err(err) if err.kind() == ErrorKind::Runtime => {
                return Err(unresolved("it does not resolve".to_owned()));
            }
            Err(err) => return Err(err),
        };
        if resolved.key != subkey.key {
            return Err(unresolved(format!("key {}", resolved.key)));
        }
        Ok(element)
    }

    /// Whether the map entry at `path` has the sub-entry `name` (clingo.h:1989).
    pub(crate) fn config_has_key(&self, path: &CStr, name: &CStr) -> Result<bool, Error> {
        self.config_entry(path)?.has_key(name)
    }
}

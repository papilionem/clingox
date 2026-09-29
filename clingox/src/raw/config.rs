//! Wrappers for the configuration (clingo.h:1900-2053, 3236).
//!
//! A path is clingo's own key syntax, which clasp resolves in one
//! `clingo_configuration_map_at` call: names separated by dots, where a number
//! selects an array element (`solver.0.seed`), and a name on an array is
//! looked up in its first element (`solver.seed`).

use std::ffi::{CStr, CString, c_char};

use clingox_sys as ffi;

use super::control::ControlHandle;
use super::{borrowed_str, c_str, call, fill_string, query};
use crate::error::{Error, ErrorKind};

/// An entry of the configuration: the object and the entry's key. The object
/// belongs to the control and is used only while the control is borrowed.
#[derive(Clone, Copy)]
struct Entry {
    config: *mut ffi::clingo_configuration_t,
    key: ffi::clingo_id_t,
}

impl Entry {
    /// The entry's type bits: value, array, map (clingo.h:1900-1912, 1930).
    fn kind(self) -> Result<u32, Error> {
        let mut kind = 0;
        // SAFETY: `config` and `key` come from `ControlHandle::entry` within the
        // current borrow of the control, and `kind` is a valid out-pointer
        // (clingo.h:1930).
        query(|| unsafe { ffi::clingo_configuration_type(self.config, self.key, &raw mut kind) })?;
        Ok(kind)
    }

    /// The size of an array entry, after the type check clingo.h requires
    /// (clingo.h:2320). Anything else is `InvalidInput`.
    fn array_size(self) -> Result<usize, Error> {
        if self.kind()? & ffi::clingo_configuration_type_array == 0 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the entry is not an array",
            ));
        }
        let mut size = 0;
        // SAFETY: the entry is an array (checked above), and `size` is a valid
        // out-pointer (clingo.h:2320).
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
}

impl ControlHandle {
    /// The entry at `path`; the empty path is the root (clingo.h:3236, 1922,
    /// 2010). An unknown path is a runtime error ("invalid key").
    fn entry(&self, path: &CStr) -> Result<Entry, Error> {
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
        if !path.is_empty() {
            let root = key;
            // SAFETY: `config` is valid as above, `root` is its root key,
            // `path` is NUL-terminated and outlives the call, and `key` is a
            // valid out-pointer. An unknown path is a runtime error, not
            // undefined behaviour (clingocontrol.cc:396-402).
            query(|| unsafe {
                ffi::clingo_configuration_map_at(config, root, path.as_ptr(), &raw mut key)
            })?;
        }
        Ok(Entry { config, key })
    }

    /// The value at `path` as clingo prints it, or `None` for a map or array,
    /// or for an option clingo leaves unassigned (clingo.h:2020, 2033, 2044).
    pub(crate) fn config_get(&self, path: &CStr) -> Result<Option<String>, Error> {
        let entry = self.entry(path)?;
        if !entry.is_value()? {
            return Ok(None);
        }
        let mut assigned = false;
        // SAFETY: `entry` holds a value (checked above), and `assigned` is a
        // valid out-pointer (clingo.h:2020).
        query(|| unsafe {
            ffi::clingo_configuration_value_is_assigned(entry.config, entry.key, &raw mut assigned)
        })?;
        if !assigned {
            return Ok(None);
        }
        fill_string(
            |size| {
                // SAFETY: `entry` holds a value, and `size` is a valid
                // out-pointer (clingo.h:2033).
                query(|| unsafe {
                    ffi::clingo_configuration_value_get_size(entry.config, entry.key, size)
                })
            },
            |buffer, size| {
                // SAFETY: as above; `buffer` has room for `size` characters, the
                // size clingo just reported, NUL included (control.cc:1067-1087).
                query(|| unsafe {
                    ffi::clingo_configuration_value_get(entry.config, entry.key, buffer, size)
                })
            },
        )
        .map(Some)
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
        let entry = self.entry(path)?;
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
    /// or an array (clingo.h:1980, 2001).
    pub(crate) fn config_keys(&self, path: &CStr) -> Result<Vec<String>, Error> {
        let entry = self.entry(path)?;
        if entry.kind()? & ffi::clingo_configuration_type_map == 0 {
            return Ok(Vec::new());
        }
        let mut size = 0;
        // SAFETY: `entry` is a map (checked above), and `size` is a valid
        // out-pointer (clingo.h:1980).
        query(|| unsafe {
            ffi::clingo_configuration_map_size(entry.config, entry.key, &raw mut size)
        })?;
        (0..size)
            .map(|index| {
                let mut name: *const c_char = std::ptr::null();
                // SAFETY: `entry` is a map with `size` entries and
                // `index < size`; `name` is a valid out-pointer (clingo.h:2001).
                query(|| unsafe {
                    ffi::clingo_configuration_map_subkey_name(
                        entry.config,
                        entry.key,
                        index,
                        &raw mut name,
                    )
                })?;
                // SAFETY: clingo returns a NUL-terminated name owned by the
                // configuration, which does not change while the control is
                // borrowed; it is copied at once.
                unsafe { borrowed_str(name) }.map(str::to_owned)
            })
            .collect()
    }

    /// The help text of the entry at `path`, copied at once (clingo.h:2306).
    pub(crate) fn config_description(&self, path: &CStr) -> Result<String, Error> {
        let entry = self.entry(path)?;
        let mut text: *const c_char = std::ptr::null();
        // SAFETY: `entry` is a key of the current configuration, and `text` is
        // a valid out-pointer (clingo.h:2306).
        query(|| unsafe {
            ffi::clingo_configuration_description(entry.config, entry.key, &raw mut text)
        })?;
        // SAFETY: clingo returns a NUL-terminated text owned by the
        // configuration, which does not change while the control is borrowed;
        // it is copied at once.
        unsafe { borrowed_str(text) }.map(str::to_owned)
    }

    /// The type bits of the entry at `path` (clingo.h:1930).
    pub(crate) fn config_kind(&self, path: &CStr) -> Result<u32, Error> {
        self.entry(path)?.kind()
    }

    /// The size of the array entry at `path` (clingo.h:2320).
    pub(crate) fn config_len(&self, path: &CStr) -> Result<usize, Error> {
        let entry = self.entry(path)?;
        entry.array_size()
    }

    /// The path of element `index` of the array entry at `path`.
    ///
    /// clingo does not check the offset (`array_at` answers for `size` and
    /// `size + 1`, and truncates a huge offset), so `index < size` is checked
    /// here, strictly.
    pub(crate) fn config_element(&self, path: &CStr, index: usize) -> Result<String, Error> {
        let entry = self.entry(path)?;
        let size = entry.array_size()?;
        if index >= size {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("index {index} is out of range for an array of {size} elements"),
            ));
        }
        let mut subkey = 0;
        // SAFETY: `entry` is an array (checked by `array_size`), `index < size`,
        // and `subkey` is a valid out-pointer (clingo.h:2337).
        query(|| unsafe {
            ffi::clingo_configuration_array_at(entry.config, entry.key, index, &raw mut subkey)
        })?;
        // clingo hands out a key, not a name. The element's path is the
        // array's path and its number, and it must lead to the same key when
        // clingo resolves it from the root; a base clingo reads differently
        // (`solver..`) is refused here rather than returned as a broken path.
        let element = format!("{}.{index}", path.to_string_lossy());
        let unresolved = |found: String| {
            Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "`{element}` does not lead to element {index} of the array ({found}, not key {subkey})"
                ),
            )
        };
        // An unknown path is a runtime error of clingo's, a plain "no".
        let resolved = match self.entry(&c_str(&element)?) {
            Ok(resolved) => resolved,
            Err(err) if err.kind() == ErrorKind::Runtime => {
                return Err(unresolved("it does not resolve".to_owned()));
            }
            Err(err) => return Err(err),
        };
        if resolved.key != subkey {
            return Err(unresolved(format!("key {}", resolved.key)));
        }
        Ok(element)
    }

    /// Whether the map entry at `path` has the sub-entry `name` (clingo.h:2384).
    pub(crate) fn config_has_key(&self, path: &CStr, name: &CStr) -> Result<bool, Error> {
        let entry = self.entry(path)?;
        if entry.kind()? & ffi::clingo_configuration_type_map == 0 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the entry is not a map",
            ));
        }
        let mut result = false;
        // SAFETY: `entry` is a map (checked above), `name` is NUL-terminated and
        // outlives the call, and `result` is a valid out-pointer (clingo.h:2384).
        query(|| unsafe {
            ffi::clingo_configuration_map_has_subkey(
                entry.config,
                entry.key,
                name.as_ptr(),
                &raw mut result,
            )
        })?;
        Ok(result)
    }
}

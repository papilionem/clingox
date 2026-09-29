//! Wrappers for the backend: `clingo_control_backend`, `clingo_backend_begin`/
//! `_end`, and every `clingo_backend_*` directive (clingo.h:1611-2260,
//! 3355-3363).
//!
//! No callback of this file's own runs during any of these calls, so unlike
//! `raw::solve` there is no trampoline and no `catch_unwind` here. A registered
//! [`GroundProgramObserver`](crate::observer::GroundProgramObserver)'s own
//! trampoline (`raw::observer`) can still run as a side effect of
//! `clingo_backend_rule` and friends, and of `clingo_backend_end`'s delayed
//! output: its slots are read afterward by `Control::with_backend`, never
//! here.

use std::ffi::CStr;
use std::os::raw::c_int;
use std::ptr::NonNull;

use clingox_sys as ffi;

use super::control::ControlHandle;
use super::{RawSymbol, c_int_of, call};
use crate::backend::{ExternalKind, HeuristicKind, TheorySequenceKind};
use crate::error::{Error, ErrorKind};

/// A literal with a weight, as `clingo_backend_weight_rule` and
/// `clingo_backend_minimize` take them. The constructor keeps
/// `clingox_sys` unnamed outside `raw` (RULES 2): the typed layer builds
/// these through [`weighted_literal`] instead of naming the C struct itself.
pub(crate) type WeightedLiteral = ffi::clingo_weighted_literal_t;

/// Builds a [`WeightedLiteral`] from a literal and its weight.
pub(crate) fn weighted_literal(literal: i32, weight: i32) -> WeightedLiteral {
    ffi::clingo_weighted_literal_t { literal, weight }
}

/// clingo's `clingo_external_type_e` for an [`ExternalKind`]
/// (clingo.h:1613-1620).
fn raw_external_type(kind: ExternalKind) -> c_int {
    c_int_of(match kind {
        ExternalKind::Free => ffi::clingo_external_type_free,
        ExternalKind::True => ffi::clingo_external_type_true,
        ExternalKind::False => ffi::clingo_external_type_false,
        ExternalKind::Release => ffi::clingo_external_type_release,
    })
}

/// clingo's `clingo_heuristic_type_e` for a [`HeuristicKind`]
/// (clingo.h:1596-1603).
fn raw_heuristic_type(kind: HeuristicKind) -> c_int {
    c_int_of(match kind {
        HeuristicKind::Level => ffi::clingo_heuristic_type_level,
        HeuristicKind::Sign => ffi::clingo_heuristic_type_sign,
        HeuristicKind::Factor => ffi::clingo_heuristic_type_factor,
        HeuristicKind::Init => ffi::clingo_heuristic_type_init,
        HeuristicKind::True => ffi::clingo_heuristic_type_true,
        HeuristicKind::False => ffi::clingo_heuristic_type_false,
    })
}

/// clingo's `clingo_theory_sequence_type_e` for a [`TheorySequenceKind`]
/// (clingo.h:1583-1588).
fn raw_sequence_type(kind: TheorySequenceKind) -> c_int {
    c_int_of(match kind {
        TheorySequenceKind::Tuple => ffi::clingo_theory_sequence_type_tuple,
        TheorySequenceKind::Set => ffi::clingo_theory_sequence_type_set,
        TheorySequenceKind::List => ffi::clingo_theory_sequence_type_list,
    })
}

impl ControlHandle {
    /// Gets the control's backend and begins it (clingo.h:3355-3363,
    /// 1656-1662). Any leftover search or backend is closed first (S4).
    pub(crate) fn open_backend(&self) -> Result<(), Error> {
        // A leftover search, not the backend session this call opens:
        // discard its handler's own failure, never promote.
        self.close_solve(false)?;
        self.close_backend()?;
        let mut ptr: *mut ffi::clingo_backend_t = std::ptr::null_mut();
        self.captured(|| {
            // SAFETY: `self.ptr` is the live control this handle owns, and no
            // search or backend is open (both closed above). `ptr` is a valid
            // out-pointer (clingo.h:3355-3363).
            unsafe { ffi::clingo_control_backend(self.ptr.as_ptr(), &raw mut ptr) }
        })?;
        let ptr = NonNull::new(ptr).ok_or_else(|| {
            Error::new(
                ErrorKind::Unknown,
                "clingo reported success but returned no backend",
            )
        })?;
        self.captured(|| {
            // SAFETY: `ptr` is the backend clingo just returned for this
            // control, not begun yet, and is used on the thread that owns the
            // control (clingo.h:1656-1662).
            unsafe { ffi::clingo_backend_begin(ptr.as_ptr()) }
        })?;
        self.backend.set(Some(ptr));
        Ok(())
    }

    /// Closes the open backend, if any (clingo.h:1664-1669). A backend still
    /// open is closed at most once, whatever the outcome: the record is taken
    /// out before the call, and it is `None` afterward whether the call
    /// succeeds or fails.
    ///
    /// Like [`ControlHandle::close_solve`], this does not resume a pending
    /// logger panic itself, because it also runs from [`Drop`]: the caller
    /// resumes it once the backend's own close error, if any, has been
    /// reported.
    pub(crate) fn close_backend(&self) -> Result<(), Error> {
        let Some(ptr) = self.backend.take() else {
            return Ok(());
        };
        drop(self.capture.take());
        let result = call(|| {
            // SAFETY: `ptr` was begun by `open_backend` and is closed here
            // exactly once: it was taken out of the `Cell`, its only copy,
            // just above (clingo.h:1664-1669).
            unsafe { ffi::clingo_backend_end(ptr.as_ptr()) }
        });
        let messages = self.capture.take();
        result.map_err(|err| err.with_messages(messages))
    }

    /// The open backend's pointer, or a logic error if none is open. The safe
    /// layer never calls a backend method outside `with_backend`'s closure, so
    /// this should never fail in practice; it exists so a stray call cannot
    /// dereference a null or dangling pointer.
    fn backend_ptr(&self) -> Result<*mut ffi::clingo_backend_t, Error> {
        self.backend
            .get()
            .map(NonNull::as_ptr)
            .ok_or_else(|| Error::new(ErrorKind::Logic, "no backend is open on this control"))
    }

    /// Runs one call on the open backend, attaching the messages clingo
    /// logged during it.
    fn backend_call(
        &self,
        f: impl FnOnce(*mut ffi::clingo_backend_t) -> bool,
    ) -> Result<(), Error> {
        let ptr = self.backend_ptr()?;
        self.captured(|| f(ptr))
    }

    /// Gets a fresh atom, optionally associated with a symbol
    /// (clingo.h:2102-2115).
    pub(crate) fn backend_add_atom(&self, symbol: Option<RawSymbol>) -> Result<u32, Error> {
        let mut storage = symbol.unwrap_or_default();
        let symbol_ptr = if symbol.is_some() {
            &raw mut storage
        } else {
            std::ptr::null_mut()
        };
        let mut atom = 0;
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control. `symbol_ptr`
            // is either null or points to `storage`, valid for the call; when
            // null, clingo associates no symbol with the atom (clingo.h says
            // the parameter is optional). `atom` is a valid out-pointer
            // (clingo.h:2102-2115).
            unsafe { ffi::clingo_backend_add_atom(ptr, symbol_ptr, &raw mut atom) }
        })?;
        Ok(atom)
    }

    /// Adds a rule (clingo.h:1964-1985).
    pub(crate) fn backend_rule(
        &self,
        choice: bool,
        head: &[u32],
        body: &[i32],
    ) -> Result<(), Error> {
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control. `head` and
            // `body` outlive the call, which copies what it keeps
            // (clingo.h:1964-1985).
            unsafe {
                ffi::clingo_backend_rule(
                    ptr,
                    choice,
                    head.as_ptr(),
                    head.len(),
                    body.as_ptr(),
                    body.len(),
                )
            }
        })
    }

    /// Adds a weight rule (clingo.h:1986-2004). The caller has already
    /// checked the lower bound and every weight are positive:
    /// clingo's own check only covers a negative weight, not zero or a
    /// non-positive bound.
    pub(crate) fn backend_weight_rule(
        &self,
        choice: bool,
        head: &[u32],
        lower_bound: i32,
        body: &[WeightedLiteral],
    ) -> Result<(), Error> {
        self.backend_call(|ptr| {
            // SAFETY: as in `backend_rule` (clingo.h:1986-2004).
            unsafe {
                ffi::clingo_backend_weight_rule(
                    ptr,
                    choice,
                    head.as_ptr(),
                    head.len(),
                    lower_bound,
                    body.as_ptr(),
                    body.len(),
                )
            }
        })
    }

    /// Adds a minimize (or weak) constraint (clingo.h:2005-2019).
    pub(crate) fn backend_minimize(
        &self,
        priority: i32,
        literals: &[WeightedLiteral],
    ) -> Result<(), Error> {
        self.backend_call(|ptr| {
            // SAFETY: as in `backend_rule` (clingo.h:2005-2019).
            unsafe {
                ffi::clingo_backend_minimize(ptr, priority, literals.as_ptr(), literals.len())
            }
        })
    }

    /// Adds a projection directive (clingo.h:2020-2033).
    pub(crate) fn backend_project(&self, atoms: &[u32]) -> Result<(), Error> {
        self.backend_call(|ptr| {
            // SAFETY: as in `backend_rule` (clingo.h:2020-2033).
            unsafe { ffi::clingo_backend_project(ptr, atoms.as_ptr(), atoms.len()) }
        })
    }

    /// Adds an external statement (clingo.h:2034-2048).
    pub(crate) fn backend_external(&self, atom: u32, kind: ExternalKind) -> Result<(), Error> {
        let type_ = raw_external_type(kind);
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control
            // (clingo.h:2034-2048).
            unsafe { ffi::clingo_backend_external(ptr, atom, type_) }
        })
    }

    /// Adds an assumption directive (clingo.h:2049-2066).
    pub(crate) fn backend_assume(&self, literals: &[i32]) -> Result<(), Error> {
        self.backend_call(|ptr| {
            // SAFETY: as in `backend_rule` (clingo.h:2049-2066).
            unsafe { ffi::clingo_backend_assume(ptr, literals.as_ptr(), literals.len()) }
        })
    }

    /// Adds a heuristic directive (clingo.h:2067-2086).
    pub(crate) fn backend_heuristic(
        &self,
        atom: u32,
        kind: HeuristicKind,
        bias: i32,
        priority: u32,
        condition: &[i32],
    ) -> Result<(), Error> {
        let type_ = raw_heuristic_type(kind);
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control, and
            // `condition` outlives the call (clingo.h:2067-2086).
            unsafe {
                ffi::clingo_backend_heuristic(
                    ptr,
                    atom,
                    type_,
                    bias,
                    priority,
                    condition.as_ptr(),
                    condition.len(),
                )
            }
        })
    }

    /// Adds an edge directive (clingo.h:2087-2101).
    pub(crate) fn backend_acyc_edge(
        &self,
        node_u: i32,
        node_v: i32,
        condition: &[i32],
    ) -> Result<(), Error> {
        self.backend_call(|ptr| {
            // SAFETY: as in `backend_heuristic` (clingo.h:2087-2101).
            unsafe {
                ffi::clingo_backend_acyc_edge(
                    ptr,
                    node_u,
                    node_v,
                    condition.as_ptr(),
                    condition.len(),
                )
            }
        })
    }

    /// Adds a numeric theory term (clingo.h:2116-2129).
    pub(crate) fn backend_theory_number(&self, number: i32) -> Result<u32, Error> {
        let mut term_id = 0;
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control, and
            // `term_id` is a valid out-pointer (clingo.h:2116-2129).
            unsafe { ffi::clingo_backend_theory_term_number(ptr, number, &raw mut term_id) }
        })?;
        Ok(term_id)
    }

    /// Adds a string theory term (clingo.h:2130-2145).
    pub(crate) fn backend_theory_string(&self, string: &CStr) -> Result<u32, Error> {
        let mut term_id = 0;
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control, `string` is
            // NUL-terminated and outlives the call, and `term_id` is a valid
            // out-pointer (clingo.h:2130-2145).
            unsafe {
                ffi::clingo_backend_theory_term_string(ptr, string.as_ptr(), &raw mut term_id)
            }
        })?;
        Ok(term_id)
    }

    /// Adds a sequence theory term (clingo.h:2146-2163).
    pub(crate) fn backend_theory_sequence(
        &self,
        kind: TheorySequenceKind,
        arguments: &[u32],
    ) -> Result<u32, Error> {
        let type_ = raw_sequence_type(kind);
        let mut term_id = 0;
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control, `arguments`
            // outlives the call, and `term_id` is a valid out-pointer
            // (clingo.h:2146-2163).
            unsafe {
                ffi::clingo_backend_theory_term_sequence(
                    ptr,
                    type_,
                    arguments.as_ptr(),
                    arguments.len(),
                    &raw mut term_id,
                )
            }
        })?;
        Ok(term_id)
    }

    /// Adds a function theory term (clingo.h:2164-2179).
    pub(crate) fn backend_theory_function(
        &self,
        name: &CStr,
        arguments: &[u32],
    ) -> Result<u32, Error> {
        let mut term_id = 0;
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control, `name` is
            // NUL-terminated and outlives the call along with `arguments`,
            // and `term_id` is a valid out-pointer (clingo.h:2164-2179).
            unsafe {
                ffi::clingo_backend_theory_term_function(
                    ptr,
                    name.as_ptr(),
                    arguments.as_ptr(),
                    arguments.len(),
                    &raw mut term_id,
                )
            }
        })?;
        Ok(term_id)
    }

    /// Converts a symbol into a theory term (clingo.h:2180-2196).
    pub(crate) fn backend_theory_symbol(&self, symbol: RawSymbol) -> Result<u32, Error> {
        let mut term_id = 0;
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control, and
            // `term_id` is a valid out-pointer (clingo.h:2180-2196).
            unsafe { ffi::clingo_backend_theory_term_symbol(ptr, symbol, &raw mut term_id) }
        })?;
        Ok(term_id)
    }

    /// Adds a theory atom element (clingo.h:2197-2220).
    pub(crate) fn backend_theory_element(
        &self,
        tuple: &[u32],
        condition: &[i32],
    ) -> Result<u32, Error> {
        let mut element_id = 0;
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control, `tuple` and
            // `condition` outlive the call, and `element_id` is a valid
            // out-pointer (clingo.h:2197-2220).
            unsafe {
                ffi::clingo_backend_theory_element(
                    ptr,
                    tuple.as_ptr(),
                    tuple.len(),
                    condition.as_ptr(),
                    condition.len(),
                    &raw mut element_id,
                )
            }
        })?;
        Ok(element_id)
    }

    /// Adds a theory atom without a guard (clingo.h:2221-2244). `atom` is `0`
    /// for a directive, `u32::MAX` for a fresh atom, or an existing atom id
    /// (the header's own sentinel convention,
    /// [`crate::backend::TheoryAtomTarget`] makes this an enum at the typed
    /// layer).
    pub(crate) fn backend_theory_atom(
        &self,
        atom: u32,
        term_id: u32,
        elements: &[u32],
    ) -> Result<u32, Error> {
        let mut atom_id = 0;
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control, `elements`
            // outlives the call, and `atom_id` is a valid out-pointer
            // (clingo.h:2221-2244).
            unsafe {
                ffi::clingo_backend_theory_atom(
                    ptr,
                    atom,
                    term_id,
                    elements.as_ptr(),
                    elements.len(),
                    &raw mut atom_id,
                )
            }
        })?;
        Ok(atom_id)
    }

    /// Adds a theory atom with a guard (clingo.h:2245-2260). Same `atom`
    /// convention as `backend_theory_atom`.
    pub(crate) fn backend_theory_atom_with_guard(
        &self,
        atom: u32,
        term_id: u32,
        elements: &[u32],
        operator_name: &CStr,
        right_hand_side_id: u32,
    ) -> Result<u32, Error> {
        let mut atom_id = 0;
        self.backend_call(|ptr| {
            // SAFETY: `ptr` is the open backend of this control, `elements`
            // and `operator_name` (NUL-terminated) outlive the call, and
            // `atom_id` is a valid out-pointer (clingo.h:2245-2260).
            unsafe {
                ffi::clingo_backend_theory_atom_with_guard(
                    ptr,
                    atom,
                    term_id,
                    elements.as_ptr(),
                    elements.len(),
                    operator_name.as_ptr(),
                    right_hand_side_id,
                    &raw mut atom_id,
                )
            }
        })?;
        Ok(atom_id)
    }
}

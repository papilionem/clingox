//! Wrappers for the symbolic atoms of a grounding (clingo.h:520-690, 3315).

use std::marker::PhantomData;

use clingox_sys as ffi;

use super::control::ControlHandle;
use super::symbol::RawSignature;
use super::{RawSymbol, fill_vec, query};
use crate::error::Error;

/// A control's symbolic atoms, valid while the control is borrowed shared.
///
/// clingo keeps the object valid until the control grounds again
/// (clingo.h:520-530). Grounding takes `&mut ControlHandle`, so it cannot
/// happen while this borrows the handle.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Atoms<'c> {
    object: *const ffi::clingo_symbolic_atoms_t,
    _control: PhantomData<&'c ControlHandle>,
}

/// An iterator position in [`Atoms`] (clingo.h:540-545).
pub(crate) type AtomIterator = ffi::clingo_symbolic_atom_iterator_t;

/// Everything clingo tells about one atom.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AtomData {
    pub(crate) symbol: RawSymbol,
    pub(crate) literal: i32,
    pub(crate) fact: bool,
    pub(crate) external: bool,
}

impl<'c> Atoms<'c> {
    /// Wraps a pointer already obtained from clingo, for a source other than
    /// [`ControlHandle::symbolic_atoms`]:
    /// `raw::propagate::Init::symbolic_atoms` gets its pointer from
    /// `clingo_propagate_init_symbolic_atoms` instead of
    /// `clingo_control_symbolic_atoms`, but the object and every accessor below
    /// are identical either way; only the source of the pointer, and the
    /// lifetime it is tied to, differ.
    pub(crate) fn from_raw(object: *const ffi::clingo_symbolic_atoms_t) -> Atoms<'c> {
        Atoms {
            object,
            _control: PhantomData,
        }
    }
}

impl ControlHandle {
    /// The symbolic atoms of the current grounding (clingo.h:3315). The
    /// caller must have closed any leftover search.
    pub(crate) fn symbolic_atoms(&self) -> Result<Atoms<'_>, Error> {
        let mut object: *const ffi::clingo_symbolic_atoms_t = std::ptr::null();
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and
            // `object` is a valid out-pointer. The atoms belong to the control
            // (clingo.h:3315).
            unsafe { ffi::clingo_control_symbolic_atoms(control, &raw mut object) }
        })?;
        Ok(Atoms::from_raw(object))
    }
}

impl Atoms<'_> {
    /// The number of atoms (clingo.h:562).
    pub(crate) fn size(self) -> Result<usize, Error> {
        let mut size = 0;
        // SAFETY: `self.object` is valid while the control is borrowed (see
        // `Atoms`), and `size` is a valid out-pointer (clingo.h:562).
        query(|| unsafe { ffi::clingo_symbolic_atoms_size(self.object, &raw mut size) })?;
        Ok(size)
    }

    /// The first atom, of one signature or of all (clingo.h:570).
    pub(crate) fn begin(self, signature: Option<RawSignature>) -> Result<AtomIterator, Error> {
        let signature = signature
            .as_ref()
            .map_or(std::ptr::null(), std::ptr::from_ref);
        let mut iterator = 0;
        // SAFETY: as in `size`; `signature` is null or points to a signature on
        // this stack frame, and `iterator` is a valid out-pointer
        // (clingo.h:570).
        query(|| unsafe {
            ffi::clingo_symbolic_atoms_begin(self.object, signature, &raw mut iterator)
        })?;
        Ok(iterator)
    }

    /// The position past the last atom (clingo.h:578).
    pub(crate) fn end(self) -> Result<AtomIterator, Error> {
        let mut iterator = 0;
        // SAFETY: as in `size`; `iterator` is a valid out-pointer
        // (clingo.h:578).
        query(|| unsafe { ffi::clingo_symbolic_atoms_end(self.object, &raw mut iterator) })?;
        Ok(iterator)
    }

    /// Whether two positions are the same (clingo.h:596).
    pub(crate) fn is_equal(self, a: AtomIterator, b: AtomIterator) -> Result<bool, Error> {
        let mut equal = false;
        // SAFETY: as in `size`; both positions were returned by clingo for
        // these atoms, and `equal` is a valid out-pointer (clingo.h:596).
        query(|| unsafe {
            ffi::clingo_symbolic_atoms_iterator_is_equal_to(self.object, a, b, &raw mut equal)
        })?;
        Ok(equal)
    }

    /// The position after `iterator`, which must point to an atom
    /// (clingo.h:671).
    pub(crate) fn next(self, iterator: AtomIterator) -> Result<AtomIterator, Error> {
        let mut next = 0;
        // SAFETY: as in `size`; the caller passes a position that points to an
        // atom, and `next` is a valid out-pointer (clingo.h:671).
        query(|| unsafe { ffi::clingo_symbolic_atoms_next(self.object, iterator, &raw mut next) })?;
        Ok(next)
    }

    /// Reads the atom at `iterator`, which must point to an atom
    /// (clingo.h:606, 619, 630, 643).
    pub(crate) fn read(self, iterator: AtomIterator) -> Result<AtomData, Error> {
        let mut data = AtomData {
            symbol: 0,
            literal: 0,
            fact: false,
            external: false,
        };
        // SAFETY: as in `size`; the caller passes a position that points to an
        // atom, and each out-pointer here and below is a field of `data`
        // (clingo.h:606, 619, 630, 643).
        query(|| unsafe {
            ffi::clingo_symbolic_atoms_symbol(self.object, iterator, &raw mut data.symbol)
        })?;
        // SAFETY: as above.
        query(|| unsafe {
            ffi::clingo_symbolic_atoms_literal(self.object, iterator, &raw mut data.literal)
        })?;
        // SAFETY: as above.
        query(|| unsafe {
            ffi::clingo_symbolic_atoms_is_fact(self.object, iterator, &raw mut data.fact)
        })?;
        // SAFETY: as above.
        query(|| unsafe {
            ffi::clingo_symbolic_atoms_is_external(self.object, iterator, &raw mut data.external)
        })?;
        Ok(data)
    }

    /// The atom for `symbol`, or `None` if it is not an atom of the grounding
    /// (clingo.h:587, 682).
    pub(crate) fn find(self, symbol: RawSymbol) -> Result<Option<AtomData>, Error> {
        let mut iterator = 0;
        // SAFETY: as in `size`; `iterator` is a valid out-pointer. Any symbol
        // is accepted (clingo.h:587).
        query(|| unsafe {
            ffi::clingo_symbolic_atoms_find(self.object, symbol, &raw mut iterator)
        })?;
        let mut valid = false;
        // SAFETY: as in `size`; `iterator` was just returned by clingo, and
        // `valid` is a valid out-pointer (clingo.h:682).
        query(|| unsafe {
            ffi::clingo_symbolic_atoms_is_valid(self.object, iterator, &raw mut valid)
        })?;
        if valid {
            self.read(iterator).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Every predicate signature clingo knows, including those without atoms
    /// (clingo.h:651, 663).
    pub(crate) fn signatures(self) -> Result<Vec<RawSignature>, Error> {
        fill_vec(
            |size| {
                // SAFETY: as in `size`; `size` is a valid out-pointer
                // (clingo.h:651).
                query(|| unsafe { ffi::clingo_symbolic_atoms_signatures_size(self.object, size) })
            },
            |signatures, size| {
                // SAFETY: as above; `signatures` has room for `size` values, the
                // size clingo just reported (clingo.h:663).
                query(|| unsafe {
                    ffi::clingo_symbolic_atoms_signatures(self.object, signatures, size)
                })
            },
        )
    }
}

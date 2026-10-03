//! Wrappers for theory atom inspection (clingo.h:738-950, 3324-3325).

use std::marker::PhantomData;
use std::os::raw::c_int;

use clingox_sys as ffi;

use super::control::ControlHandle;
use super::{fill_string, query, raw_slice};
use crate::atoms::ProgramLiteral;
use crate::error::Error;
use crate::theory::Id;

/// A control's theory atoms, valid while the control is borrowed shared.
///
/// clingo resets all structural information about theory atoms, elements and
/// terms after solving (clingo.h:726-730); reading it needs no `&mut` (S5),
/// because grounding and solving both take `&mut ControlHandle`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Theory<'c> {
    object: *const ffi::clingo_theory_atoms_t,
    _control: PhantomData<&'c ControlHandle>,
}

/// The kind of a theory term, as clingo reports it (clingo.h:738-745).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TermType {
    Tuple,
    List,
    Set,
    Function,
    Number,
    Symbol,
    /// A value outside the six clingo 5.8.2 defines.
    Other(c_int),
}

impl<'c> Theory<'c> {
    /// Wraps a pointer already obtained from clingo, for a source other than
    /// [`ControlHandle::theory_atoms`]: `raw::propagate::Init::theory_atoms`
    /// gets its pointer from `clingo_propagate_init_theory_atoms`
    /// instead of `clingo_control_theory_atoms`, but the object and every
    /// accessor below are identical either way; only the source of the
    /// pointer, and the lifetime it is tied to, differ.
    pub(crate) fn from_raw(object: *const ffi::clingo_theory_atoms_t) -> Theory<'c> {
        Theory {
            object,
            _control: PhantomData,
        }
    }
}

impl ControlHandle {
    /// An object to inspect the theory atoms of the current grounding
    /// (clingo.h:3324). The caller must have closed any leftover search.
    pub(crate) fn theory_atoms(&self) -> Result<Theory<'_>, Error> {
        let mut object: *const ffi::clingo_theory_atoms_t = std::ptr::null();
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and
            // `object` is a valid out-pointer. The theory atoms belong to the
            // control (clingo.h:3324).
            unsafe { ffi::clingo_control_theory_atoms(control, &raw mut object) }
        })?;
        Ok(Theory::from_raw(object))
    }
}

impl<'c> Theory<'c> {
    /// The number of theory atoms (clingo.h:885).
    pub(crate) fn size(self) -> Result<usize, Error> {
        let mut size = 0;
        // SAFETY: `self.object` is valid while the control is borrowed (see
        // `Theory`), and `size` is a valid out-pointer (clingo.h:885).
        query(|| unsafe { ffi::clingo_theory_atoms_size(self.object, &raw mut size) })?;
        Ok(size)
    }

    /// The kind of a theory term (clingo.h:761-763).
    ///
    /// The bindgen constants are matched by name, never by position, as
    /// `raw::model_type` already does for `clingo_model_type_e`.
    ///
    /// # Safety rule
    ///
    /// `term` must be an id from the current `0..size()` range of this same,
    /// still-borrowed `Theory`: an out-of-range id is a clean
    /// `clingo_error_logic` here (checked directly against clingo 5.8.2), not
    /// a crash, unlike an out-of-range *atom* id (see `atom_term`). The safe
    /// layer never passes anything else.
    pub(crate) fn term_type(self, term: Id) -> Result<TermType, Error> {
        let mut kind = 0;
        // SAFETY: as in `size`; `term` came from the safe layer's own
        // `0..size()` range or from an id clingo itself returned (an
        // argument, a term's own id), and `kind` is a valid out-pointer
        // (clingo.h:761-763).
        query(|| unsafe {
            ffi::clingo_theory_atoms_term_type(self.object, term.raw(), &raw mut kind)
        })?;
        Ok(match u32::try_from(kind) {
            Ok(ffi::clingo_theory_term_type_tuple) => TermType::Tuple,
            Ok(ffi::clingo_theory_term_type_list) => TermType::List,
            Ok(ffi::clingo_theory_term_type_set) => TermType::Set,
            Ok(ffi::clingo_theory_term_type_function) => TermType::Function,
            Ok(ffi::clingo_theory_term_type_number) => TermType::Number,
            Ok(ffi::clingo_theory_term_type_symbol) => TermType::Symbol,
            _ => TermType::Other(kind),
        })
    }

    /// The number of a numeric theory term (clingo.h:770-777). The caller
    /// must have checked `term_type` is `Number` first.
    pub(crate) fn term_number(self, term: Id) -> Result<i32, Error> {
        let mut number = 0;
        // SAFETY: as in `term_type`; `number` is a valid out-pointer
        // (clingo.h:770-777).
        query(|| unsafe {
            ffi::clingo_theory_atoms_term_number(self.object, term.raw(), &raw mut number)
        })?;
        Ok(number)
    }

    /// The name of a function or symbol theory term (clingo.h:782-791). The
    /// caller must have checked `term_type` is `Function` or `Symbol` first.
    pub(crate) fn term_name(self, term: Id) -> Result<&'c str, Error> {
        let mut name = std::ptr::null();
        // SAFETY: as in `term_type`; `name` is a valid out-pointer
        // (clingo.h:782-791).
        query(|| unsafe {
            ffi::clingo_theory_atoms_term_name(self.object, term.raw(), &raw mut name)
        })?;
        // SAFETY: the header ties the string's lifetime to the current solve
        // step (clingo.h:774-775); `'c` is a borrow of the `Control` itself,
        // which cannot ground or solve again while it is alive (S5), so `'c`
        // never outlives the step this string is valid for.
        unsafe { super::borrowed_str(name) }
    }

    /// The argument term ids of a compound theory term (clingo.h:792-800).
    ///
    /// The header's own precondition names only `Function`, but clingo 5.8.2
    /// answers the same way for `Tuple`, `List` and `Set` (checked directly
    /// against the Python module), so the safe layer calls this for every
    /// compound kind.
    pub(crate) fn term_arguments(self, term: Id) -> Result<&'c [Id], Error> {
        let mut arguments: *const ffi::clingo_id_t = std::ptr::null();
        let mut size = 0;
        // SAFETY: as in `term_type`; both out-pointers are valid
        // (clingo.h:792-800).
        query(|| unsafe {
            ffi::clingo_theory_atoms_term_arguments(
                self.object,
                term.raw(),
                &raw mut arguments,
                &raw mut size,
            )
        })?;
        // SAFETY: `Id` is `#[repr(transparent)]` over `clingo_id_t` (u32), so
        // a valid array of `clingo_id_t` is a valid array of `Id`.
        // `arguments` points to `size` term ids owned by the theory atoms
        // table, valid while the control is borrowed (S5).
        Ok(unsafe { raw_slice(arguments.cast::<Id>(), size) })
    }

    /// The string representation of a theory term (clingo.h:801-827).
    ///
    /// Only the debug-build cross-check of `TheoryTerm`'s `Display` and the
    /// tests call it, so it is compiled for them alone and a release build has
    /// no dead code.
    #[cfg(any(debug_assertions, test))]
    pub(crate) fn term_to_string(self, term: Id) -> Result<String, Error> {
        fill_string(
            |size| {
                // SAFETY: as in `term_type`; `size` is a valid out-pointer
                // (clingo.h:801-813).
                query(|| unsafe {
                    ffi::clingo_theory_atoms_term_to_string_size(self.object, term.raw(), size)
                })
            },
            |buffer, size| {
                // SAFETY: `fill_string` passes a buffer of exactly the size
                // clingo asked for, including the NUL (clingo.h:814-827).
                query(|| unsafe {
                    ffi::clingo_theory_atoms_term_to_string(self.object, term.raw(), buffer, size)
                })
            },
        )
    }

    /// The term ids of a theory element's tuple (clingo.h:828-837).
    pub(crate) fn element_tuple(self, element: Id) -> Result<&'c [Id], Error> {
        let mut tuple: *const ffi::clingo_id_t = std::ptr::null();
        let mut size = 0;
        // SAFETY: as in `size`; `element` came from the safe layer's own
        // `0..len()` range of a theory atom's `elements()` (clingo.h:828-837).
        query(|| unsafe {
            ffi::clingo_theory_atoms_element_tuple(
                self.object,
                element.raw(),
                &raw mut tuple,
                &raw mut size,
            )
        })?;
        // SAFETY: as in `term_arguments`.
        Ok(unsafe { raw_slice(tuple.cast::<Id>(), size) })
    }

    /// The aspif literals of a theory element's condition (clingo.h:838-851).
    ///
    /// Unlike `term_arguments`, `element_tuple` and `atom_elements`, which
    /// point at stable per-object storage inside the theory atoms table
    /// (`DomainData::getTerm`/`getElement`/`TheoryData::getAtom` in
    /// `libgringo/src/output/literals.cc:1549-1567`), this array is a view
    /// into `DomainData::tempLits_`, one scratch `std::vector` that *every*
    /// call to `clingo_theory_atoms_element_condition`, for any element,
    /// clears and refills, and which reallocates as it grows
    /// (`libgringo/src/output/literals.cc:1555-1560`; UPSTREAM-ISSUES U24).
    /// So the literals are copied into an owned `Vec` right here, before
    /// returning, while they are still the ones this call just wrote: no
    /// borrow of the scratch buffer escapes this function for another
    /// clingo call to invalidate.
    pub(crate) fn element_condition(self, element: Id) -> Result<Vec<ProgramLiteral>, Error> {
        let mut condition: *const ffi::clingo_literal_t = std::ptr::null();
        let mut size = 0;
        // SAFETY: as in `element_tuple`; both out-pointers are valid
        // (clingo.h:838-851).
        query(|| unsafe {
            ffi::clingo_theory_atoms_element_condition(
                self.object,
                element.raw(),
                &raw mut condition,
                &raw mut size,
            )
        })?;
        // SAFETY: `ProgramLiteral` is `#[repr(transparent)]` over
        // `clingo_literal_t` (i32), so a valid array of `clingo_literal_t` is
        // a valid array of `ProgramLiteral`. Every literal of a theory
        // element's condition is a real aspif literal (never zero, always
        // within clasp's variable range): the same invariant that already
        // holds for every other aspif literal clingo hands back, which is
        // what makes `ProgramLiteral::from_raw` reject only impossible
        // values. `condition` points to `size` such literals, valid only
        // until clingo's next call on this same scratch buffer (U24): the
        // slice is read once, immediately, and copied into an owned `Vec`
        // (`to_vec`) before this function returns, so nothing borrows it
        // past this point.
        let literals = unsafe { raw_slice(condition.cast::<ProgramLiteral>(), size) };
        Ok(literals.to_vec())
    }

    /// The id of a theory element's condition (clingo.h:852-861).
    pub(crate) fn element_condition_id(self, element: Id) -> Result<i32, Error> {
        let mut condition = 0;
        // SAFETY: as in `element_tuple`; `condition` is a valid out-pointer
        // (clingo.h:852-861).
        query(|| unsafe {
            ffi::clingo_theory_atoms_element_condition_id(
                self.object,
                element.raw(),
                &raw mut condition,
            )
        })?;
        Ok(condition)
    }

    /// The string representation of a theory element (clingo.h:862-884).
    pub(crate) fn element_to_string(self, element: Id) -> Result<String, Error> {
        fill_string(
            |size| {
                // SAFETY: as in `element_tuple`; `size` is a valid out-pointer
                // (clingo.h:862-872).
                query(|| unsafe {
                    ffi::clingo_theory_atoms_element_to_string_size(
                        self.object,
                        element.raw(),
                        size,
                    )
                })
            },
            |buffer, size| {
                // SAFETY: `fill_string` passes a buffer of exactly the size
                // clingo asked for, including the NUL (clingo.h:873-884).
                query(|| unsafe {
                    ffi::clingo_theory_atoms_element_to_string(
                        self.object,
                        element.raw(),
                        buffer,
                        size,
                    )
                })
            },
        )
    }

    /// The theory term of a theory atom (clingo.h:892-900).
    ///
    /// # Safety rule
    ///
    /// `atom` must be an id from the current `0..size()` range of this same,
    /// still-borrowed `Theory`. Unlike a term or element id, an out-of-range
    /// *atom* id is not a clean clingo error: it segfaults the process (checked
    /// directly against clingo 5.8.2, "Safety analysis: atom ids are not
    /// bounds-checked"). The safe layer's `TheoryAtoms` never exposes an
    /// atom-by-id accessor and only ever calls the `atom_*` functions with an
    /// id `iter()` generated from `0..size()` itself, so no atom id from
    /// outside that range ever reaches here.
    pub(crate) fn atom_term(self, atom: Id) -> Result<Id, Error> {
        let mut term = 0;
        // SAFETY: `self.object` is valid while the control is borrowed, and
        // `atom` is in range for it (see this method's safety rule above);
        // `term` is a valid out-pointer (clingo.h:892-900).
        query(|| unsafe {
            ffi::clingo_theory_atoms_atom_term(self.object, atom.raw(), &raw mut term)
        })?;
        Ok(Id::from_raw(term))
    }

    /// The theory element ids of a theory atom (clingo.h:901-908). Same
    /// safety rule as `atom_term`.
    pub(crate) fn atom_elements(self, atom: Id) -> Result<&'c [Id], Error> {
        let mut elements: *const ffi::clingo_id_t = std::ptr::null();
        let mut size = 0;
        // SAFETY: as in `atom_term`; both out-pointers are valid
        // (clingo.h:901-908).
        query(|| unsafe {
            ffi::clingo_theory_atoms_atom_elements(
                self.object,
                atom.raw(),
                &raw mut elements,
                &raw mut size,
            )
        })?;
        // SAFETY: as in `term_arguments`.
        Ok(unsafe { raw_slice(elements.cast::<Id>(), size) })
    }

    /// Whether a theory atom has a guard (clingo.h:909-920). Same safety rule
    /// as `atom_term`.
    pub(crate) fn atom_has_guard(self, atom: Id) -> Result<bool, Error> {
        let mut has_guard = false;
        // SAFETY: as in `atom_term`; `has_guard` is a valid out-pointer
        // (clingo.h:909-920).
        query(|| unsafe {
            ffi::clingo_theory_atoms_atom_has_guard(self.object, atom.raw(), &raw mut has_guard)
        })?;
        Ok(has_guard)
    }

    /// The guard of a theory atom: a connective and a term id
    /// (clingo.h:921-928). Same safety rule as `atom_term`. The caller must
    /// have checked `atom_has_guard` first: clingo 5.8.2 tolerates the call
    /// without a guard and reports a null connective, so the safe layer never
    /// calls this otherwise.
    pub(crate) fn atom_guard(self, atom: Id) -> Result<(&'c str, Id), Error> {
        let mut connective = std::ptr::null();
        let mut term = 0;
        // SAFETY: as in `atom_term`; both out-pointers are valid
        // (clingo.h:921-928).
        query(|| unsafe {
            ffi::clingo_theory_atoms_atom_guard(
                self.object,
                atom.raw(),
                &raw mut connective,
                &raw mut term,
            )
        })?;
        // SAFETY: as in `term_name`, the same solve-step-scoped lifetime the
        // header states for this string too (clingo.h:913-914).
        let connective = unsafe { super::borrowed_str(connective) }?;
        Ok((connective, Id::from_raw(term)))
    }

    /// The aspif literal of a theory atom, or `0` for a `directive`-role atom
    /// that is never part of a rule (clingo.h:929-937).
    /// Same safety rule as `atom_term`.
    pub(crate) fn atom_literal(self, atom: Id) -> Result<i32, Error> {
        let mut literal = 0;
        // SAFETY: as in `atom_term`; `literal` is a valid out-pointer
        // (clingo.h:929-937).
        query(|| unsafe {
            ffi::clingo_theory_atoms_atom_literal(self.object, atom.raw(), &raw mut literal)
        })?;
        Ok(literal)
    }

    /// The string representation of a theory atom (clingo.h:938-950). Same
    /// safety rule as `atom_term`.
    pub(crate) fn atom_to_string(self, atom: Id) -> Result<String, Error> {
        fill_string(
            |size| {
                // SAFETY: as in `atom_term`; `size` is a valid out-pointer
                // (clingo.h:938-948).
                query(|| unsafe {
                    ffi::clingo_theory_atoms_atom_to_string_size(self.object, atom.raw(), size)
                })
            },
            |buffer, size| {
                // SAFETY: `fill_string` passes a buffer of exactly the size
                // clingo asked for, including the NUL (clingo.h:949-950).
                query(|| unsafe {
                    ffi::clingo_theory_atoms_atom_to_string(self.object, atom.raw(), buffer, size)
                })
            },
        )
    }
}

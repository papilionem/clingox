//! Wrappers for reading a model (clingo.h:2328-2424).
//!
//! A [`Model`] is only ever a reference that [`ControlHandle::solve_model`]
//! lends, pointing at clingo's model object. The functions here take it by
//! reference and pass that pointer back to clingo.
//!
//! [`ControlHandle::solve_model`]: super::ControlHandle::solve_model

use std::ffi::c_int;
use std::marker::{PhantomData, PhantomPinned};

use clingox_sys as ffi;

use super::{Atoms, RawSymbol, call, fill_vec, query};
use crate::Model;
use crate::error::Error;

/// The contents of a [`Model`]: nothing that Rust can see.
///
/// It is zero-sized with alignment 1, so a reference to it made from clingo's
/// model pointer covers no bytes, and clingo may change the object behind it.
/// Its fields are private to `raw`, so no value of it, and hence no `Model`,
/// can be made anywhere else: every `&Model` comes from clingo. The marker
/// makes it `!Send`, `!Sync` and `!Unpin`, as a foreign object should be.
#[repr(C)]
pub(crate) struct ModelData {
    _opaque: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

// `solve_model` relies on `Model` being a view of zero bytes.
const _: () = assert!(size_of::<Model>() == 0 && align_of::<Model>() == 1);

/// The show type bits (clingo.h:2290-2297), for `ShowType`.
pub(crate) mod show {
    use clingox_sys as ffi;

    pub(crate) const SHOWN: u32 = ffi::clingo_show_type_shown;
    pub(crate) const ATOMS: u32 = ffi::clingo_show_type_atoms;
    pub(crate) const TERMS: u32 = ffi::clingo_show_type_terms;
    pub(crate) const THEORY: u32 = ffi::clingo_show_type_theory;
    pub(crate) const COMPLEMENT: u32 = ffi::clingo_show_type_complement;
}

/// clingo's pointer to the model a reference views.
fn ptr(model: &Model) -> *const ffi::clingo_model_t {
    std::ptr::from_ref(model).cast::<ffi::clingo_model_t>()
}

/// The running number of the model (clingo.h:2328).
///
/// clingo reports failure through the return value, but this call cannot
/// fail: it copies a field inside a `try` block that nothing in it can throw
/// from (control.cc:960-963, clingocontrol.hh:450). So the flag is not read
/// through `call`, and the error state it would leave is reset by the next
/// `call` (S1).
pub(crate) fn model_number(model: &Model) -> u64 {
    let mut number = 0;
    // SAFETY: `ptr(model)` is a live model: every `&Model` is lent by
    // `solve_model` for as long as its model is valid. `number` is a valid
    // out-pointer (clingo.h:2328).
    let ok = unsafe { ffi::clingo_model_number(ptr(model), &raw mut number) };
    debug_assert!(ok, "clingo_model_number only copies a field");
    number
}

/// The id of the solver thread that found the model (clingo.h:2377-2383).
///
/// clingo reports failure through the return value, but this call cannot
/// fail: `threadId()` (`clingocontrol.hh:451`) is `{ return model_->sId; }`,
/// the identical "copies a field inside a `try` block that nothing in it can
/// throw from" shape [`model_number`] already documents and handles
/// (`control.cc:869-872`). So the flag
/// is not read through [`call`], and the error state it would leave is reset
/// by the next `call` (S1).
pub(crate) fn model_thread_id(model: &Model) -> u32 {
    let mut id = 0;
    // SAFETY: `ptr(model)` is a live model (see `model_number`), and `id` is
    // a valid out-pointer (clingo.h:2377).
    let ok = unsafe { ffi::clingo_model_thread_id(ptr(model), &raw mut id) };
    debug_assert!(ok, "clingo_model_thread_id only copies a field");
    id
}

/// The kind of a model clingo reports (clingo.h:2283-2287).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModelType {
    StableModel,
    BraveConsequences,
    CautiousConsequences,
    /// A value outside the three clingo 5.8.2 defines.
    Other(c_int),
}

/// The model's kind (clingo.h:2317-2322).
///
/// The bindgen constants are matched by name, never by position, so a header
/// whose declaration order changed could not silently swap two kinds.
pub(crate) fn model_type(model: &Model) -> Result<ModelType, Error> {
    let mut kind = 0;
    // SAFETY: `ptr(model)` is a live model (see `model_number`), and `kind`
    // is a valid out-pointer (clingo.h:2317).
    query(|| unsafe { ffi::clingo_model_type(ptr(model), &raw mut kind) })?;
    Ok(match u32::try_from(kind) {
        Ok(ffi::clingo_model_type_stable_model) => ModelType::StableModel,
        Ok(ffi::clingo_model_type_brave_consequences) => ModelType::BraveConsequences,
        Ok(ffi::clingo_model_type_cautious_consequences) => ModelType::CautiousConsequences,
        _ => ModelType::Other(kind),
    })
}

/// The tri-state of a literal under brave or cautious enumeration
/// (clingo.h:2304-2310).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConsequenceValue {
    False,
    True,
    Unknown,
    /// A value outside the three clingo 5.8.2 defines.
    Other(c_int),
}

/// Whether a literal is a consequence (clingo.h:2371-2384). Outside brave or
/// cautious enumeration this always agrees with `model_is_true` on the same
/// literal (the header's own fallback text).
pub(crate) fn model_is_consequence(model: &Model, literal: i32) -> Result<ConsequenceValue, Error> {
    let mut result = 0;
    // SAFETY: `ptr(model)` is a live model (see `model_number`), and `result`
    // is a valid out-pointer. Any literal is accepted, as for `model_is_true`
    // (clingo.h:2371).
    query(|| unsafe { ffi::clingo_model_is_consequence(ptr(model), literal, &raw mut result) })?;
    Ok(match u32::try_from(result) {
        Ok(ffi::clingo_consequence_false) => ConsequenceValue::False,
        Ok(ffi::clingo_consequence_true) => ConsequenceValue::True,
        Ok(ffi::clingo_consequence_unknown) => ConsequenceValue::Unknown,
        _ => ConsequenceValue::Other(result),
    })
}

/// The priority level behind each entry of the cost vector, highest priority
/// first (clingo.h:2403-2416). `clingo_model_cost_size` also sizes this array
/// (the header states the two share a size), so `model_cost`'s size query is
/// reused instead of a separate one.
pub(crate) fn model_priority(model: &Model) -> Result<Vec<i32>, Error> {
    fill_vec(
        |size| {
            // SAFETY: `ptr(model)` is a live model (see `model_number`), and
            // `size` is a valid out-pointer (clingo.h:2390).
            query(|| unsafe { ffi::clingo_model_cost_size(ptr(model), size) })
        },
        |priorities, size| {
            // SAFETY: as above; `priorities` has room for `size` values, the
            // size clingo just reported (clingo.h:2416).
            query(|| unsafe { ffi::clingo_model_priority(ptr(model), priorities, size) })
        },
    )
}

/// The cost vector, highest priority first (clingo.h:2390, 2402).
pub(crate) fn model_cost(model: &Model) -> Result<Vec<i64>, Error> {
    fill_vec(
        |size| {
            // SAFETY: `ptr(model)` is a live model (see `model_number`), and
            // `size` is a valid out-pointer (clingo.h:2390).
            query(|| unsafe { ffi::clingo_model_cost_size(ptr(model), size) })
        },
        |costs, size| {
            // SAFETY: as above; `costs` has room for `size` values, the size
            // clingo just reported (clingo.h:2402).
            query(|| unsafe { ffi::clingo_model_cost(ptr(model), costs, size) })
        },
    )
}

/// Whether the model is proven optimal (clingo.h:2424).
pub(crate) fn model_optimality_proven(model: &Model) -> Result<bool, Error> {
    let mut proven = false;
    // SAFETY: `ptr(model)` is a live model (see `model_number`), and `proven`
    // is a valid out-pointer (clingo.h:2424).
    query(|| unsafe { ffi::clingo_model_optimality_proven(ptr(model), &raw mut proven) })?;
    Ok(proven)
}

/// The symbols selected by the show bits, in clingo's order (clingo.h:2336,
/// 2353).
pub(crate) fn model_symbols(model: &Model, show: u32) -> Result<Vec<RawSymbol>, Error> {
    fill_vec(
        |size| {
            // SAFETY: `ptr(model)` is a live model (see `model_number`), and
            // `size` is a valid out-pointer. Any combination of show bits is
            // accepted (clingo.h:2336).
            query(|| unsafe { ffi::clingo_model_symbols_size(ptr(model), show, size) })
        },
        |symbols, size| {
            // SAFETY: as above; `symbols` has room for `size` symbols, the size
            // clingo just reported (clingo.h:2353).
            query(|| unsafe { ffi::clingo_model_symbols(ptr(model), show, symbols, size) })
        },
    )
}

/// Whether the atom is true in the model (clingo.h:2361).
pub(crate) fn model_contains(model: &Model, atom: RawSymbol) -> Result<bool, Error> {
    let mut contained = false;
    // SAFETY: `ptr(model)` is a live model (see `model_number`), and
    // `contained` is a valid out-pointer (clingo.h:2361).
    query(|| unsafe { ffi::clingo_model_contains(ptr(model), atom, &raw mut contained) })?;
    Ok(contained)
}

/// Whether a program literal is true in the model (clingo.h:2369).
pub(crate) fn model_is_true(model: &Model, literal: i32) -> Result<bool, Error> {
    let mut is_true = false;
    // SAFETY: `ptr(model)` is a live model (see `model_number`), and
    // `is_true` is a valid out-pointer. Any literal is accepted
    // (clingo.h:2369).
    query(|| unsafe { ffi::clingo_model_is_true(ptr(model), literal, &raw mut is_true) })?;
    Ok(is_true)
}

/// The model of a solve event's model event, as a mutable reference
/// (`ExtendableModel`, DESIGN S6): unlike [`model_number`] and its
/// siblings, which read through the `const *` every ordinary lent
/// [`Model`] gives, [`clingo_model_extend`] needs the genuinely non-const
/// pointer clingo hands the callback for this one event
/// (`clingo.hh:4297`: `Model m{static_cast<clingo_model_t *>(event)};`,
/// confirming H:2435's own "only models passed to the
/// `clingo_solve_event_callback_t` are extendable").
///
/// # Safety
///
/// `model` must be a live, non-const `clingo_model_t *`, valid for `'m`
/// (the duration of the model event that delivered it).
pub(crate) unsafe fn extendable_model<'m>(model: *mut ffi::clingo_model_t) -> &'m mut Model {
    // SAFETY: the caller's contract above. `Model` is zero-sized with
    // alignment 1 (the static assert next to `ModelData`), so the mutable
    // reference covers no bytes of clingo's own object; every ordinary read
    // through a `&Model` built from the same pointer (`ptr`, above) narrows
    // it to `const` in the same way C itself would, and nothing here hands
    // out a second live `&mut` to the same model: the trampoline calls this
    // exactly once per model event, and the returned reference does not
    // outlive that call (S6's own lifetime note).
    unsafe { &mut *model.cast::<Model>() }
}

/// A shared view of the model clingo passes to the application's model
/// printer.
///
/// # Safety
///
/// `model` must be a live `clingo_model_t *` that stays valid for `'m` (the
/// duration of the printer call that delivered it).
pub(crate) unsafe fn printer_model<'m>(model: *const ffi::clingo_model_t) -> &'m Model {
    // SAFETY: the caller's contract above. `Model` is zero-sized with
    // alignment 1 (the static assert next to `ModelData`), so the reference
    // covers no bytes of clingo's object, and a `&` never allows a write.
    unsafe { &*model.cast::<Model>() }
}

/// Adds symbols to the model (clingo.h:2441). Only meaningful from the
/// model reached through [`extendable_model`] (H:2431-2434).
pub(crate) fn model_extend(model: &mut Model, symbols: &[RawSymbol]) -> Result<(), Error> {
    // SAFETY: `model` is the live, non-const model `extendable_model`
    // produced, borrowed mutably for the call. `symbols` holds
    // `symbols.len()` valid symbols (`RawSymbol`/`Symbol` are
    // `repr(transparent)`) that outlive the call; clingo copies them
    // (clingo.h:2441).
    call(|| unsafe {
        ffi::clingo_model_extend(
            std::ptr::from_mut(model).cast(),
            symbols.as_ptr(),
            symbols.len(),
        )
    })
}

// ---------------------------------------------------------------------------
// SolveControl (clingo.h:2452-2474): `clingo_model_context` gives back a
// `clingo_solve_control_t *`, on which `clingo_solve_control_symbolic_atoms`/
// `clingo_solve_control_add_clause` are called for the rest of the current
// solving step. Held as a bare pointer, exactly like [`super::
// RawPropagateControl`]: neither this type nor [`Model`] ever forms a Rust
// reference into whatever bytes clingo keeps behind either pointer, so a live
// `&Model` and a `RawSolveControl` borrowed from it coexisting asserts
// nothing about memory Rust reads (the ASan
// integration test is the primary evidence for the C side of this, this
// type's own pointer handling is what the Miri tests below cover).

/// The raw `clingo_solve_control_t` [`model_context`] hands back, borrowed
/// for exactly the duration of the [`Model`] it came from.
pub(crate) struct RawSolveControl<'m> {
    ptr: *mut ffi::clingo_solve_control_t,
    _marker: PhantomData<&'m Model>,
}

impl<'m> RawSolveControl<'m> {
    /// # Safety
    ///
    /// `ptr` must be a valid `clingo_solve_control_t` pointer, usable for
    /// `'m`.
    unsafe fn from_raw(ptr: *mut ffi::clingo_solve_control_t) -> RawSolveControl<'m> {
        RawSolveControl {
            ptr,
            _marker: PhantomData,
        }
    }

    /// clingo.h:2452-2459.
    pub(crate) fn symbolic_atoms(&self) -> Result<Atoms<'m>, Error> {
        let mut object: *const ffi::clingo_symbolic_atoms_t = std::ptr::null();
        // SAFETY: `self.ptr` is valid for `'m` (constructor's contract), and
        // `object` is a valid out-pointer (clingo.h:2452).
        call(|| unsafe {
            ffi::clingo_solve_control_symbolic_atoms(self.ptr.cast_const(), &raw mut object)
        })?;
        Ok(Atoms::from_raw(object))
    }

    /// clingo.h:2461-2474. Takes `&self`, not `&mut self`:
    /// clingo's own header gives `clingo_solve_control_add_clause` a `*mut`
    /// receiver, so it is passed straight through from the field, never
    /// derived from `&self` itself.
    pub(crate) fn add_clause(&self, clause: &[i32]) -> Result<(), Error> {
        // SAFETY: `self.ptr` is valid for `'m` (constructor's contract);
        // `clause` holds `clause.len()` literals and outlives the call
        // (clingo.h:2461-2474). Any literal value is accepted, including one
        // foreign to this program's own grounding (checked directly against
        // clingo 5.8.2): clingo either narrows the step or reports
        // `bad_alloc`/`runtime`, never crashing.
        call(|| unsafe {
            ffi::clingo_solve_control_add_clause(self.ptr, clause.as_ptr(), clause.len())
        })
    }
}

/// The solve control of a model (clingo.h:2444-2450).
///
/// clingo reports failure through the return value, but this call cannot fail:
/// its own C body (`control.cc:955-958`) is a pure pointer cast
/// (`static_cast`+`const_cast`, no allocation, no lookup, no virtual dispatch),
/// the same "cannot throw" shape [`model_number`]/ [`model_thread_id`] already
/// document. So the flag is not read through [`call`], and the error state it
/// would leave is reset by the next `call` (S1).
pub(crate) fn model_context(model: &Model) -> RawSolveControl<'_> {
    let mut control: *mut ffi::clingo_solve_control_t = std::ptr::null_mut();
    // SAFETY: `ptr(model)` is a live model (see `model_number`), and
    // `control` is a valid out-pointer (clingo.h:2444).
    let ok = unsafe { ffi::clingo_model_context(ptr(model), &raw mut control) };
    debug_assert!(ok, "clingo_model_context only performs a pointer cast");
    // SAFETY: on success (asserted above) clingo always writes a non-null
    // `clingo_solve_control_t` pointer, usable for as long as `model` stays
    // borrowed (the same object `model`'s own pointer names, reinterpreted).
    unsafe { RawSolveControl::from_raw(control) }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;

    use super::*;

    /// [`RawSolveControl::from_raw`]'s own pointer handling, Miri-testable
    /// without a real clingo model (which Miri cannot call into): a
    /// dangling but well-aligned `clingo_solve_control_t` pointer (the same
    /// technique `raw::stats::MutableStats::for_test` uses for
    /// `clingo_statistics_t`, since both are zero-sized opaque C types,
    /// `clingox-sys/src/bindings.rs`) round-trips through the type's own
    /// storage and its `as_const`-style cast unchanged; no FFI call is made.
    #[test]
    fn raw_solve_control_stores_and_casts_its_pointer_without_calling_clingo() {
        let dangling = NonNull::<ffi::clingo_solve_control_t>::dangling().as_ptr();
        // SAFETY: this test never dereferences the pointer or calls a
        // clingo function with it; it only exercises the wrapper's own
        // storage and cast, which read the pointer value, not its pointee.
        let control = unsafe { RawSolveControl::from_raw(dangling) };
        assert_eq!(control.ptr, dangling);
        assert_eq!(control.ptr.cast_const(), dangling.cast_const());
    }

    /// `Model` and `RawSolveControl` are both zero-sized: a live `&Model`
    /// and a `RawSolveControl` built from the same address coexist without
    /// either ever exposing a Rust reference into clingo's own object (the
    /// pointer field is data, not a borrow), which is what makes the `ASan`
    /// "read the model after `add_clause`" integration test sound to write
    /// in the first place.
    #[test]
    fn model_is_zero_sized_and_raw_solve_control_carries_only_a_pointer() {
        assert_eq!(size_of::<Model>(), 0);
        assert_eq!(align_of::<Model>(), 1);
        assert_eq!(
            size_of::<RawSolveControl<'_>>(),
            size_of::<*mut ffi::clingo_solve_control_t>(),
            "RawSolveControl holds exactly one pointer, nothing else"
        );
    }
}

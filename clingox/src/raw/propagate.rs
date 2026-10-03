//! Propagator registration, `PropagateInit`, watches, dispatch for the five
//! `Propagator` callbacks, and the minimal `Assignment`/`PropagateControl` the
//! safe layer needs (clingo.h:969-1370, 1373-1460, 3159-3175).
//!
//! `init` is the only one of the five that ever runs concurrently with nothing
//! else: it runs once, before any solver thread exists (`clingo.h:1483-1489`;
//! checked directly against `control.cc:2088-2124` and `clasp/src/clingo.cpp`).
//! The other four can run on several solver threads at once, and `undo` can run
//! re-entrantly on the *same* thread while `propagate` is still on the stack
//! (S11, DESIGN.md). [`Propagator`] is `Send + Sync` with every method taking
//! `&self` for exactly that reason.
//!
//! Every trampoline follows the ground callback's shape (`raw::trampoline`,
//! S8): read the boxed [`PropagatorContext`] from `data`, run the matching
//! [`Propagator`] method inside a panic guard, store a returned error or a
//! caught panic in the context's slots (one slot pair per registered
//! propagator, shared by all five callbacks: a failure on any one of them must
//! stop every later call on that same propagator at once, S8), and return
//! `false`/nothing to clingo on either. `init`'s own error or panic poisons the
//! whole `Control` unconditionally (it is structurally identical to the
//! ground/observer callbacks S3 already covers);
//! `propagate`/`undo`/`check`/`decide`'s error or panic never poisons, whatever
//! its kind (measured directly, a further `solve` on the same control completes
//! normally after one of these). Each slot remembers which callback recorded it
//! ([`Origin`]), so [`Control::settle_propagators`](crate::control::Control)
//! can tell the two cases apart.
//!
//! Unlike `raw::observer`, most of the conversion between clingo's raw values
//! and clingox's public types happens right here, in the raw layer (mirroring
//! `raw::observer`'s own `heuristic_kind`/`external_kind` helpers, which
//! already return safe-layer enums): `Init` and `PropagateControl` below hand
//! back [`SolverLiteral`], [`Flow`], [`CheckMode`], [`UndoMode`] and the atom
//! views directly, so the safe layer (`crate::propagate`) only adds the
//! post-`Stop` guard and the cross-control literal validation, never touches
//! `clingox_sys`.

use std::any::Any;
use std::ffi::{c_int, c_uint, c_void};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};

use clingox_sys as ffi;

use super::control::ControlHandle;
use super::trampoline::{Slot, fail};
use super::{ErrorState, c_int_of, call, query, raw_slice, weighted_literal};
use crate::error::Error;
use crate::propagate::{
    Assignment, CheckMode, ClauseType, Flow, PropagateControl, PropagateInit, Propagator,
    SolverLiteral, UndoMode, WeightConstraintKind, foreign_literal,
};

// ---------------------------------------------------------------------------
// RawAssignment: the read-only accessors behind `Assignment`, plus
// `has_literal`, which the cross-control literal guard also calls directly.

/// A view of the assignment clingo shows a propagator: shared by
/// [`Init::assignment`] and [`PropagateControl::assignment`], since
/// `clingo_propagate_init_assignment` and `clingo_propagate_control_
/// assignment` both hand back the same object type (clingo.h:1272, 1391).
#[derive(Clone, Copy, Debug)]
pub(crate) struct RawAssignment<'a> {
    object: *const ffi::clingo_assignment_t,
    _marker: PhantomData<&'a ()>,
}

impl<'a> RawAssignment<'a> {
    /// # Safety
    ///
    /// `object` must be a valid `clingo_assignment_t` pointer, readable for
    /// `'a`.
    unsafe fn from_raw(object: *const ffi::clingo_assignment_t) -> RawAssignment<'a> {
        RawAssignment {
            object,
            _marker: PhantomData,
        }
    }

    /// clingo.h:997-1001: reads a field, cannot fail.
    pub(crate) fn decision_level(self) -> u32 {
        // SAFETY: `self.object` is valid for `'a` (constructor's contract).
        unsafe { ffi::clingo_assignment_decision_level(self.object) }
    }

    /// clingo.h:1003-1007: reads a field, cannot fail.
    pub(crate) fn root_level(self) -> u32 {
        // SAFETY: as `decision_level`.
        unsafe { ffi::clingo_assignment_root_level(self.object) }
    }

    /// clingo.h:1009-1013: reads a field, cannot fail.
    pub(crate) fn has_conflict(self) -> bool {
        // SAFETY: as `decision_level`.
        unsafe { ffi::clingo_assignment_has_conflict(self.object) }
    }

    /// clingo.h:1057-1060: reads a field, cannot fail.
    pub(crate) fn size(self) -> usize {
        // SAFETY: as `decision_level`.
        unsafe { ffi::clingo_assignment_size(self.object) }
    }

    /// clingo.h:1085-1088: reads a field, cannot fail.
    pub(crate) fn is_total(self) -> bool {
        // SAFETY: as `decision_level`.
        unsafe { ffi::clingo_assignment_is_total(self.object) }
    }

    /// Whether `literal` is part of this assignment (clingo.h:1015-1019).
    ///
    /// Safe for any `i32`, including huge, negative, zero and
    /// `i32::MIN`/`i32::MAX` values: probed directly through pyclingo's
    /// `_lib`/`_ffi`, bypassing the Python wrapper, on a real assignment; it
    /// never crashes, only returns `false` for anything out of range. This is
    /// exactly what makes it usable as a guard: a `SolverLiteral` legitimately
    /// obtained from a *different* control's grounding is a value this function
    /// must reject safely, not crash on, since `SolverLiteral` has no public
    /// constructor that could be checked before the fact.
    pub(crate) fn has_literal(self, literal: i32) -> bool {
        // SAFETY: as `decision_level`; `clingo_assignment_has_literal` never
        // dereferences `literal` itself, only compares it against the
        // assignment's own size (see the doc comment above).
        unsafe { ffi::clingo_assignment_has_literal(self.object, literal) }
    }

    /// clingo.h:1017-1024: the decision level of a literal. `u32::MAX` for a
    /// known but currently unassigned literal, not an error (`clasp/src/
    /// clingo.cpp:62-66`; checked against the
    /// oracle); the safe layer converts that sentinel to `None`.
    pub(crate) fn level(self, literal: i32) -> Result<u32, Error> {
        let mut result = 0;
        // SAFETY: `self.object` is valid for `'a` (constructor's contract),
        // and `result` is a valid out-pointer for the call.
        query(|| unsafe { ffi::clingo_assignment_level(self.object, literal, &raw mut result) })?;
        Ok(result)
    }

    /// clingo.h:1025-1032: the decision literal at a level. `decision(0)` is
    /// always clasp's own "trivially true" sentinel, regardless of the
    /// program (`clasp/src/clingo.cpp:67-74`, checked directly with a
    /// fact-free program too).
    pub(crate) fn decision(self, level: u32) -> Result<SolverLiteral, Error> {
        let mut result = 0;
        // SAFETY: as `level`.
        query(|| unsafe { ffi::clingo_assignment_decision(self.object, level, &raw mut result) })?;
        Ok(SolverLiteral::from_raw_valid(result))
    }

    /// clingo.h:1033-1040.
    pub(crate) fn is_fixed(self, literal: i32) -> Result<bool, Error> {
        let mut result = false;
        // SAFETY: as `level`.
        query(|| unsafe {
            ffi::clingo_assignment_is_fixed(self.object, literal, &raw mut result)
        })?;
        Ok(result)
    }

    /// clingo.h:1041-1049.
    pub(crate) fn is_true(self, literal: i32) -> Result<bool, Error> {
        let mut result = false;
        // SAFETY: as `level`.
        query(|| unsafe { ffi::clingo_assignment_is_true(self.object, literal, &raw mut result) })?;
        Ok(result)
    }

    /// clingo.h:1050-1058.
    pub(crate) fn is_false(self, literal: i32) -> Result<bool, Error> {
        let mut result = false;
        // SAFETY: as `level`.
        query(|| unsafe {
            ffi::clingo_assignment_is_false(self.object, literal, &raw mut result)
        })?;
        Ok(result)
    }

    /// clingo.h:1059-1066: `free`/`true`/`false`, mapped through
    /// [`truth_value_from_raw`].
    pub(crate) fn truth_value(self, literal: i32) -> Result<Option<bool>, Error> {
        let mut result = 0;
        // SAFETY: as `level`.
        query(|| unsafe {
            ffi::clingo_assignment_truth_value(self.object, literal, &raw mut result)
        })?;
        Ok(truth_value_from_raw(result))
    }

    /// clingo.h:1072-1079: the positive literal at an ascending offset,
    /// distinct from the trail's own chronological order (`trail_at`).
    pub(crate) fn at(self, offset: usize) -> Result<SolverLiteral, Error> {
        let mut result = 0;
        // SAFETY: as `level`.
        query(|| unsafe { ffi::clingo_assignment_at(self.object, offset, &raw mut result) })?;
        Ok(SolverLiteral::from_raw_valid(result))
    }

    /// clingo.h:1085-1090: reads a field; declared `Result` because the
    /// header gives it the success-flag shape, though no failure is known.
    pub(crate) fn trail_size(self) -> Result<u32, Error> {
        let mut result = 0;
        // SAFETY: as `level`.
        query(|| unsafe { ffi::clingo_assignment_trail_size(self.object, &raw mut result) })?;
        Ok(result)
    }

    /// clingo.h:1091-1105.
    pub(crate) fn trail_begin(self, level: u32) -> Result<u32, Error> {
        let mut result = 0;
        // SAFETY: as `level`.
        query(|| unsafe {
            ffi::clingo_assignment_trail_begin(self.object, level, &raw mut result)
        })?;
        Ok(result)
    }

    /// clingo.h:1106-1115: never fails at the clingo level for any `level`
    /// at or above the current decision level
    /// (`AbstractAssignment::trailEnd`'s own unconditional `trailSize()`
    /// fallback, `clasp/libpotassco/src/clingo.cpp:26-30`); the safe layer
    /// validates `level` itself before this is ever called.
    pub(crate) fn trail_end(self, level: u32) -> Result<u32, Error> {
        let mut result = 0;
        // SAFETY: as `level`.
        query(|| unsafe { ffi::clingo_assignment_trail_end(self.object, level, &raw mut result) })?;
        Ok(result)
    }

    /// clingo.h:1116-1123: the (possibly negated) literal at a trail
    /// offset, in chronological order, distinct from `at`'s own ascending
    /// order.
    pub(crate) fn trail_at(self, offset: u32) -> Result<SolverLiteral, Error> {
        let mut result = 0;
        // SAFETY: as `level`.
        query(|| unsafe { ffi::clingo_assignment_trail_at(self.object, offset, &raw mut result) })?;
        Ok(SolverLiteral::from_raw_valid(result))
    }
}

/// # Safety
///
/// `object` must be a valid `clingo_assignment_t` pointer, readable for `'a`
/// (the same contract as [`RawAssignment::from_raw`], which this calls).
unsafe fn assignment_from_raw<'a>(object: *const ffi::clingo_assignment_t) -> Assignment<'a> {
    // SAFETY: the caller's contract above.
    Assignment::from_raw(unsafe { RawAssignment::from_raw(object) })
}

// ---------------------------------------------------------------------------
// Enum conversions (S17): never by position or by `transmute`, always from
// the bindgen constant. `WeightConstraintKind`'s C values are `-1, 0, 1`,
// not `0, 1, 2`: unlike `CheckMode`/`UndoMode`/`ClauseType` below, whose
// bindgen constants are `c_uint` (needing `c_int_of` to reach the `c_int`
// the functions actually take), `clingo_weight_constraint_type_e` is itself
// typed `c_int`, since bindgen gives a signed representation to an enum
// with a negative variant; its constants need no further conversion.

fn check_mode_to_raw(mode: CheckMode) -> c_int {
    c_int_of(match mode {
        CheckMode::Off => ffi::clingo_propagator_check_mode_none,
        CheckMode::Total => ffi::clingo_propagator_check_mode_total,
        CheckMode::Fixpoint => ffi::clingo_propagator_check_mode_fixpoint,
        CheckMode::Both => ffi::clingo_propagator_check_mode_both,
    })
}

fn check_mode_from_raw(raw: c_int) -> CheckMode {
    match c_uint::try_from(raw) {
        Ok(v) if v == ffi::clingo_propagator_check_mode_total => CheckMode::Total,
        Ok(v) if v == ffi::clingo_propagator_check_mode_fixpoint => CheckMode::Fixpoint,
        Ok(v) if v == ffi::clingo_propagator_check_mode_both => CheckMode::Both,
        other => {
            // clingox only ever writes a value it defined itself
            // (`set_check_mode`), so an unrecognised readback should not
            // happen; `Off` is the least surprising fallback (`none`'s own
            // C value, 0, matched too, above the catch-all).
            debug_assert!(
                other == Ok(ffi::clingo_propagator_check_mode_none),
                "clingo reported an unknown check mode: {raw}"
            );
            CheckMode::Off
        }
    }
}

fn undo_mode_to_raw(mode: UndoMode) -> c_int {
    c_int_of(match mode {
        UndoMode::Default => ffi::clingo_propagator_undo_mode_default,
        UndoMode::Always => ffi::clingo_propagator_undo_mode_always,
    })
}

fn undo_mode_from_raw(raw: c_int) -> UndoMode {
    match c_uint::try_from(raw) {
        Ok(v) if v == ffi::clingo_propagator_undo_mode_always => UndoMode::Always,
        other => {
            debug_assert!(
                other == Ok(ffi::clingo_propagator_undo_mode_default),
                "clingo reported an unknown undo mode: {raw}"
            );
            UndoMode::Default
        }
    }
}

/// clingo.h:197-203: `free`/`true`/`false`, mapped to `Option<bool>` (`None`
/// for `free`), the "`Option` for 'not this variant'" rule (RULES §4) rather
/// than a bespoke three-value enum (as for `Assignment::
/// truth_value`).
fn truth_value_from_raw(raw: c_int) -> Option<bool> {
    match c_uint::try_from(raw) {
        Ok(v) if v == ffi::clingo_truth_value_true => Some(true),
        Ok(v) if v == ffi::clingo_truth_value_false => Some(false),
        other => {
            debug_assert!(
                other == Ok(ffi::clingo_truth_value_free),
                "clingo reported an unknown truth value: {raw}"
            );
            None
        }
    }
}

/// Input only: no clingo function ever hands a `WeightConstraintKind` back.
fn weight_constraint_kind_to_raw(kind: WeightConstraintKind) -> c_int {
    // Unlike `CheckMode`/`UndoMode`/`ClauseType`'s bindgen constants,
    // `clingo_weight_constraint_type_e` is itself typed `c_int` (not
    // `c_uint`), since the enum has a negative variant
    // (`clingo_weight_constraint_type_implication_left = -1`); the constant
    // already has the right signed value, no `c_int_of`/cast needed.
    match kind {
        WeightConstraintKind::ImplicationLeft => {
            ffi::clingo_weight_constraint_type_implication_left
        }
        WeightConstraintKind::Equivalence => ffi::clingo_weight_constraint_type_equivalence,
        WeightConstraintKind::ImplicationRight => {
            ffi::clingo_weight_constraint_type_implication_right
        }
    }
}

/// Input only: no clingo function ever hands a `ClauseType` back.
fn clause_type_to_raw(kind: ClauseType) -> c_int {
    c_int_of(match kind {
        ClauseType::Learnt => ffi::clingo_clause_type_learnt,
        ClauseType::Static => ffi::clingo_clause_type_static,
        ClauseType::Volatile => ffi::clingo_clause_type_volatile,
        ClauseType::VolatileStatic => ffi::clingo_clause_type_volatile_static,
    })
}

// ---------------------------------------------------------------------------
// Init: the 19 `clingo_propagate_init_*` functions.

/// The raw `clingo_propagate_init_t` clingo hands a propagator's `init`
/// callback, borrowed for exactly the duration of that call.
pub(crate) struct Init<'i> {
    ptr: *mut ffi::clingo_propagate_init_t,
    _marker: PhantomData<&'i mut ffi::clingo_propagate_init_t>,
}

impl<'i> Init<'i> {
    /// # Safety
    ///
    /// `ptr` must be a valid `clingo_propagate_init_t` pointer, exclusively
    /// usable for `'i`: clingo's own contract for the `init` trampoline,
    /// which never runs concurrently with itself or with any other
    /// propagator callback (clingo.h:1483-1489).
    pub(crate) unsafe fn from_raw(ptr: *mut ffi::clingo_propagate_init_t) -> Init<'i> {
        Init {
            ptr,
            _marker: PhantomData,
        }
    }

    fn as_const(&self) -> *const ffi::clingo_propagate_init_t {
        self.ptr.cast_const()
    }

    /// clingo.h:1178-1187.
    pub(crate) fn solver_literal(&self, program_literal: i32) -> Result<SolverLiteral, Error> {
        let mut result = 0;
        // SAFETY: `self.ptr` is valid for `'i` (constructor's contract), and
        // `result` is a valid out-pointer for the call.
        query(|| unsafe {
            ffi::clingo_propagate_init_solver_literal(
                self.as_const(),
                program_literal,
                &raw mut result,
            )
        })?;
        Ok(SolverLiteral::from_raw_valid(result))
    }

    /// clingo.h:1189-1196.
    pub(crate) fn add_watch(&mut self, literal: SolverLiteral) -> Result<(), Error> {
        // SAFETY: `self.ptr` is valid for `'i` (constructor's contract).
        call(|| unsafe { ffi::clingo_propagate_init_add_watch(self.ptr, literal.get()) })
    }

    /// clingo.h:1198-1207.
    pub(crate) fn add_watch_to_thread(
        &mut self,
        literal: SolverLiteral,
        thread_id: u32,
    ) -> Result<(), Error> {
        // SAFETY: as `add_watch`.
        call(|| unsafe {
            ffi::clingo_propagate_init_add_watch_to_thread(self.ptr, literal.get(), thread_id)
        })
    }

    /// clingo.h:1209-1216.
    pub(crate) fn remove_watch(&mut self, literal: SolverLiteral) -> Result<(), Error> {
        // SAFETY: as `add_watch`.
        call(|| unsafe { ffi::clingo_propagate_init_remove_watch(self.ptr, literal.get()) })
    }

    /// clingo.h:1218-1227.
    pub(crate) fn remove_watch_from_thread(
        &mut self,
        literal: SolverLiteral,
        thread_id: u32,
    ) -> Result<(), Error> {
        // SAFETY: as `add_watch`.
        call(|| unsafe {
            ffi::clingo_propagate_init_remove_watch_from_thread(self.ptr, literal.get(), thread_id)
        })
    }

    /// clingo.h:1229-1240.
    pub(crate) fn freeze_literal(&mut self, literal: SolverLiteral) -> Result<(), Error> {
        // SAFETY: as `add_watch`.
        call(|| unsafe { ffi::clingo_propagate_init_freeze_literal(self.ptr, literal.get()) })
    }

    /// clingo.h:1242-1248. The returned view is only valid for `'i`: the
    /// header says symbolic atoms are unreachable "once the search has
    /// started," and `'i` is exactly the window before that.
    pub(crate) fn symbolic_atoms(&self) -> Result<super::Atoms<'i>, Error> {
        let mut object: *const ffi::clingo_symbolic_atoms_t = std::ptr::null();
        // SAFETY: `self.ptr` is valid for `'i` (constructor's contract), and
        // `object` is a valid out-pointer.
        query(|| unsafe {
            ffi::clingo_propagate_init_symbolic_atoms(self.as_const(), &raw mut object)
        })?;
        Ok(super::Atoms::from_raw(object))
    }

    /// clingo.h:1250-1256, same lifetime note as `symbolic_atoms`.
    pub(crate) fn theory_atoms(&self) -> Result<super::Theory<'i>, Error> {
        let mut object: *const ffi::clingo_theory_atoms_t = std::ptr::null();
        // SAFETY: as `symbolic_atoms`.
        query(|| unsafe {
            ffi::clingo_propagate_init_theory_atoms(self.as_const(), &raw mut object)
        })?;
        Ok(super::Theory::from_raw(object))
    }

    /// clingo.h:1258-1263: an `int`, never a success flag, and cannot fail.
    pub(crate) fn number_of_threads(&self) -> u32 {
        // SAFETY: `self.ptr` is valid for `'i` (constructor's contract).
        let n = unsafe { ffi::clingo_propagate_init_number_of_threads(self.as_const()) };
        u32::try_from(n).unwrap_or(0)
    }

    /// clingo.h:1270-1276: void, cannot fail.
    pub(crate) fn set_check_mode(&mut self, mode: CheckMode) {
        // SAFETY: as `number_of_threads`.
        unsafe { ffi::clingo_propagate_init_set_check_mode(self.ptr, check_mode_to_raw(mode)) };
    }

    /// clingo.h:1278-1282: cannot fail.
    pub(crate) fn check_mode(&self) -> CheckMode {
        // SAFETY: as `number_of_threads`.
        check_mode_from_raw(unsafe { ffi::clingo_propagate_init_get_check_mode(self.as_const()) })
    }

    /// clingo.h:1284-1290: void, cannot fail.
    pub(crate) fn set_undo_mode(&mut self, mode: UndoMode) {
        // SAFETY: as `number_of_threads`.
        unsafe { ffi::clingo_propagate_init_set_undo_mode(self.ptr, undo_mode_to_raw(mode)) };
    }

    /// clingo.h:1292-1296: cannot fail.
    pub(crate) fn undo_mode(&self) -> UndoMode {
        // SAFETY: as `number_of_threads`.
        undo_mode_from_raw(unsafe { ffi::clingo_propagate_init_get_undo_mode(self.as_const()) })
    }

    /// clingo.h:1298-1302: returns the pointer directly, cannot fail.
    pub(crate) fn assignment(&self) -> Assignment<'_> {
        // SAFETY: as `number_of_threads`; clingo returns a valid
        // `clingo_assignment_t` pointer here whenever `self` is, readable
        // for as long as `self` is borrowed.
        unsafe { assignment_from_raw(ffi::clingo_propagate_init_assignment(self.as_const())) }
    }

    /// clingo.h:1304-1319.
    pub(crate) fn add_literal(&mut self, freeze: bool) -> Result<SolverLiteral, Error> {
        let mut result = 0;
        // SAFETY: `self.ptr` is valid for `'i` (constructor's contract), and
        // `result` is a valid out-pointer.
        call(|| unsafe {
            ffi::clingo_propagate_init_add_literal(self.ptr, freeze, &raw mut result)
        })?;
        Ok(SolverLiteral::from_raw_valid(result))
    }

    /// clingo.h:1321-1334.
    pub(crate) fn add_clause(&mut self, clause: &[SolverLiteral]) -> Result<Flow, Error> {
        let raw_clause: Vec<i32> = clause.iter().map(|l| l.get()).collect();
        let mut result = true;
        // SAFETY: `self.ptr` is valid for `'i` (constructor's contract);
        // `raw_clause` holds `raw_clause.len()` literals and outlives the
        // call, and `result` is a valid out-pointer.
        call(|| unsafe {
            ffi::clingo_propagate_init_add_clause(
                self.ptr,
                raw_clause.as_ptr(),
                raw_clause.len(),
                &raw mut result,
            )
        })?;
        Ok(Flow::from_continue(result))
    }

    /// clingo.h:1336-1358. `weight`/`bound` are `i32` (`clingo_weight_t` is
    /// `int32_t`, `clingox-sys/src/bindings.rs:13`), used for both the
    /// weighted literals and the bound: an `i64` would
    /// disagree with the header (the same holds for `add_minimize`).
    pub(crate) fn add_weight_constraint(
        &mut self,
        literal: SolverLiteral,
        literals: &[(SolverLiteral, i32)],
        bound: i32,
        kind: WeightConstraintKind,
        compare_equal: bool,
    ) -> Result<Flow, Error> {
        let raw_literals: Vec<ffi::clingo_weighted_literal_t> = literals
            .iter()
            .map(|&(lit, weight)| weighted_literal(lit.get(), weight))
            .collect();
        let mut result = true;
        // SAFETY: `self.ptr` is valid for `'i` (constructor's contract);
        // `raw_literals` holds `raw_literals.len()` weighted literals and
        // outlives the call, and `result` is a valid out-pointer.
        call(|| unsafe {
            ffi::clingo_propagate_init_add_weight_constraint(
                self.ptr,
                literal.get(),
                raw_literals.as_ptr(),
                raw_literals.len(),
                bound,
                weight_constraint_kind_to_raw(kind),
                compare_equal,
                &raw mut result,
            )
        })?;
        Ok(Flow::from_continue(result))
    }

    /// clingo.h:1360-1369; `weight`/`priority` are `i32`.
    pub(crate) fn add_minimize(
        &mut self,
        literal: SolverLiteral,
        weight: i32,
        priority: i32,
    ) -> Result<(), Error> {
        // SAFETY: `self.ptr` is valid for `'i` (constructor's contract).
        call(|| unsafe {
            ffi::clingo_propagate_init_add_minimize(self.ptr, literal.get(), weight, priority)
        })
    }

    /// clingo.h:1371-1380.
    pub(crate) fn propagate(&mut self) -> Result<Flow, Error> {
        let mut result = true;
        // SAFETY: `self.ptr` is valid for `'i` (constructor's contract), and
        // `result` is a valid out-pointer.
        call(|| unsafe { ffi::clingo_propagate_init_propagate(self.ptr, &raw mut result) })?;
        Ok(Flow::from_continue(result))
    }
}

// ---------------------------------------------------------------------------
// PropagateControl: all eight `clingo_propagate_control_*` functions
// (`thread_id`, `assignment`, `add_clause`, `add_literal`,
// `add_watch`, `has_watch`, `remove_watch` and `propagate`).

/// The raw `clingo_propagate_control_t` clingo hands `propagate`/`undo`/
/// `check`, borrowed for exactly the duration of that call.
///
/// Stored as `*mut` regardless of which callback it came from: `undo`
/// receives a `clingo_propagate_control_t const *` at the C level
/// (clingo.h:1541-1542), cast back to a mutable pointer here purely as an
/// internal representation choice (the object behind both pointer kinds is
/// the same C++ `ClingoPropagator::Control`, `clasp/src/clingo.cpp`); the
/// actual "no `add_clause` from `undo`" guarantee comes from the *safe*
/// layer's `&self`/`&mut self` split on [`PropagateControl`], never from
/// this pointer's constness.
pub(crate) struct RawPropagateControl<'c> {
    ptr: *mut ffi::clingo_propagate_control_t,
    _marker: PhantomData<&'c mut ffi::clingo_propagate_control_t>,
}

impl<'c> RawPropagateControl<'c> {
    /// # Safety
    ///
    /// `ptr` must be a valid `clingo_propagate_control_t` pointer, usable
    /// for `'c`.
    unsafe fn from_raw(ptr: *mut ffi::clingo_propagate_control_t) -> RawPropagateControl<'c> {
        RawPropagateControl {
            ptr,
            _marker: PhantomData,
        }
    }

    /// # Safety
    ///
    /// As [`RawPropagateControl::from_raw`], for a pointer clingo gave as
    /// `const` (the `undo` callback).
    unsafe fn from_raw_const(
        ptr: *const ffi::clingo_propagate_control_t,
    ) -> RawPropagateControl<'c> {
        // SAFETY: the caller's contract; see the type's own doc comment for
        // why casting away constness here is sound.
        unsafe { RawPropagateControl::from_raw(ptr.cast_mut()) }
    }

    /// clingo.h:1382-1389: an id, cannot fail.
    pub(crate) fn thread_id(&self) -> u32 {
        // SAFETY: `self.ptr` is valid for `'c` (constructor's contract).
        unsafe { ffi::clingo_propagate_control_thread_id(self.ptr.cast_const()) }
    }

    /// clingo.h:1391-1396: returns the pointer directly, cannot fail.
    pub(crate) fn assignment(&self) -> Assignment<'_> {
        // SAFETY: as `thread_id`; clingo returns a valid
        // `clingo_assignment_t` pointer here whenever `self` is, readable
        // while `self` is borrowed.
        unsafe {
            assignment_from_raw(ffi::clingo_propagate_control_assignment(
                self.ptr.cast_const(),
            ))
        }
    }

    /// clingo.h:1430-1445.
    pub(crate) fn add_clause(
        &mut self,
        clause: &[SolverLiteral],
        kind: ClauseType,
    ) -> Result<Flow, Error> {
        let raw_clause: Vec<i32> = clause.iter().map(|l| l.get()).collect();
        let mut result = true;
        // SAFETY: `self.ptr` is valid for `'c` (constructor's contract);
        // `raw_clause` holds `raw_clause.len()` literals and outlives the
        // call, and `result` is a valid out-pointer.
        call(|| unsafe {
            ffi::clingo_propagate_control_add_clause(
                self.ptr,
                raw_clause.as_ptr(),
                raw_clause.len(),
                clause_type_to_raw(kind),
                &raw mut result,
            )
        })?;
        Ok(Flow::from_continue(result))
    }

    /// clingo.h:1611-1625: a fresh, volatile solver literal, valid only for
    /// the current solving step and solver thread.
    pub(crate) fn add_literal(&mut self) -> Result<SolverLiteral, Error> {
        let mut result = 0;
        // SAFETY: `self.ptr` is valid for `'c` (constructor's contract), and
        // `result` is a valid out-pointer for the call.
        call(|| unsafe { ffi::clingo_propagate_control_add_literal(self.ptr, &raw mut result) })?;
        Ok(SolverLiteral::from_raw_valid(result))
    }

    /// clingo.h:1627-1642: watches `literal` on the current solver thread
    /// only, unlike [`Init::add_watch`], which by default watches on every
    /// thread.
    pub(crate) fn add_watch(&mut self, literal: SolverLiteral) -> Result<(), Error> {
        // SAFETY: `self.ptr` is valid for `'c` (constructor's contract).
        call(|| unsafe { ffi::clingo_propagate_control_add_watch(self.ptr, literal.get()) })
    }

    /// clingo.h:1644-1654: `bool`-returning directly ("whether the literal
    /// is watched"), never a success flag; cannot fail.
    pub(crate) fn has_watch(&self, literal: SolverLiteral) -> bool {
        // SAFETY: `self.ptr` is valid for `'c` (constructor's contract).
        unsafe { ffi::clingo_propagate_control_has_watch(self.ptr.cast_const(), literal.get()) }
    }

    /// clingo.h:1656-1666: `void`, cannot fail; a conditional no-op for an
    /// unwatched literal.
    pub(crate) fn remove_watch(&mut self, literal: SolverLiteral) {
        // SAFETY: `self.ptr` is valid for `'c` (constructor's contract).
        unsafe { ffi::clingo_propagate_control_remove_watch(self.ptr, literal.get()) };
    }

    /// clingo.h:1691-1706: propagates the consequences of the clauses added
    /// so far, before the outer `propagate`/`undo`/`check` call returns.
    pub(crate) fn propagate(&mut self) -> Result<Flow, Error> {
        let mut result = true;
        // SAFETY: `self.ptr` is valid for `'c` (constructor's contract), and
        // `result` is a valid out-pointer for the call.
        call(|| unsafe { ffi::clingo_propagate_control_propagate(self.ptr, &raw mut result) })?;
        Ok(Flow::from_continue(result))
    }
}

// ---------------------------------------------------------------------------
// Registration and dispatch.

/// Which callback recorded a `PropagatorContext`'s stored error or panic: only
/// `init`'s own poisons the whole `Control` unconditionally;
/// `propagate`/`undo`/`check`/`decide`'s never does, whatever an error's kind
/// or a panic.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Init,
    Callback,
}

/// As [`raw::trampoline::guard`], but also tags a caught panic with which
/// callback recorded it: only `init`'s own panic should poison the control;
/// `propagate`/`undo`/`check`/`decide`'s should not, and this
/// is how [`PropagatorSlots::take_panic`] tells
/// [`Control::settle_propagators`](crate::control::Control) which case it
/// is. Otherwise identical to `guard`: first-writer-wins against `slot`, a
/// caught panic never unwinds past this call.
fn guard_tagged<R>(
    slot: &Slot<(Origin, Box<dyn Any + Send>)>,
    origin: Origin,
    f: impl FnOnce() -> R,
) -> Option<R> {
    if slot.is_set() {
        return None;
    }
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => Some(value),
        Err(payload) => {
            slot.store((origin, payload));
            None
        }
    }
}

/// What the data pointer of a registered propagator points to (DESIGN S8,
/// S10, S11): the propagator itself (called through `&P`, never `&mut P`,
/// per S11: clasp can call `undo` re-entrantly on the same thread while
/// `propagate` is still on the call stack, and can call `propagate` for two
/// different threads on the same `&self` at once), and one error/panic slot
/// pair shared by all five trampolines. A failure on any one of them must
/// stop every later call on this same propagator at once (S8); a *different*
/// registered propagator has its own, independent context and is
/// unaffected (each propagator's error slot is its own).
struct PropagatorContext<P> {
    propagator: P,
    error: Slot<(Origin, Error)>,
    panic: Slot<(Origin, Box<dyn Any + Send>)>,
}

impl<P: Propagator> PropagatorContext<P> {
    fn new(propagator: P) -> Self {
        PropagatorContext {
            propagator,
            error: Slot::default(),
            panic: Slot::default(),
        }
    }

    /// Runs one of the four `Result`-returning callbacks (S8): refuses at
    /// once if an earlier callback on this same propagator already failed,
    /// otherwise runs `body` inside [`guard_tagged`], recording a returned
    /// error or a caught panic, both tagged with `origin` (for the
    /// poisoning decision above). `None` means clingo should see `false`.
    fn run<R>(&self, origin: Origin, body: impl FnOnce(&P) -> Result<R, Error>) -> Option<R> {
        if self.error.is_set() {
            return None;
        }
        match guard_tagged(&self.panic, origin, || body(&self.propagator)) {
            Some(Ok(value)) => Some(value),
            Some(Err(err)) => {
                self.error.store((origin, err));
                None
            }
            None => None,
        }
    }

    /// As [`PropagatorContext::run`], for `undo`, which cannot report a failure
    /// at all (S9): a panic is still caught and recorded (tagged
    /// [`Origin::Callback`], `undo`'s own panic never poisons either), and the
    /// next callback on this propagator that *can* report failure does so.
    fn run_void(&self, origin: Origin, body: impl FnOnce(&P)) {
        if self.error.is_set() {
            return;
        }
        let _ = guard_tagged(&self.panic, origin, || body(&self.propagator));
    }
}

/// A type-erased handle onto a registered propagator's error and panic
/// slots, so [`ControlHandle`] can check every registered propagator
/// without naming its concrete `P` (mirrors `raw::observer::ObserverSlots`).
pub(super) trait PropagatorSlots {
    /// Takes the recorded error, if any (first writer wins, S8), already marked
    /// how it poisons: unconditionally when it came from `init`, never,
    /// whatever its kind, when it came from
    /// `propagate`/`undo`/`check`/`decide`. The safe layer only needs to call
    /// `Control::note` on it, exactly as it does for any other error.
    fn take_error(&self) -> Option<Error>;
    /// Takes the recorded panic payload, if any, together with whether it
    /// should poison the control on its own: `true` only when it came from
    /// `init`; `propagate`/`undo`/`check`/`decide`'s panic
    /// still resumes but never poisons.
    fn take_panic(&self) -> Option<(bool, Box<dyn Any + Send>)>;
}

impl<P: Propagator> PropagatorSlots for PropagatorContext<P> {
    fn take_error(&self) -> Option<Error> {
        self.error.take().map(|(origin, err)| match origin {
            Origin::Init => err.poisoning(),
            Origin::Callback => err.excused(),
        })
    }

    fn take_panic(&self) -> Option<(bool, Box<dyn Any + Send>)> {
        self.panic
            .take()
            .map(|(origin, payload)| (origin == Origin::Init, payload))
    }
}

/// Reads the boxed [`PropagatorContext`] a trampoline's `data` pointer
/// names; every one of the five trampolines below starts with this same
/// cast.
///
/// # Safety
///
/// `data` must point to a `PropagatorContext<P>` that is valid for `'a`:
/// `ControlHandle::register_propagator` boxes it and keeps it in
/// `ControlHandle::propagators` until the control itself is dropped, after
/// `clingo_control_free` (clingo may call it up to that point).
unsafe fn context_of<'a, P: Propagator>(data: *mut c_void) -> &'a PropagatorContext<P> {
    // SAFETY: the caller's contract above.
    unsafe { &*data.cast::<PropagatorContext<P>>().cast_const() }
}

unsafe extern "C" fn init<P: Propagator, S: ErrorState>(
    init: *mut ffi::clingo_propagate_init_t,
    data: *mut c_void,
) -> bool {
    // SAFETY: as `context_of`'s contract; `init` is the pointer clingo
    // passed for this call, valid and exclusively usable for its duration
    // (clingo.h:1483-1489, control.cc:2088-2094).
    let context = unsafe { context_of::<P>(data) };
    // SAFETY: as above.
    let mut wrapper = PropagateInit::from_raw(unsafe { Init::from_raw(init) });
    if let Some(()) = context.run(Origin::Init, |p| p.init(&mut wrapper)) {
        true
    } else {
        fail::<S>(c"a propagator's init failed");
        false
    }
}

unsafe extern "C" fn propagate<P: Propagator, S: ErrorState>(
    control: *mut ffi::clingo_propagate_control_t,
    changes: *const ffi::clingo_literal_t,
    size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as `init`; `control` and `data` are the pointers clingo passed
    // for this call (clingo.h:1468-1470, control.cc:2096-2100). `changes` is
    // `size` valid `clingo_literal_t`s for the call; `SolverLiteral` is
    // `#[repr(transparent)]` over it.
    let context = unsafe { context_of::<P>(data) };
    // SAFETY: as above.
    let mut wrapper = PropagateControl::from_raw(unsafe { RawPropagateControl::from_raw(control) });
    // SAFETY: as above.
    let changes = unsafe { raw_slice(changes.cast::<SolverLiteral>(), size) };
    if let Some(()) = context.run(Origin::Callback, |p| p.propagate(&mut wrapper, changes)) {
        true
    } else {
        fail::<S>(c"a propagator's propagate failed");
        false
    }
}

unsafe extern "C" fn undo<P: Propagator>(
    control: *const ffi::clingo_propagate_control_t,
    changes: *const ffi::clingo_literal_t,
    size: usize,
    data: *mut c_void,
) {
    // SAFETY: as `propagate`, except `control` is clingo's own `const`
    // pointer for this callback (clingo.h:1472-1473, control.cc:2102-2106).
    let context = unsafe { context_of::<P>(data) };
    // SAFETY: as above.
    let wrapper =
        PropagateControl::from_raw(unsafe { RawPropagateControl::from_raw_const(control) });
    // SAFETY: as above.
    let changes = unsafe { raw_slice(changes.cast::<SolverLiteral>(), size) };
    // `undo` is void at the C level (S9): nothing here can report a failure
    // to clingo; a panic is still caught and resumed later, by whichever
    // callback on this propagator runs next and can report one. Tagged
    // `Origin::Callback`: `undo`'s own panic never poisons either.
    context.run_void(Origin::Callback, |p| p.undo(&wrapper, changes));
}

unsafe extern "C" fn check<P: Propagator, S: ErrorState>(
    control: *mut ffi::clingo_propagate_control_t,
    data: *mut c_void,
) -> bool {
    // SAFETY: as `propagate` (clingo.h:1476, control.cc:2108-2112).
    let context = unsafe { context_of::<P>(data) };
    // SAFETY: as above.
    let mut wrapper = PropagateControl::from_raw(unsafe { RawPropagateControl::from_raw(control) });
    if let Some(()) = context.run(Origin::Callback, |p| p.check(&mut wrapper)) {
        true
    } else {
        fail::<S>(c"a propagator's check failed");
        false
    }
}

unsafe extern "C" fn decide<P: Propagator, S: ErrorState>(
    thread_id: ffi::clingo_id_t,
    assignment: *const ffi::clingo_assignment_t,
    fallback: ffi::clingo_literal_t,
    data: *mut c_void,
    decision: *mut ffi::clingo_literal_t,
) -> bool {
    // SAFETY: as `init`'s contract for `data`; `assignment` is a valid
    // `clingo_assignment_t` pointer for the call, and `decision` a valid
    // out-pointer (clingo.h:1567-1575, control.cc:2114-2123).
    let context = unsafe { context_of::<P>(data) };
    // SAFETY: as above.
    let assignment = unsafe { assignment_from_raw(assignment) };
    let fallback = SolverLiteral::from_raw_valid(fallback);
    if let Some(raw) = context.run(Origin::Callback, |p| {
        match p.decide(thread_id, &assignment, fallback)? {
            // The literal comes back from user code, not from clingo, so
            // it needs the same validation as any literal clingox hands
            // to clingo on the way in: an unchecked one
            // reaches clingo.h:1567-1575's `*decision` out-pointer, the
            // same hazard already closed for `PropagateInit`/
            // `PropagateControl`'s own literal-taking methods.
            Some(chosen) if assignment.has_literal(chosen) => Ok(chosen.get()),
            Some(chosen) => Err(foreign_literal(chosen)),
            // `None` is a real decline: clingo's own documented contract for
            // `decide` is "return 0 to let a propagator registered later make a
            // decision" (H:1567-1568), so `None` must write the raw `0` clingo
            // checks for, never `fallback`'s own value: writing `fallback`
            // unchanged is indistinguishable, at the C level, from a deliberate
            // choice of that literal, and silently blocks every
            // later-registered propagator's own `decide` from ever being asked
            // (the earlier defect).
            None => Ok(0),
        }
    }) {
        // SAFETY: `decision` is the valid out-pointer clingo passed for
        // this call.
        unsafe { *decision = raw };
        true
    } else {
        fail::<S>(c"a propagator's decide failed");
        false
    }
}

/// The C struct clingo copies into its own state at registration
/// (`clingo_control_register_propagator` copies `*propagator` by value,
/// `control.cc:2136-2140`), one instance per concrete `P`.
fn propagator_struct<P: Propagator>() -> ffi::clingo_propagator_t {
    ffi::clingo_propagator_t {
        init: Some(init::<P, super::ClingoErrorState>),
        propagate: Some(propagate::<P, super::ClingoErrorState>),
        undo: Some(undo::<P>),
        check: Some(check::<P, super::ClingoErrorState>),
        decide: Some(decide::<P, super::ClingoErrorState>),
    }
}

impl ControlHandle {
    /// Registers a propagator (clingo.h:3159-3175): boxes it (with its error
    /// and panic slots) once, hands clingo the C struct and the box's address,
    /// and, on success, keeps the box in [`ControlHandle::propagators`] for the
    /// rest of the control's life (S4, S8, S10, S11), exactly as
    /// [`ControlHandle::register_observer`] does for an observer. clingo runs
    /// every registered propagator, in registration order (checked directly
    /// against pyclingo 5.8.2), and never removes a registration on its own.
    pub(crate) fn register_propagator<P: Propagator + 'static>(
        &mut self,
        propagator: P,
        sequential: bool,
    ) -> Result<(), Error> {
        let context = Box::new(PropagatorContext::new(propagator));
        let data = std::ptr::from_ref::<PropagatorContext<P>>(&context)
            .cast_mut()
            .cast::<c_void>();
        let raw_propagator = propagator_struct::<P>();
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and no
            // search is open (`logged` closed it). `raw_propagator` is a
            // local clingo copies into its own state during this call
            // (control.cc:2136-2140); `data` points to `context`'s heap
            // allocation, which is moved (not reallocated) into
            // `self.propagators` right after this call succeeds, and kept
            // there until `clingo_control_free` (clingo.h:3159-3175).
            unsafe {
                ffi::clingo_control_register_propagator(
                    control,
                    &raw const raw_propagator,
                    data,
                    sequential,
                )
            }
        })?;
        self.propagators
            .push(context as Box<dyn PropagatorSlots + Send + Sync>);
        Ok(())
    }

    /// After a call that might have run a propagator callback, the first
    /// recorded panic among every registered propagator, if any (S8: only one
    /// can ever be set on any one propagator, since clingo stops calling any of
    /// its five trampolines once one returns `false`/records a panic, but every
    /// propagator is checked defensively, mirroring
    /// [`ControlHandle::take_observer_panic`]), together with whether it should
    /// poison the control on its own (`true` only for `init`'s own panic).
    pub(crate) fn take_propagator_panic(&self) -> Option<(bool, Box<dyn Any + Send>)> {
        self.propagators.iter().find_map(|p| p.take_panic())
    }

    /// As [`ControlHandle::take_propagator_panic`], for a returned error
    /// (already marked to poison unconditionally when it came from `init`,
    /// [`PropagatorSlots::take_error`]).
    pub(crate) fn take_propagator_error(&self) -> Option<Error> {
        self.propagators.iter().find_map(|p| p.take_error())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::CStr;
    use std::sync::Mutex;

    use super::*;

    // The `clingo_truth_value_t` conversion, a pure function of a
    // `c_int`, testable without a real assignment (mirrors `check_mode_
    // from_raw`/`undo_mode_from_raw`'s own shape above, neither of which
    // has a dedicated unit test either; this one is
    // added because logic testable without clingo gets a Miri unit test).

    #[test]
    fn truth_value_from_raw_maps_the_three_variants() {
        assert_eq!(
            truth_value_from_raw(c_int_of(ffi::clingo_truth_value_true)),
            Some(true)
        );
        assert_eq!(
            truth_value_from_raw(c_int_of(ffi::clingo_truth_value_false)),
            Some(false)
        );
        assert_eq!(
            truth_value_from_raw(c_int_of(ffi::clingo_truth_value_free)),
            None
        );
    }
    use crate::error::ErrorKind;
    use crate::propagate::{Assignment as SafeAssignment, PropagateInit as SafePropagateInit};

    thread_local! {
        static SET: Cell<Option<c_uint>> = const { Cell::new(None) };
    }

    enum Recorded {}

    impl ErrorState for Recorded {
        fn set(code: c_uint, _: &CStr) {
            SET.with(|s| s.set(Some(code)));
        }
        fn code() -> c_int {
            0
        }
        fn message() -> Option<String> {
            None
        }
    }

    /// A propagator whose every method is controllable from the test.
    #[derive(Default)]
    #[expect(
        clippy::struct_excessive_bools,
        reason = "one independent panic switch per dispatched callback, not related flags"
    )]
    struct Scripted {
        init_result: Mutex<Option<Result<(), Error>>>,
        init_panics: bool,
        propagate_result: Mutex<Option<Result<(), Error>>>,
        propagate_panics: bool,
        check_result: Mutex<Option<Result<(), Error>>>,
        check_panics: bool,
        undo_panics: bool,
        undo_calls: Mutex<u32>,
        decide_result: Mutex<Option<Result<Option<SolverLiteral>, Error>>>,
        decide_panics: bool,
    }

    impl Propagator for Scripted {
        fn init(&self, _init: &mut SafePropagateInit<'_>) -> crate::error::Result<()> {
            assert!(!self.init_panics, "init panics on purpose");
            self.init_result.lock().unwrap().take().unwrap_or(Ok(()))
        }

        fn propagate(
            &self,
            _control: &mut PropagateControl<'_>,
            _changes: &[SolverLiteral],
        ) -> crate::error::Result<()> {
            assert!(!self.propagate_panics, "propagate panics on purpose");
            self.propagate_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(()))
        }

        fn undo(&self, _control: &PropagateControl<'_>, _changes: &[SolverLiteral]) {
            *self.undo_calls.lock().unwrap() += 1;
            assert!(!self.undo_panics, "undo panics on purpose");
        }

        fn check(&self, _control: &mut PropagateControl<'_>) -> crate::error::Result<()> {
            assert!(!self.check_panics, "check panics on purpose");
            self.check_result.lock().unwrap().take().unwrap_or(Ok(()))
        }

        fn decide(
            &self,
            _thread_id: u32,
            _assignment: &SafeAssignment<'_>,
            _fallback: SolverLiteral,
        ) -> crate::error::Result<Option<SolverLiteral>> {
            assert!(!self.decide_panics, "decide panics on purpose");
            self.decide_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(None))
        }
    }

    fn evaluate_init(context: &PropagatorContext<Scripted>) -> bool {
        SET.with(|s| s.set(None));
        // SAFETY: `context` outlives the call, and a null
        // `clingo_propagate_init_t` pointer is never dereferenced by
        // `Scripted::init` above (it never touches `init`).
        unsafe {
            init::<Scripted, Recorded>(
                std::ptr::null_mut(),
                std::ptr::from_ref(context).cast_mut().cast(),
            )
        }
    }

    fn evaluate_propagate(context: &PropagatorContext<Scripted>) -> bool {
        SET.with(|s| s.set(None));
        // SAFETY: `context` outlives the call; a null
        // `clingo_propagate_control_t` pointer and an empty change list are
        // never dereferenced by `Scripted::propagate` above (it never touches
        // `control` or `changes`).
        unsafe {
            propagate::<Scripted, Recorded>(
                std::ptr::null_mut(),
                std::ptr::null(),
                0,
                std::ptr::from_ref(context).cast_mut().cast(),
            )
        }
    }

    fn evaluate_check(context: &PropagatorContext<Scripted>) -> bool {
        SET.with(|s| s.set(None));
        // SAFETY: as `evaluate_propagate`; `Scripted::check` never touches
        // `control` either.
        unsafe {
            check::<Scripted, Recorded>(
                std::ptr::null_mut(),
                std::ptr::from_ref(context).cast_mut().cast(),
            )
        }
    }

    /// Only the `Err`/panic paths of `decide` are Miri-testable this way: a
    /// successful `decide` now validates its returned literal against
    /// `clingo_assignment_has_literal`, a real C call Miri cannot make and a
    /// fake pointer cannot safely stand in for, so `decide_ok_...` is not among
    /// these tests; `clingox/tests/api_propagator_decide.rs` covers that path
    /// against real clingo instead. A `null` assignment pointer is safe here
    /// only because `Err`/a panic from `Scripted::decide` short-circuits before
    /// the trampoline's own validation step ever runs.
    fn evaluate_decide(
        context: &PropagatorContext<Scripted>,
        decision: &mut ffi::clingo_literal_t,
    ) -> bool {
        SET.with(|s| s.set(None));
        // SAFETY: `context` and `decision` outlive the call. `assignment` is
        // null, never dereferenced by `Scripted::decide` (which never
        // touches it) nor, on the `Err`/panic paths under test here, by the
        // trampoline's own validation step (see the doc comment above).
        unsafe {
            decide::<Scripted, Recorded>(
                0,
                std::ptr::null(),
                0,
                std::ptr::from_ref(context).cast_mut().cast(),
                decision,
            )
        }
    }

    #[test]
    fn init_ok_reports_success() {
        let context = PropagatorContext::new(Scripted::default());
        assert!(evaluate_init(&context));
        assert_eq!(SET.with(Cell::get), None);
        assert!(!context.error.is_set());
    }

    #[test]
    fn init_err_is_stored_marked_to_poison_and_stops_later_calls() {
        let context = PropagatorContext::new(Scripted::default());
        *context.propagator.init_result.lock().unwrap() =
            Some(Err(Error::new(ErrorKind::Conversion, "no")));
        assert!(!evaluate_init(&context));
        assert_eq!(SET.with(Cell::get), Some(ffi::clingo_error_unknown));
        assert!(!evaluate_init(&context), "the slot stays set");

        let slots: &dyn PropagatorSlots = &context;
        let err = slots.take_error().unwrap();
        assert_eq!(err.kind(), ErrorKind::Conversion, "kind is unchanged");
        assert_eq!(
            *err.poison(),
            crate::error::Poison::Always,
            "init's own error poisons unconditionally"
        );
    }

    #[test]
    fn init_panic_is_caught_and_recorded() {
        let context = PropagatorContext::new(Scripted {
            init_panics: true,
            ..Scripted::default()
        });
        assert!(!evaluate_init(&context));
        let slots: &dyn PropagatorSlots = &context;
        let (poisons, payload) = slots.take_panic().unwrap();
        assert!(poisons, "init's own panic poisons unconditionally");
        assert_eq!(
            *payload.downcast::<&str>().unwrap(),
            "init panics on purpose"
        );
    }

    #[test]
    fn propagate_ok_reports_success() {
        let context = PropagatorContext::new(Scripted::default());
        assert!(evaluate_propagate(&context));
        assert_eq!(SET.with(Cell::get), None);
        assert!(!context.error.is_set());
    }

    #[test]
    fn propagate_err_is_stored_and_does_not_poison() {
        let context = PropagatorContext::new(Scripted::default());
        *context.propagator.propagate_result.lock().unwrap() =
            Some(Err(Error::new(ErrorKind::Conversion, "no")));
        assert!(!evaluate_propagate(&context));
        assert_eq!(SET.with(Cell::get), Some(ffi::clingo_error_unknown));
        assert!(!evaluate_propagate(&context), "the slot stays set");

        let slots: &dyn PropagatorSlots = &context;
        let err = slots.take_error().unwrap();
        assert_eq!(err.kind(), ErrorKind::Conversion, "kind is unchanged");
        assert_eq!(
            *err.poison(),
            crate::error::Poison::Never,
            "propagate's own error never poisons, whatever its kind"
        );
    }

    #[test]
    fn propagate_panic_is_caught_and_recorded() {
        let context = PropagatorContext::new(Scripted {
            propagate_panics: true,
            ..Scripted::default()
        });
        assert!(!evaluate_propagate(&context));
        let slots: &dyn PropagatorSlots = &context;
        let (poisons, payload) = slots.take_panic().unwrap();
        assert!(!poisons, "propagate's own panic does not poison either");
        assert_eq!(
            *payload.downcast::<&str>().unwrap(),
            "propagate panics on purpose"
        );
    }

    #[test]
    fn check_ok_reports_success() {
        let context = PropagatorContext::new(Scripted::default());
        assert!(evaluate_check(&context));
        assert_eq!(SET.with(Cell::get), None);
        assert!(!context.error.is_set());
    }

    #[test]
    fn check_err_is_stored_and_does_not_poison() {
        let context = PropagatorContext::new(Scripted::default());
        *context.propagator.check_result.lock().unwrap() =
            Some(Err(Error::new(ErrorKind::Conversion, "no")));
        assert!(!evaluate_check(&context));
        assert_eq!(SET.with(Cell::get), Some(ffi::clingo_error_unknown));
        assert!(!evaluate_check(&context), "the slot stays set");

        let slots: &dyn PropagatorSlots = &context;
        let err = slots.take_error().unwrap();
        assert_eq!(err.kind(), ErrorKind::Conversion, "kind is unchanged");
        assert_eq!(
            *err.poison(),
            crate::error::Poison::Never,
            "check's own error never poisons, whatever its kind"
        );
    }

    #[test]
    fn check_panic_is_caught_and_recorded() {
        let context = PropagatorContext::new(Scripted {
            check_panics: true,
            ..Scripted::default()
        });
        assert!(!evaluate_check(&context));
        let slots: &dyn PropagatorSlots = &context;
        let (poisons, payload) = slots.take_panic().unwrap();
        assert!(!poisons, "check's own panic does not poison either");
        assert_eq!(
            *payload.downcast::<&str>().unwrap(),
            "check panics on purpose"
        );
    }

    #[test]
    fn decide_err_is_stored_and_the_out_pointer_is_left_untouched() {
        let context = PropagatorContext::new(Scripted::default());
        *context.propagator.decide_result.lock().unwrap() =
            Some(Err(Error::new(ErrorKind::Conversion, "no")));
        let mut decision = -1;
        assert!(!evaluate_decide(&context, &mut decision));
        assert_eq!(SET.with(Cell::get), Some(ffi::clingo_error_unknown));
        assert_eq!(decision, -1, "the out-pointer is untouched on failure");

        let slots: &dyn PropagatorSlots = &context;
        let err = slots.take_error().unwrap();
        assert_eq!(err.kind(), ErrorKind::Conversion, "kind is unchanged");
        assert_eq!(
            *err.poison(),
            crate::error::Poison::Never,
            "decide's own error never poisons, whatever its kind"
        );
    }

    #[test]
    fn decide_panic_is_caught_and_recorded() {
        let context = PropagatorContext::new(Scripted {
            decide_panics: true,
            ..Scripted::default()
        });
        let mut decision = -1;
        assert!(!evaluate_decide(&context, &mut decision));
        let slots: &dyn PropagatorSlots = &context;
        let (poisons, payload) = slots.take_panic().unwrap();
        assert!(!poisons, "decide's own panic does not poison either");
        assert_eq!(
            *payload.downcast::<&str>().unwrap(),
            "decide panics on purpose"
        );
    }

    #[test]
    fn undo_is_void_and_a_panic_is_recorded_not_propagated_here() {
        let context = PropagatorContext::new(Scripted {
            undo_panics: true,
            ..Scripted::default()
        });
        // SAFETY: a null `clingo_propagate_control_t const *` is never
        // dereferenced by `Scripted::undo` above, and an empty change list
        // is a valid null/zero-length slice for `raw_slice`.
        unsafe {
            undo::<Scripted>(
                std::ptr::null(),
                std::ptr::null(),
                0,
                std::ptr::from_ref(&context).cast_mut().cast(),
            );
        }
        assert_eq!(*context.propagator.undo_calls.lock().unwrap(), 1);
        let slots: &dyn PropagatorSlots = &context;
        let (poisons, payload) = slots.take_panic().unwrap();
        assert!(!poisons, "undo's own panic does not poison either");
        assert_eq!(
            *payload.downcast::<&str>().unwrap(),
            "undo panics on purpose"
        );
    }

    // The next two tests pin that each propagator has its own error/panic slot
    // (not a shared slot across every registered propagator): that difference
    // cannot be observed from an integration test, since clasp itself aborts
    // the whole `init` phase at the first propagator's failure regardless of
    // whether clingox's own slots are shared or separate; a later
    // propagator's `init` never runs either way, so the *effect* a real solve
    // can see is identical under both implementations. Only a unit test that
    // calls the trampolines directly on two distinct `PropagatorContext`s,
    // bypassing clasp's own control flow entirely, can actually tell them
    // apart: if `PropagatorContext::error`/`panic` were ever replaced by one
    // shared pair of slots for every registered propagator, `sibling`'s own
    // slot would already be set by `failing`'s failure, and these assertions
    // would fail.

    #[test]
    fn a_failure_on_one_propagator_never_touches_a_sibling() {
        let failing = PropagatorContext::new(Scripted::default());
        *failing.propagator.init_result.lock().unwrap() =
            Some(Err(Error::new(ErrorKind::Conversion, "no")));
        let sibling = PropagatorContext::new(Scripted::default());
        assert!(!evaluate_init(&failing));
        SET.with(|s| s.set(None));
        assert!(
            // SAFETY: as `evaluate_init`'s own call: `sibling` outlives the
            // call, and a null `clingo_propagate_init_t` pointer is never
            // dereferenced by `Scripted::init`.
            unsafe {
                init::<Scripted, Recorded>(
                    std::ptr::null_mut(),
                    std::ptr::from_ref(&sibling).cast_mut().cast(),
                )
            },
            "a different propagator's own context is unaffected"
        );
        let sibling_slots: &dyn PropagatorSlots = &sibling;
        assert!(
            sibling_slots.take_error().is_none(),
            "the sibling's own error slot was never set"
        );
    }

    #[test]
    fn a_panic_on_one_propagator_never_touches_a_sibling() {
        let panicking = PropagatorContext::new(Scripted {
            init_panics: true,
            ..Scripted::default()
        });
        let sibling = PropagatorContext::new(Scripted::default());
        assert!(!evaluate_init(&panicking));
        let panicking_slots: &dyn PropagatorSlots = &panicking;
        assert!(panicking_slots.take_panic().is_some());

        SET.with(|s| s.set(None));
        assert!(
            // SAFETY: as `a_failure_on_one_propagator_never_touches_a_sibling`.
            unsafe {
                init::<Scripted, Recorded>(
                    std::ptr::null_mut(),
                    std::ptr::from_ref(&sibling).cast_mut().cast(),
                )
            },
            "a different propagator's own context is unaffected by another's panic"
        );
        let sibling_slots: &dyn PropagatorSlots = &sibling;
        assert!(
            sibling_slots.take_panic().is_none(),
            "the sibling's own panic slot was never set"
        );
    }
}

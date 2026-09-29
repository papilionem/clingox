//! The 19 trampolines of `clingo_ground_program_observer_t` (clingo.h:2648-
//! 2848), and `clingo_control_register_observer`/`_register_backend`
//! (clingo.h:3326-3349).
//!
//! Every trampoline follows the ground callback's shape (`raw::trampoline`,
//! S8): read the boxed context from `data`, run the matching
//! [`GroundProgramObserver`] method inside [`guard`], store a returned error
//! or a caught panic in the context's slots, set clingo's error state and
//! return `false` on either. `run` below is the one place that dance is
//! written; every trampoline only extracts its own arguments from raw
//! pointers and calls it.
//!
//! Unlike the ground callback (borrowed, scoped to one `ground_with` call),
//! the observer is `Send + 'static` and owned by the `Control` for as long
//! as it stays registered (S10): its
//! [`ObserverContext`] is boxed once, at `register_observer`, and the box is
//! kept in [`ControlHandle::observers`] until the control itself is dropped,
//! after `clingo_control_free` (never before: clingo may call it up to that
//! point).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::path::Path;

use clingox_sys as ffi;

use super::control::ControlHandle;
use super::trampoline::{PanicSlot, Slot, fail, guard};
use super::{ErrorState, borrowed_str, c_str, raw_slice};
use crate::atoms::ProgramLiteral;
use crate::backend::{Atom, BackendWriterKind, ExternalKind, HeuristicKind};
use crate::error::{Error, ErrorKind};
use crate::observer::{GroundProgramObserver, TheoryCompoundKind};
use crate::symbol::Symbol;
use crate::theory::Id;

/// What the data pointer of a registered observer points to (DESIGN S8, S10).
///
/// `strings` remembers every theory string term's text by id, so
/// `theory_atom_with_guard`'s trampoline can resolve its `operator_id` to text:
/// unlike `clingo_backend_theory_atom_with_guard` (which takes the operator as
/// a plain C string), the observer callback only gives the id of an
/// already-declared string term (the aspif format always declares a term before
/// using it), and `operator` is therefore exposed as `&str`, not an `Id`. Ids
/// reset every step, but whichever operator a new step uses is redeclared, at
/// that id, before it is referenced there, so a stale entry from an earlier
/// step is always overwritten before it would be read.
struct ObserverContext<O> {
    observer: RefCell<O>,
    strings: RefCell<HashMap<u32, String>>,
    error: Slot<Error>,
    panic: PanicSlot,
}

impl<O: GroundProgramObserver> ObserverContext<O> {
    fn new(observer: O) -> Self {
        ObserverContext {
            observer: RefCell::new(observer),
            strings: RefCell::new(HashMap::new()),
            error: Slot::default(),
            panic: PanicSlot::default(),
        }
    }

    /// Runs `body` on the observer, guarding against the observer calling
    /// back into the same grounding on this thread (mirrors
    /// `GroundContext::call`; the reentrancy this catches is not otherwise
    /// reachable from safe code).
    fn call(&self, body: impl FnOnce(&mut O) -> Result<(), Error>) -> Result<(), Error> {
        let mut observer = self.observer.try_borrow_mut().map_err(|_| {
            Error::new(
                ErrorKind::Logic,
                "an observer callback was entered while it was already running",
            )
        })?;
        body(&mut observer)
    }
}

/// A type-erased handle onto a registered observer's error and panic slots,
/// so [`ControlHandle`] can check every registered observer after a
/// grounding call without naming its concrete `O` (which only the
/// registering call ever knows).
pub(super) trait ObserverSlots {
    /// Takes the recorded callback error, if any (first writer wins, S8).
    fn take_error(&self) -> Option<Error>;
    /// Takes the recorded panic payload, if any.
    fn take_panic(&self) -> Option<Box<dyn std::any::Any + Send>>;
}

impl<O: GroundProgramObserver> ObserverSlots for ObserverContext<O> {
    fn take_error(&self) -> Option<Error> {
        self.error.take()
    }

    fn take_panic(&self) -> Option<Box<dyn std::any::Any + Send>> {
        self.panic.take()
    }
}

/// The shared body every observer trampoline runs (S8), once its own
/// arguments have been read from the raw pointers clingo passed. Reports an
/// earlier failure on this context at once (defensive: clingo's own contract
/// already stops calling after a `false`), otherwise runs `body` inside
/// [`guard`], stores a returned error or caught panic, and sets clingo's
/// error state on either.
fn run<O: GroundProgramObserver, S: ErrorState>(
    context: &ObserverContext<O>,
    body: impl FnOnce(&mut O) -> Result<(), Error>,
) -> bool {
    if context.error.is_set() {
        fail::<S>(c"an earlier observer callback failed");
        return false;
    }
    match guard(&context.panic, || context.call(body)) {
        Some(Ok(())) => true,
        Some(Err(err)) => {
            context.error.store(err);
            fail::<S>(c"an observer callback returned an error");
            false
        }
        None => {
            fail::<S>(c"an observer callback panicked");
            false
        }
    }
}

/// clingo's `clingo_heuristic_type_e` back to a [`HeuristicKind`] (reverse of
/// `raw::backend::raw_heuristic_type`, clingo.h:1596-1603).
fn heuristic_kind(raw: c_int) -> Result<HeuristicKind, Error> {
    match c_uint::try_from(raw) {
        Ok(v) if v == ffi::clingo_heuristic_type_level => Ok(HeuristicKind::Level),
        Ok(v) if v == ffi::clingo_heuristic_type_sign => Ok(HeuristicKind::Sign),
        Ok(v) if v == ffi::clingo_heuristic_type_factor => Ok(HeuristicKind::Factor),
        Ok(v) if v == ffi::clingo_heuristic_type_init => Ok(HeuristicKind::Init),
        Ok(v) if v == ffi::clingo_heuristic_type_true => Ok(HeuristicKind::True),
        Ok(v) if v == ffi::clingo_heuristic_type_false => Ok(HeuristicKind::False),
        _ => Err(Error::new(
            ErrorKind::Unknown,
            format!("clingo reported an unknown heuristic type ({raw})"),
        )),
    }
}

/// clingo's `clingo_external_type_e` back to an [`ExternalKind`] (reverse of
/// `raw::backend::raw_external_type`, clingo.h:1613-1620).
fn external_kind(raw: c_int) -> Result<ExternalKind, Error> {
    match c_uint::try_from(raw) {
        Ok(v) if v == ffi::clingo_external_type_free => Ok(ExternalKind::Free),
        Ok(v) if v == ffi::clingo_external_type_true => Ok(ExternalKind::True),
        Ok(v) if v == ffi::clingo_external_type_false => Ok(ExternalKind::False),
        Ok(v) if v == ffi::clingo_external_type_release => Ok(ExternalKind::Release),
        _ => Err(Error::new(
            ErrorKind::Unknown,
            format!("clingo reported an unknown external type ({raw})"),
        )),
    }
}

/// `name_id_or_type` to a [`TheoryCompoundKind`] (clingo.h:2801-2805):
/// `-1`/`-2`/`-3` name the three sequence kinds, and any other value is the
/// id of the function's name (itself a string term).
fn compound_kind(raw: c_int) -> TheoryCompoundKind {
    match raw {
        -1 => TheoryCompoundKind::Tuple,
        -2 => TheoryCompoundKind::Set,
        -3 => TheoryCompoundKind::List,
        name => {
            debug_assert!(
                name >= 0,
                "theory_term_compound's name_id_or_type is -1, -2, -3 or a term id: {name}"
            );
            TheoryCompoundKind::Function(Id::from_raw(u32::try_from(name).unwrap_or(0)))
        }
    }
}

/// `atom_id_or_zero`/`atom` (`0` for a directive) to `Option<Atom>`
/// (the observer never sees `TheoryAtomTarget::Fresh`).
fn observed_atom(raw: u32) -> Option<Atom> {
    (raw != 0).then(|| Atom::from_raw(raw))
}

/// Every `Atom`/`ProgramLiteral` an observer trampoline hands to the
/// caller's own [`GroundProgramObserver`] is checked against the same
/// invariant [`ProgramLiteral::from_raw`] enforces everywhere else in the
/// crate, even
/// though most of these values are built directly from clingo's own raw
/// array (`raw_slice`, `#[repr(transparent)]`) rather than through that
/// constructor, for zero-copy access to memory clingo itself owns. A
/// hostile aspif file can make clingo hand out an atom or literal outside
/// [`ProgramLiteral::MAX_MAGNITUDE`]; left unchecked, it used to reach
/// [`Atom::pos`]/[`Atom::neg`]'s own `debug_assert!`, which aborts a debug
/// build since that assertion is relied on
/// elsewhere in the crate as a true invariant, never a caller-facing check.
/// Every one of the call sites below runs inside `run`'s [`guard`], so
/// rejecting a bad value with these is an ordinary `Err`, never a panic
/// that could unwind past this `extern "C"` boundary into clingo's C++
/// frames.
fn literal_out_of_range(raw: i32) -> Error {
    Error::new(
        ErrorKind::Runtime,
        format!("clingo reported a literal ({raw}) outside ProgramLiteral::MAX_MAGNITUDE"),
    )
}

/// As [`literal_out_of_range`], confirming a literal clingox already
/// reinterpreted straight from clingo's own array (rather than built
/// through [`ProgramLiteral::from_raw`]) still satisfies that constructor's
/// own rule.
fn ensure_literal_valid(literal: ProgramLiteral) -> Result<(), Error> {
    if ProgramLiteral::from_raw(literal.get()).is_some() {
        Ok(())
    } else {
        Err(literal_out_of_range(literal.get()))
    }
}

/// [`ensure_literal_valid`] for a whole slice.
fn ensure_literals_valid(literals: &[ProgramLiteral]) -> Result<(), Error> {
    literals.iter().try_for_each(|&l| ensure_literal_valid(l))
}

/// As [`literal_out_of_range`], naming an atom id instead: `Atom`'s own
/// invariant is exactly `ProgramLiteral`'s, applied to the atom's raw id as
/// its positive literal ([`Atom::pos`]).
fn atom_out_of_range(raw: u32) -> Error {
    Error::new(
        ErrorKind::Runtime,
        format!(
            "clingo reported an atom id ({raw}) outside the range Atom's own invariant assumes \
             (nonzero, within ProgramLiteral::MAX_MAGNITUDE)"
        ),
    )
}

/// [`ensure_literal_valid`] for an atom, reusing [`ProgramLiteral::from_raw`]
/// on its raw id as `Atom::pos` itself would.
fn ensure_atom_valid(atom: Atom) -> Result<(), Error> {
    if ProgramLiteral::from_raw(i32::try_from(atom.raw()).unwrap_or(i32::MAX)).is_some() {
        Ok(())
    } else {
        Err(atom_out_of_range(atom.raw()))
    }
}

/// [`ensure_atom_valid`] for a whole slice.
fn ensure_atoms_valid(atoms: &[Atom]) -> Result<(), Error> {
    atoms.iter().try_for_each(|&a| ensure_atom_valid(a))
}

/// Reads the boxed [`ObserverContext`] a trampoline's `data` pointer names;
/// every one of the 19 trampolines below starts with this same cast.
///
/// # Safety
///
/// `data` must point to an `ObserverContext<O>` that is valid for `'a` (the
/// caller's contract, clingo.h:3326-3335): `ControlHandle::register_observer`
/// boxes it and keeps it in `ControlHandle::observers` until after
/// `clingo_control_free`. Observer callbacks run on the caller's thread
/// during grounding (DESIGN S10), so only shared access is ever taken here.
unsafe fn context_of<'a, O: GroundProgramObserver>(data: *mut c_void) -> &'a ObserverContext<O> {
    // SAFETY: the caller's contract above.
    unsafe { &*data.cast::<ObserverContext<O>>().cast_const() }
}

unsafe extern "C" fn init_program<O: GroundProgramObserver, S: ErrorState>(
    incremental: bool,
    data: *mut c_void,
) -> bool {
    // SAFETY: `data` is the context `ControlHandle::register_observer`
    // boxed and registered for this control, kept alive in
    // `ControlHandle::observers` until after `clingo_control_free` (the
    // caller's contract, clingo.h:3326-3335). Observer callbacks run on
    // the caller's thread during grounding (DESIGN S10), so only shared
    // access is ever taken here.
    let context = unsafe { context_of(data) };
    run::<O, S>(context, |observer| observer.init_program(incremental))
}

unsafe extern "C" fn begin_step<O: GroundProgramObserver, S: ErrorState>(
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    run::<O, S>(context, GroundProgramObserver::begin_step)
}

unsafe extern "C" fn end_step<O: GroundProgramObserver, S: ErrorState>(data: *mut c_void) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    run::<O, S>(context, GroundProgramObserver::end_step)
}

unsafe extern "C" fn rule<O: GroundProgramObserver, S: ErrorState>(
    choice: bool,
    head: *const ffi::clingo_atom_t,
    head_size: usize,
    body: *const ffi::clingo_literal_t,
    body_size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    // SAFETY: `head` is null or `head_size` valid `clingo_atom_t`s for this
    // call (the caller's contract, clingo.h:2694-2703); `Atom` is
    // `#[repr(transparent)]` over `clingo_atom_t`.
    let head = unsafe { raw_slice(head.cast::<Atom>(), head_size) };
    // SAFETY: as above; `ProgramLiteral` is `#[repr(transparent)]` over
    // `clingo_literal_t`, and every literal a rule body carries is a real,
    // nonzero aspif literal within clasp's variable range, the same
    // invariant `TheoryElement::condition` already relies on.
    let body = unsafe { raw_slice(body.cast::<ProgramLiteral>(), body_size) };
    run::<O, S>(context, |observer| {
        ensure_atoms_valid(head)?;
        ensure_literals_valid(body)?;
        observer.rule(choice, head, body)
    })
}

unsafe extern "C" fn weight_rule<O: GroundProgramObserver, S: ErrorState>(
    choice: bool,
    head: *const ffi::clingo_atom_t,
    head_size: usize,
    lower_bound: ffi::clingo_weight_t,
    body: *const ffi::clingo_weighted_literal_t,
    body_size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    // SAFETY: as in `rule`.
    let head = unsafe { raw_slice(head.cast::<Atom>(), head_size) };
    // SAFETY: `body` is null or `body_size` valid `clingo_weighted_literal_t`s
    // for this call (clingo.h:1986-2004 describes the same shape on the
    // authoring side); each is read into an owned `(ProgramLiteral, i32)`
    // pair below rather than reinterpreted in place, since a Rust tuple's
    // layout is not guaranteed to match the C struct's. Built with
    // `ProgramLiteral::from_raw`, not the assuming `from_valid`, and inside
    // `run`'s `guard` below: a hostile aspif's
    // out-of-range literal is rejected as an ordinary `Err`, not a
    // `debug_assert!` panic that could unwind past this `extern "C"`
    // boundary.
    let body = unsafe { raw_slice(body, body_size) };
    run::<O, S>(context, |observer| {
        ensure_atoms_valid(head)?;
        let body: Vec<(ProgramLiteral, i32)> = body
            .iter()
            .map(|wl| {
                let literal = ProgramLiteral::from_raw(wl.literal)
                    .ok_or_else(|| literal_out_of_range(wl.literal))?;
                Ok((literal, wl.weight))
            })
            .collect::<Result<_, Error>>()?;
        observer.weight_rule(choice, head, lower_bound, &body)
    })
}

unsafe extern "C" fn minimize<O: GroundProgramObserver, S: ErrorState>(
    priority: ffi::clingo_weight_t,
    literals: *const ffi::clingo_weighted_literal_t,
    size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    // SAFETY: as in `weight_rule`, and built the same validated way, for the
    // same reason.
    let literals = unsafe { raw_slice(literals, size) };
    run::<O, S>(context, |observer| {
        let literals: Vec<(ProgramLiteral, i32)> = literals
            .iter()
            .map(|wl| {
                let literal = ProgramLiteral::from_raw(wl.literal)
                    .ok_or_else(|| literal_out_of_range(wl.literal))?;
                Ok((literal, wl.weight))
            })
            .collect::<Result<_, Error>>()?;
        observer.minimize(priority, &literals)
    })
}

unsafe extern "C" fn project<O: GroundProgramObserver, S: ErrorState>(
    atoms: *const ffi::clingo_atom_t,
    size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    // SAFETY: as in `rule`.
    let atoms = unsafe { raw_slice(atoms.cast::<Atom>(), size) };
    run::<O, S>(context, |observer| {
        ensure_atoms_valid(atoms)?;
        observer.project(atoms)
    })
}

unsafe extern "C" fn output_atom<O: GroundProgramObserver, S: ErrorState>(
    symbol: ffi::clingo_symbol_t,
    atom: ffi::clingo_atom_t,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    let symbol = Symbol::from_clingo(symbol);
    let atom = observed_atom(atom);
    run::<O, S>(context, |observer| {
        if let Some(atom) = atom {
            ensure_atom_valid(atom)?;
        }
        observer.output_atom(symbol, atom)
    })
}

unsafe extern "C" fn output_term<O: GroundProgramObserver, S: ErrorState>(
    symbol: ffi::clingo_symbol_t,
    condition: *const ffi::clingo_literal_t,
    size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    let symbol = Symbol::from_clingo(symbol);
    // SAFETY: as in `rule`'s body.
    let condition = unsafe { raw_slice(condition.cast::<ProgramLiteral>(), size) };
    run::<O, S>(context, |observer| {
        ensure_literals_valid(condition)?;
        observer.output_term(symbol, condition)
    })
}

unsafe extern "C" fn external<O: GroundProgramObserver, S: ErrorState>(
    atom: ffi::clingo_atom_t,
    type_: ffi::clingo_external_type_t,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    let atom = Atom::from_raw(atom);
    run::<O, S>(context, |observer| {
        ensure_atom_valid(atom)?;
        observer.external(atom, external_kind(type_)?)
    })
}

unsafe extern "C" fn assume<O: GroundProgramObserver, S: ErrorState>(
    literals: *const ffi::clingo_literal_t,
    size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    // SAFETY: as in `rule`'s body.
    let literals = unsafe { raw_slice(literals.cast::<ProgramLiteral>(), size) };
    run::<O, S>(context, |observer| {
        ensure_literals_valid(literals)?;
        observer.assume(literals)
    })
}

unsafe extern "C" fn heuristic<O: GroundProgramObserver, S: ErrorState>(
    atom: ffi::clingo_atom_t,
    type_: ffi::clingo_heuristic_type_t,
    bias: c_int,
    priority: c_uint,
    condition: *const ffi::clingo_literal_t,
    size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    let atom = Atom::from_raw(atom);
    // SAFETY: as in `rule`'s body.
    let condition = unsafe { raw_slice(condition.cast::<ProgramLiteral>(), size) };
    run::<O, S>(context, |observer| {
        ensure_atom_valid(atom)?;
        ensure_literals_valid(condition)?;
        observer.heuristic(atom, heuristic_kind(type_)?, bias, priority, condition)
    })
}

unsafe extern "C" fn acyc_edge<O: GroundProgramObserver, S: ErrorState>(
    node_u: c_int,
    node_v: c_int,
    condition: *const ffi::clingo_literal_t,
    size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    // SAFETY: as in `rule`'s body.
    let condition = unsafe { raw_slice(condition.cast::<ProgramLiteral>(), size) };
    run::<O, S>(context, |observer| {
        ensure_literals_valid(condition)?;
        observer.acyc_edge(node_u, node_v, condition)
    })
}

unsafe extern "C" fn theory_term_number<O: GroundProgramObserver, S: ErrorState>(
    term_id: ffi::clingo_id_t,
    number: c_int,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    let term = Id::from_raw(term_id);
    run::<O, S>(context, |observer| {
        observer.theory_term_number(term, number)
    })
}

unsafe extern "C" fn theory_term_string<O: GroundProgramObserver, S: ErrorState>(
    term_id: ffi::clingo_id_t,
    name: *const c_char,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    let term = Id::from_raw(term_id);
    // SAFETY: `name` is a non-null, NUL-terminated string valid for this call
    // (the caller's contract, clingo.h:2779-2787: a theory string term's
    // text). Copied at once, into the map and (on success) into the trait
    // call, since nothing guarantees clingo keeps it alive past this call.
    let name = unsafe { borrowed_str(name) };
    if let Ok(name) = &name {
        context
            .strings
            .borrow_mut()
            .insert(term_id, (*name).to_owned());
    }
    run::<O, S>(context, |observer| observer.theory_term_string(term, name?))
}

unsafe extern "C" fn theory_term_compound<O: GroundProgramObserver, S: ErrorState>(
    term_id: ffi::clingo_id_t,
    name_id_or_type: c_int,
    arguments: *const ffi::clingo_id_t,
    size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    let term = Id::from_raw(term_id);
    let kind = compound_kind(name_id_or_type);
    // SAFETY: `arguments` is null or `size` valid `clingo_id_t`s for this
    // call; `Id` is `#[repr(transparent)]` over `clingo_id_t`.
    let arguments = unsafe { raw_slice(arguments.cast::<Id>(), size) };
    run::<O, S>(context, |observer| {
        observer.theory_term_compound(term, kind, arguments)
    })
}

unsafe extern "C" fn theory_element<O: GroundProgramObserver, S: ErrorState>(
    element_id: ffi::clingo_id_t,
    terms: *const ffi::clingo_id_t,
    terms_size: usize,
    condition: *const ffi::clingo_literal_t,
    condition_size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    let element = Id::from_raw(element_id);
    // SAFETY: as in `theory_term_compound`.
    let terms = unsafe { raw_slice(terms.cast::<Id>(), terms_size) };
    // SAFETY: as in `rule`'s body.
    let condition = unsafe { raw_slice(condition.cast::<ProgramLiteral>(), condition_size) };
    run::<O, S>(context, |observer| {
        ensure_literals_valid(condition)?;
        observer.theory_element(element, terms, condition)
    })
}

unsafe extern "C" fn theory_atom<O: GroundProgramObserver, S: ErrorState>(
    atom_id_or_zero: ffi::clingo_id_t,
    term_id: ffi::clingo_id_t,
    elements: *const ffi::clingo_id_t,
    size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    let atom = observed_atom(atom_id_or_zero);
    let term = Id::from_raw(term_id);
    // SAFETY: as in `theory_term_compound`.
    let elements = unsafe { raw_slice(elements.cast::<Id>(), size) };
    run::<O, S>(context, |observer| {
        if let Some(atom) = atom {
            ensure_atom_valid(atom)?;
        }
        observer.theory_atom(atom, term, elements)
    })
}

unsafe extern "C" fn theory_atom_with_guard<O: GroundProgramObserver, S: ErrorState>(
    atom_id_or_zero: ffi::clingo_id_t,
    term_id: ffi::clingo_id_t,
    elements: *const ffi::clingo_id_t,
    size: usize,
    operator_id: ffi::clingo_id_t,
    right_hand_side_id: ffi::clingo_id_t,
    data: *mut c_void,
) -> bool {
    // SAFETY: as in `init_program`.
    let context = unsafe { context_of(data) };
    let atom = observed_atom(atom_id_or_zero);
    let term = Id::from_raw(term_id);
    // SAFETY: as in `theory_term_compound`.
    let elements = unsafe { raw_slice(elements.cast::<Id>(), size) };
    let right_hand_side = Id::from_raw(right_hand_side_id);
    let operator = context.strings.borrow().get(&operator_id).cloned();
    run::<O, S>(context, |observer| {
        if let Some(atom) = atom {
            ensure_atom_valid(atom)?;
        }
        let operator = operator.ok_or_else(|| {
            Error::new(
                ErrorKind::Unknown,
                format!(
                    "clingo referenced theory operator term {operator_id} before declaring it \
                     as a string term"
                ),
            )
        })?;
        observer.theory_atom_with_guard(atom, term, elements, &operator, right_hand_side)
    })
}

/// Builds the `clingo_ground_program_observer_t` clingo copies at
/// registration (control.cc:2285): every field is `Some`, monomorphized for
/// `O`, since [`GroundProgramObserver`] gives every method a default body,
/// so there is nothing to leave `None` for (`H:2661`'s "can be set to NULL
/// if not needed" does not apply here).
fn observer_struct<O: GroundProgramObserver, S: ErrorState>()
-> ffi::clingo_ground_program_observer_t {
    ffi::clingo_ground_program_observer_t {
        init_program: Some(init_program::<O, S>),
        begin_step: Some(begin_step::<O, S>),
        end_step: Some(end_step::<O, S>),
        rule: Some(rule::<O, S>),
        weight_rule: Some(weight_rule::<O, S>),
        minimize: Some(minimize::<O, S>),
        project: Some(project::<O, S>),
        output_atom: Some(output_atom::<O, S>),
        output_term: Some(output_term::<O, S>),
        external: Some(external::<O, S>),
        assume: Some(assume::<O, S>),
        heuristic: Some(heuristic::<O, S>),
        acyc_edge: Some(acyc_edge::<O, S>),
        theory_term_number: Some(theory_term_number::<O, S>),
        theory_term_string: Some(theory_term_string::<O, S>),
        theory_term_compound: Some(theory_term_compound::<O, S>),
        theory_element: Some(theory_element::<O, S>),
        theory_atom: Some(theory_atom::<O, S>),
        theory_atom_with_guard: Some(theory_atom_with_guard::<O, S>),
    }
}

impl ControlHandle {
    /// Registers a ground program observer (clingo.h:3326-3335): boxes it
    /// (with its error and panic slots) once, hands clingo the C struct and
    /// the box's address, and, on success, keeps the box in
    /// [`ControlHandle::observers`] for the rest of the control's life
    /// (S4, S8, S10). Any leftover search is closed first, as for every other
    /// control operation.
    ///
    /// clingo composes repeated registrations itself (`OutputBase::
    /// registerObserver`, `libgringo/src/output/output.cc:588-597`: a tee
    /// when `replace` is false, a plain replacement of the routing to the
    /// solver otherwise), so nothing here needs to remove an earlier
    /// registration; it only needs to keep every boxed context alive for as
    /// long as clingo might still hold its address.
    pub(crate) fn register_observer<O: GroundProgramObserver>(
        &mut self,
        observer: O,
        replace: bool,
    ) -> Result<(), Error> {
        let context = Box::new(ObserverContext::new(observer));
        let data = std::ptr::from_ref::<ObserverContext<O>>(&context)
            .cast_mut()
            .cast::<c_void>();
        let raw_observer = observer_struct::<O, super::ClingoErrorState>();
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and no
            // search is open (`logged` closed it). `raw_observer` is a local
            // clingo copies into its own state during this call
            // (control.cc:2282-2287); `data` points to `context`'s heap
            // allocation, which is moved (not reallocated) into
            // `self.observers` right after this call succeeds, and kept
            // there until `clingo_control_free` (clingo.h:3326-3335).
            unsafe {
                ffi::clingo_control_register_observer(
                    control,
                    &raw const raw_observer,
                    replace,
                    data,
                )
            }
        })?;
        self.observers
            .push(context as Box<dyn ObserverSlots + Send>);
        Ok(())
    }

    /// Registers a backend writer (clingo.h:3336-3349): clingo manages the
    /// file and the routing itself, so there is no Rust-side state to keep
    /// alive beyond this call (it opens no
    /// Rust-visible handle at all).
    pub(crate) fn register_backend_writer(
        &mut self,
        kind: BackendWriterKind,
        file: &Path,
        replace: bool,
    ) -> Result<(), Error> {
        let file = path_to_cstring(file)?;
        let bits = c_uint::try_from(kind.bits()).unwrap_or(0);
        self.logged(|control| {
            // SAFETY: `control` is the live control this handle owns, and no
            // search is open (`logged` closed it). `file` is NUL-terminated
            // and outlives the call; clingo opens and manages the file
            // itself (clingo.h:3336-3349).
            unsafe { ffi::clingo_control_register_backend(control, bits, file.as_ptr(), replace) }
        })
    }

    /// After a call that could trigger a registered observer, the first
    /// recorded panic among every one of them, if any (S8: only one can ever be
    /// set, since clingo stops calling any trampoline once one returns `false`,
    /// but every observer is checked defensively).
    ///
    /// `pub(crate)`, not `pub(super)`: besides [`ControlHandle::
    /// resolve_ground`] (once the only reader), `crate::control::Control` now
    /// also reads this directly, from every entry point outside
    /// `ground`/`ground_with` through which clingo can still call a registered
    /// observer (`Control::guarded`, `Control:: settle_events` and
    /// `Control::with_backend`, see the rustdoc of
    /// [`crate::observer::GroundProgramObserver`] for the full list of paths
    /// and their clingo source lines).
    pub(crate) fn take_observer_panic(&self) -> Option<Box<dyn std::any::Any + Send>> {
        self.observers.iter().find_map(|o| o.take_panic())
    }

    /// As [`ControlHandle::take_observer_panic`], for a returned error.
    pub(crate) fn take_observer_error(&self) -> Option<Error> {
        self.observers.iter().find_map(|o| o.take_error())
    }
}

/// A filesystem path as clingo takes a C string: rejects a NUL byte and
/// non-UTF-8 (clingox's own `InvalidInput`-flavoured checks; clingo itself
/// takes a plain `char const *`, so anything not representable as one is
/// rejected before the call).
fn path_to_cstring(path: &Path) -> Result<CString, Error> {
    let text = path.to_str().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("{} is not valid UTF-8", path.display()),
        )
    })?;
    c_str(text)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::{CStr, c_uint};

    use super::*;
    use crate::error::Result;

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

    #[derive(Default)]
    struct Recorder {
        rules: Vec<(bool, Vec<Atom>, Vec<ProgramLiteral>)>,
        fail_on_rule: bool,
        panic_on_rule: bool,
    }

    impl GroundProgramObserver for Recorder {
        fn rule(&mut self, choice: bool, head: &[Atom], body: &[ProgramLiteral]) -> Result<()> {
            assert!(!self.panic_on_rule, "rule panics on purpose");
            if self.fail_on_rule {
                return Err(Error::new(ErrorKind::Conversion, "rule fails on purpose"));
            }
            self.rules.push((choice, head.to_vec(), body.to_vec()));
            Ok(())
        }
    }

    fn atom(id: u32) -> ffi::clingo_atom_t {
        id
    }

    #[test]
    fn rule_reads_head_and_body_and_forwards_to_the_observer() {
        let context = ObserverContext::new(Recorder::default());
        let head = [atom(1), atom(2)];
        let body = [3_i32, -4];
        SET.with(|s| s.set(None));
        // SAFETY: `context`, `head` and `body` all outlive this call, and
        // `context` is a live `ObserverContext<Recorder>`.
        let ok = unsafe {
            rule::<Recorder, Recorded>(
                true,
                head.as_ptr(),
                head.len(),
                body.as_ptr(),
                body.len(),
                std::ptr::from_ref(&context).cast_mut().cast(),
            )
        };
        assert!(ok);
        assert_eq!(SET.with(Cell::get), None);
        let recorded = &context.observer.borrow().rules;
        assert_eq!(recorded.len(), 1);
        let (choice, recorded_head, recorded_body) = &recorded[0];
        assert!(*choice);
        assert_eq!(recorded_head, &[Atom::from_raw(1), Atom::from_raw(2)]);
        assert_eq!(
            recorded_body,
            &[
                ProgramLiteral::from_valid(3),
                ProgramLiteral::from_valid(-4)
            ]
        );
    }

    #[test]
    fn a_returned_error_is_stored_and_stops_later_calls() {
        let context = ObserverContext::new(Recorder {
            fail_on_rule: true,
            ..Recorder::default()
        });
        let data = std::ptr::from_ref(&context).cast_mut().cast();
        SET.with(|s| s.set(None));
        // SAFETY: `data` points to the live `context` above, which outlives
        // this call; a null pointer with a zero length is a valid empty
        // slice for `raw_slice`.
        let ok = unsafe {
            rule::<Recorder, Recorded>(false, std::ptr::null(), 0, std::ptr::null(), 0, data)
        };
        assert!(!ok);
        assert_eq!(SET.with(Cell::get), Some(ffi::clingo_error_unknown));
        assert!(context.error.is_set());

        // Once the slot is set, a later call fails at once without running
        // the observer again (S8).
        // SAFETY: as above.
        let ok = unsafe { begin_step::<Recorder, Recorded>(data) };
        assert!(!ok);

        assert_eq!(context.error.take().unwrap().kind(), ErrorKind::Conversion);
    }

    #[test]
    fn a_panic_is_caught_and_recorded() {
        let context = ObserverContext::new(Recorder {
            panic_on_rule: true,
            ..Recorder::default()
        });
        let data = std::ptr::from_ref(&context).cast_mut().cast();
        SET.with(|s| s.set(None));
        // SAFETY: as in `a_returned_error_is_stored_and_stops_later_calls`.
        let ok = unsafe {
            rule::<Recorder, Recorded>(false, std::ptr::null(), 0, std::ptr::null(), 0, data)
        };
        assert!(!ok);
        let payload = context.panic.take().unwrap();
        assert_eq!(
            *payload.downcast::<&str>().unwrap(),
            "rule panics on purpose"
        );
    }

    #[derive(Default)]
    struct GuardRecorder {
        operator: Option<String>,
    }

    impl GroundProgramObserver for GuardRecorder {
        fn theory_atom_with_guard(
            &mut self,
            _atom: Option<Atom>,
            _term: Id,
            _elements: &[Id],
            operator: &str,
            _right_hand_side: Id,
        ) -> Result<()> {
            self.operator = Some(operator.to_owned());
            Ok(())
        }
    }

    #[test]
    fn theory_atom_with_guard_resolves_the_operator_from_an_earlier_string_term() {
        let context = ObserverContext::new(GuardRecorder::default());
        let name = CString::new("=").unwrap();
        let data = std::ptr::from_ref(&context).cast_mut().cast();
        // SAFETY: `data` points to the live `context` above, which outlives
        // both calls; `name` is a valid NUL-terminated string outliving the
        // first call.
        let ok = unsafe { theory_term_string::<GuardRecorder, Recorded>(9, name.as_ptr(), data) };
        assert!(ok);
        // SAFETY: as above; a null pointer with a zero length is a valid
        // empty slice for `raw_slice`.
        let ok = unsafe {
            theory_atom_with_guard::<GuardRecorder, Recorded>(0, 1, std::ptr::null(), 0, 9, 2, data)
        };
        assert!(ok);
        assert_eq!(context.observer.borrow().operator.as_deref(), Some("="));
    }

    #[test]
    fn theory_atom_with_guard_fails_on_an_undeclared_operator() {
        let context = ObserverContext::new(GuardRecorder::default());
        let data = std::ptr::from_ref(&context).cast_mut().cast();
        // SAFETY: as in
        // `theory_atom_with_guard_resolves_the_operator_from_an_earlier_string_term`,
        // without the earlier `theory_term_string` call: `operator_id` 9 is
        // never declared, which is exactly the case this test pins.
        let ok = unsafe {
            theory_atom_with_guard::<GuardRecorder, Recorded>(0, 1, std::ptr::null(), 0, 9, 2, data)
        };
        assert!(!ok);
        assert_eq!(context.error.take().unwrap().kind(), ErrorKind::Unknown);
    }

    #[test]
    fn compound_kind_maps_the_three_sentinels_and_a_term_id() {
        assert_eq!(compound_kind(-1), TheoryCompoundKind::Tuple);
        assert_eq!(compound_kind(-2), TheoryCompoundKind::Set);
        assert_eq!(compound_kind(-3), TheoryCompoundKind::List);
        assert_eq!(
            compound_kind(5),
            TheoryCompoundKind::Function(Id::from_raw(5))
        );
    }

    #[test]
    fn observed_atom_maps_zero_to_none() {
        assert_eq!(observed_atom(0), None);
        assert_eq!(observed_atom(3), Some(Atom::from_raw(3)));
    }
}

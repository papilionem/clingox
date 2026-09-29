//! Propagators: extending the search with a custom theory.
//!
//! [`Control::register_propagator`] registers a [`Propagator`], which sees
//! the search from the inside: [`Propagator::init`] runs once before each
//! solving step, and [`Propagator::propagate`], [`Propagator::undo`],
//! [`Propagator::check`] and [`Propagator::decide`] run during it, on
//! whichever solver thread clingo is using. This module has the propagator
//! side (registration, [`PropagateInit`], watches, and the trait's five
//! callbacks, all wired to run), the complete read-only [`Assignment`] and
//! [`Trail`], and [`PropagateControl`]'s complete surface:
//! `thread_id`, `assignment`, `add_clause`, `add_literal`,
//! `add_watch`, `has_watch`, `remove_watch` and this type's own `propagate`.
//!
//! # Reentrancy (DESIGN S11)
//!
//! clasp releases its own lock inside [`PropagateControl::add_clause`],
//! [`PropagateControl::propagate`], [`PropagateControl::add_watch`] and
//! [`PropagateControl::add_literal`]. A backtrack past the propagator's own
//! watched literals can still follow, calling [`Propagator::undo`] one or
//! more times — but, checked directly against the vendored clasp 5.8.2, that
//! backtrack is always resolved after control returns to clasp, never while
//! the triggering call is still on the same thread's own stack (see
//! [`PropagateControl`]'s own rustdoc for the exact mechanism). This crate's
//! soundness never depended on which way that went: every [`Propagator`]
//! method takes `&self`, never `&mut self`, so a `Mutex` a propagator locked
//! in `propagate` and tried to lock again in a reentrant `undo` would
//! deadlock, not merely race, if a future clasp version ever took that path.
//! Keep state in a container that tolerates this either way (`OnceLock`, an
//! atomic, or a `Cell`/`RefCell` a user's own per-thread state does not
//! share across threads) and never hold a lock of your own across a call
//! into [`PropagateControl`].
//!
//! # Examples
//!
//! Registering a propagator that does nothing changes no model:
//!
//! ```
//! use clingox::propagate::Propagator;
//! use clingox::{Control, Part};
//!
//! struct NoOp;
//! impl Propagator for NoOp {}
//!
//! let mut ctl = Control::new()?;
//! ctl.add_base("1 { a; b }.")?;
//! ctl.ground(&[Part::base()])?;
//! ctl.register_propagator(NoOp)?;
//! assert!(ctl.solve(&[])?.is_sat());
//! # Ok::<(), clingox::Error>(())
//! ```

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::cell::Cell;
use std::fmt;

use crate::atoms::{ProgramLiteral, SymbolicAtoms};
use crate::control::ErrorSink;
use crate::error::{Error, ErrorKind, Result};
use crate::raw;
use crate::theory::TheoryAtoms;

// ---------------------------------------------------------------------------
// Flow

/// Whether propagation may continue, or clingo has signalled the program is
/// unsatisfiable and no further calls on the object that returned this
/// should be made.
///
/// Returned by [`PropagateInit::add_clause`], [`PropagateInit::
/// add_weight_constraint`], [`PropagateInit::propagate`],
/// [`PropagateControl::add_clause`] and [`PropagateControl::propagate`].
/// After a [`Flow::Stop`], clingox itself refuses any
/// further call this crate guards against reaching clingo (see each
/// method's own documentation for exactly which ones); clingo's own header
/// says plainly that none of them should be called again either way
/// (`clingo.h`: "no further calls on the init object or functions on the
/// assignment should be called when the result of this method is false").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Propagation may continue.
    Continue,
    /// clingo has signalled that the program became unsatisfiable.
    Stop,
}

impl Flow {
    pub(crate) fn from_continue(continue_: bool) -> Flow {
        if continue_ {
            Flow::Continue
        } else {
            Flow::Stop
        }
    }

    /// Whether this is [`Flow::Stop`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::propagate::Flow;
    ///
    /// assert!(Flow::Stop.is_stop());
    /// assert!(!Flow::Continue.is_stop());
    /// ```
    pub fn is_stop(self) -> bool {
        matches!(self, Flow::Stop)
    }
}

// ---------------------------------------------------------------------------
// SolverLiteral

/// A solver literal: the number clasp gives a variable once solving starts.
///
/// It is a separate id space from [`ProgramLiteral`] (DESIGN S17): clingo
/// maps a program literal to its solver literal with [`PropagateInit::
/// solver_literal`], and a propagator otherwise only ever sees solver
/// literals clingo itself hands back, through that method,
/// [`PropagateInit::add_literal`], a `propagate`/`decide` callback's own
/// arguments, or [`SolverLiteral::negate`].
///
/// **This type has no public constructor from a bare `i32`, deliberately.**
/// Unlike [`ProgramLiteral::from_raw`], nothing in this crate's design has a
/// caller legitimately holding a bare solver-literal integer clingo did not
/// just hand back. This is load-bearing, not a style choice: an
/// unregistered or out-of-range solver literal passed to clingo is real,
/// verified undefined behaviour, not a documented error path. Two cases
/// were reproduced directly against clingo 5.8.2:
///
/// - a bare integer never obtained from clingo (`init.add_clause([1_000_000])`
///   in pyclingo) segfaulted the process for every magnitude tried except
///   one, which is undefined behaviour over clasp's own per-variable arrays,
///   not a principled cutoff;
/// - a `SolverLiteral` obtained *legitimately* from one control's grounding,
///   passed to a *different*, smaller control, also segfaulted, even though
///   both controls' own literals are individually well-formed.
///
/// Every `PropagateInit`/`PropagateControl` method that takes one or more
/// `SolverLiteral`s therefore validates each one against the control's own
/// assignment first (`clingo_assignment_has_literal`, which is itself safe
/// for any `i32`, including `i32::MIN`/`i32::MAX`), returning
/// [`ErrorKind::InvalidInput`] for the first unknown one rather than
/// reaching clingo. Combined with having no public raw constructor, this
/// closes the segfault hazard by construction *and* against the
/// cross-control case: a future change must not add a `from_raw` for
/// API symmetry with `ProgramLiteral` without re-reading this note.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SolverLiteral(i32);

impl SolverLiteral {
    /// Builds one from a value clingo itself just handed back (a mapped
    /// program literal, a freshly added literal, or one read from a
    /// `changes`/`decide` argument): never from an arbitrary caller-supplied
    /// integer. See the type's own documentation for why this stays
    /// crate-private.
    pub(crate) fn from_raw_valid(raw: i32) -> SolverLiteral {
        SolverLiteral(raw)
    }

    /// The raw value, for building the arrays clingo's C functions take.
    pub(crate) fn get(self) -> i32 {
        self.0
    }

    /// Whether the literal is positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::propagate::{PropagateInit, Propagator};
    /// use clingox::{Control, Part, Result};
    ///
    /// struct CheckSign;
    /// impl Propagator for CheckSign {
    ///     fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
    ///         let lit = init.add_literal(true)?;
    ///         assert!(lit.is_positive());
    ///         assert!(!lit.negate().is_positive());
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.register_propagator(CheckSign)?;
    /// ctl.solve(&[])?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// The solver variable of the literal: its absolute value, the same for the
    /// literal and its [negation](SolverLiteral::negate).
    ///
    /// Variables are numbered densely from 1 up to and including the
    /// [size](crate::propagate::Assignment::size) of the assignment (measured
    /// on clingo 5.8.2: the largest variable of `{p(1..20)}.` is 21, the size),
    /// so a propagator can index its own per-variable tables of `size + 1`
    /// slots with it (the clingo crate's `get_integer` on a literal). The value
    /// says nothing about the literal's sign; ask
    /// [`is_positive`](SolverLiteral::is_positive) for that.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::propagate::{PropagateInit, Propagator};
    /// use clingox::{Control, Part, Result};
    ///
    /// struct Tables;
    /// impl Propagator for Tables {
    ///     fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
    ///         let size = init.assignment().size();
    ///         // One slot per variable, slot 0 unused.
    ///         let mut seen = vec![false; size + 1];
    ///         for atom in &init.symbolic_atoms()? {
    ///             let lit = init.solver_literal(atom?.literal())?;
    ///             assert_eq!(lit.variable(), lit.negate().variable());
    ///             seen[lit.variable() as usize] = true;
    ///         }
    ///         assert!(seen.iter().any(|&s| s));
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("{a;b}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.register_propagator(Tables)?;
    /// ctl.solve(&[])?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    #[must_use]
    pub fn variable(self) -> u32 {
        self.0.unsigned_abs()
    }

    /// The literal with the opposite sign.
    ///
    /// It is its own inverse: `l.negate().negate() == l`. `i32::MIN` never
    /// appears here in practice, since clasp keeps its own variable
    /// numbering well within range, but a broken invariant would panic here
    /// in a debug build rather than silently wrap, matching
    /// [`ProgramLiteral::negate`].
    #[must_use]
    pub fn negate(self) -> SolverLiteral {
        debug_assert_ne!(self.0, i32::MIN, "a SolverLiteral is never i32::MIN");
        SolverLiteral(-self.0)
    }
}

impl std::ops::Neg for SolverLiteral {
    type Output = SolverLiteral;

    /// Same as [`SolverLiteral::negate`].
    fn neg(self) -> SolverLiteral {
        self.negate()
    }
}

impl fmt::Debug for SolverLiteral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SolverLiteral({})", self.0)
    }
}

// ---------------------------------------------------------------------------
// CheckMode, UndoMode, WeightConstraintKind, ClauseType

/// When [`Propagator::check`] is called (`clingo_propagator_check_mode_e`).
///
/// Set with [`PropagateInit::set_check_mode`], defaults to
/// [`CheckMode::Total`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CheckMode {
    /// `check` is never called.
    Off,
    /// `check` is called once the assignment is total.
    Total,
    /// `check` is called whenever propagation reaches a fixpoint.
    Fixpoint,
    /// Both of the above.
    Both,
}

/// When [`Propagator::undo`] is called (`clingo_propagator_undo_mode_e`).
///
/// Set with [`PropagateInit::set_undo_mode`], defaults to
/// [`UndoMode::Default`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UndoMode {
    /// `undo` is called for every non-empty change list.
    Default,
    /// `undo` is also called whenever `check` was called at that level.
    Always,
}

/// How a weight constraint added by [`PropagateInit::add_weight_constraint`]
/// relates to its own literal (`clingo_weight_constraint_type_e`).
///
/// The C values are `-1`, `0`, `1`, not `0`, `1`, `2`: clingox converts from
/// the bindgen constant explicitly in every direction, never by declaration
/// order (DESIGN S17). Verified directly against clingo 5.8.2: with the
/// constraint `{a=1,b=1} >= 1` associated with literal `c`,
///
/// - [`WeightConstraintKind::ImplicationLeft`]: no model has the weighted sum
///   reach the bound while `c` is false, but `c` can be true while the sum does
///   not reach it (`c` alone is a model; `a` alone and `b` alone are not);
/// - [`WeightConstraintKind::ImplicationRight`] is the mirror image: no model
///   has `c` true while the sum does not reach the bound, but the sum can reach
///   it while `c` is false;
/// - [`WeightConstraintKind::Equivalence`] forbids both.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeightConstraintKind {
    /// The constraint being true forces the literal true; the literal can be
    /// true without the constraint holding.
    ImplicationLeft,
    /// The constraint and the literal always agree.
    Equivalence,
    /// The literal being true forces the constraint to hold; the constraint
    /// can hold without the literal being true.
    ImplicationRight,
}

/// The lifetime policy of a clause added with [`PropagateControl::add_clause`]
/// (`clingo_clause_type_e`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClauseType {
    /// Subject to the solver's own deletion policy.
    Learnt,
    /// Never deleted by the solver's own policy.
    Static,
    /// Like [`ClauseType::Learnt`], but deleted at the end of the solving
    /// step regardless.
    Volatile,
    /// Like [`ClauseType::Static`], but deleted at the end of the solving
    /// step regardless.
    VolatileStatic,
}

// ---------------------------------------------------------------------------
// Assignment

/// A read-only view of a solver's (partial) assignment.
///
/// [`Assignment::decision_level`], [`Assignment::root_level`], [`Assignment::
/// has_conflict`], [`Assignment::size`] and [`Assignment::is_total`] need
/// nothing but the assignment itself; every accessor that takes a
/// [`SolverLiteral`], a level or an offset validates it against this assignment
/// first, refusing an unknown one with [`ErrorKind:: InvalidInput`] before ever
/// reaching clingo.
///
/// # Examples
///
/// ```
/// use clingox::propagate::{PropagateInit, Propagator};
/// use clingox::{Control, Part, Result};
///
/// struct ReadAssignment;
/// impl Propagator for ReadAssignment {
///     fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
///         let a = init.assignment();
///         assert_eq!(a.decision_level(), 0);
///         assert_eq!(a.root_level(), 0);
///         assert!(!a.has_conflict());
///         // `size` counts the solver's whole variable space
///         // (`clasp/src/clingo.cpp:76`), not how many literals are
///         // currently assigned (that is `Trail::size` instead, from
///         // `Assignment::trail`); it is `2` here because grounding this
///         // program produced two variables, not because two literals
///         // happen to be unit-propagated facts already (checked against
///         // clingo 5.8.2).
///         assert_eq!(a.size(), 2);
///         assert!(!a.is_total());
///
///         // `a`'s own literal (offset 0 in ascending order) is fixed,
///         // true, at level 0.
///         let lit_a = a.at(0)?;
///         assert!(a.is_fixed(lit_a)?);
///         assert!(a.is_true(lit_a)?);
///         assert_eq!(a.truth_value(lit_a)?, Some(true));
///         assert_eq!(a.level(lit_a)?, Some(0));
///         // decision(0) always succeeds, for any assignment: clasp's own
///         // "trivially true" sentinel, always valid at decision level 0.
///         a.decision(0)?;
///         Ok(())
///     }
/// }
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("a. c :- a. {b}.")?;
/// ctl.ground(&[Part::base()])?;
/// ctl.register_propagator(ReadAssignment)?;
/// ctl.solve(&[])?;
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct Assignment<'a> {
    raw: raw::RawAssignment<'a>,
    /// The owning [`PropagateInit`]/[`PropagateControl`]'s own `stopped`
    /// flag, `None` for an assignment with no such owner (the one clingo
    /// hands directly to [`Propagator::decide`], which has no `Stop`
    /// concept of its own). A shared `&Cell<bool>` rather than a copied
    /// `bool`, so this stays live: the flag can still flip to `true` after
    /// this `Assignment` was created (a later `Flow::Stop` on the same
    /// `init`/`propagate`/`check` call), and every query must see that.
    stopped: Option<&'a Cell<bool>>,
}

impl<'a> Assignment<'a> {
    pub(crate) fn from_raw(raw: raw::RawAssignment<'a>) -> Assignment<'a> {
        Assignment { raw, stopped: None }
    }

    /// Ties this assignment to its owning [`PropagateInit`]/
    /// [`PropagateControl`]'s own `stopped` flag:
    /// every fallible query then refuses with [`ErrorKind::InvalidInput`]
    /// once the propagator reported [`Flow::Stop`], checked before the query
    /// ever reaches clingo.
    pub(crate) fn with_stopped(mut self, stopped: &'a Cell<bool>) -> Assignment<'a> {
        self.stopped = Some(stopped);
        self
    }

    /// [`PropagateInit`]/[`PropagateControl`]'s own guard, reused here: the
    /// five infallible getters and [`Assignment::has_literal`] never call
    /// this (J-notes.md's ruling: they are direct-value C functions the
    /// header never lists among the ones a post-`Stop` call must not make).
    fn check_not_stopped(&self) -> Result<()> {
        check_owner_not_stopped(self.stopped)
    }

    /// The current decision level.
    pub fn decision_level(&self) -> u32 {
        self.raw.decision_level()
    }

    /// The current root level: decision levels at or below it are never
    /// backtracked during solving.
    pub fn root_level(&self) -> u32 {
        self.raw.root_level()
    }

    /// Whether the assignment is conflicting.
    pub fn has_conflict(&self) -> bool {
        self.raw.has_conflict()
    }

    /// The size of the solver's whole variable space: `max(numVars,
    /// numProblemVars) + trailOffset` (`clasp/src/clingo.cpp:76`, checked
    /// directly against clingo 5.8.2). **Not** how many literals are currently
    /// assigned; [`Trail::size`] (from [`Assignment::trail`]) is that count
    /// instead, and the two genuinely differ: with `CheckMode::Fixpoint` on
    /// `{a; b}.`, the first `check` sees `size() == 4` while `trail.size() ==
    /// 1`. [`Assignment::at`] enumerates every one of these `size()` positions,
    /// free ones included, not only assigned literals.
    pub fn size(&self) -> usize {
        self.raw.size()
    }

    /// Whether every literal is assigned.
    pub fn is_total(&self) -> bool {
        self.raw.is_total()
    }

    /// Whether `literal` is known to this assignment.
    ///
    /// Safe for any [`SolverLiteral`], including one legitimately obtained from
    /// a different control's grounding: it answers `false` rather than crashing
    /// (checked directly against clingo 5.8.2). This is the primitive every
    /// other literal-taking method on this type validates a literal with before
    /// reaching clingo, and it is also what [`SolverLiteral`]'s own
    /// cross-control validation guard for [`PropagateInit`]/
    /// [`PropagateControl`] is built from.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::propagate::{PropagateInit, Propagator};
    /// use clingox::{Control, Part, Result};
    ///
    /// struct CheckHasLiteral;
    /// impl Propagator for CheckHasLiteral {
    ///     fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
    ///         let lit = init.add_literal(true)?;
    ///         assert!(init.assignment().has_literal(lit));
    ///         assert!(init.assignment().has_literal(-lit));
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.register_propagator(CheckHasLiteral)?;
    /// ctl.solve(&[])?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn has_literal(&self, literal: SolverLiteral) -> bool {
        self.raw.has_literal(literal.get())
    }

    /// The decision level `literal` was assigned at, or `None` if it is known
    /// to this assignment but currently unassigned.
    ///
    /// Internally, clasp reports `u32::MAX` for the "known but unassigned" case
    /// rather than an error (`clasp/src/clingo.cpp:62-66`, checked directly);
    /// clingox does not expose that raw sentinel, wrapping it as `None` instead
    /// (RULES §4's "`Option` for 'not this variant'" rule).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` is not known to this
    /// assignment, or if an earlier [`Flow::Stop`] already stopped the
    /// [`PropagateInit`]/[`PropagateControl`] call this assignment came from.
    pub fn level(&self, literal: SolverLiteral) -> Result<Option<u32>> {
        self.check_not_stopped()?;
        if !self.has_literal(literal) {
            return Err(foreign_literal(literal));
        }
        let level = self
            .raw
            .level(literal.get())
            .map_err(|e| e.context("reading a literal's decision level"))?;
        Ok((level != u32::MAX).then_some(level))
    }

    /// The decision literal at `level`: the first literal in [`Assignment::
    /// trail`] whose own level is `level` (clingo's own note: "the first
    /// literal with a larger level than the previous literals is a
    /// decision").
    ///
    /// `decision(0)` is always clasp's own "trivially true" sentinel,
    /// regardless of the program: not a literal belonging to any particular
    /// atom, even though it happens to coincide with one in a program that
    /// has a fact (checked directly, both ways).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `level` is greater than [`Assignment::
    /// decision_level`], or if an earlier [`Flow::Stop`] already stopped the
    /// [`PropagateInit`]/[`PropagateControl`] call this assignment came
    /// from.
    pub fn decision(&self, level: u32) -> Result<SolverLiteral> {
        self.check_not_stopped()?;
        if level > self.decision_level() {
            return Err(out_of_range_level(level, self.decision_level()));
        }
        self.raw
            .decision(level)
            .map_err(|e| e.context("reading the decision literal at a level"))
    }

    /// Whether `literal` has a fixed truth value: it was assigned at
    /// decision level `0` and can never be undone by backtracking (unlike a
    /// literal genuinely decided or implied above level `0`).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` is not known to this
    /// assignment, or if an earlier [`Flow::Stop`] already stopped the
    /// [`PropagateInit`]/[`PropagateControl`] call this assignment came
    /// from.
    pub fn is_fixed(&self, literal: SolverLiteral) -> Result<bool> {
        self.check_not_stopped()?;
        if !self.has_literal(literal) {
            return Err(foreign_literal(literal));
        }
        self.raw
            .is_fixed(literal.get())
            .map_err(|e| e.context("checking whether a literal is fixed"))
    }

    /// Whether `literal` is currently true.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` is not known to this
    /// assignment, or if an earlier [`Flow::Stop`] already stopped the
    /// [`PropagateInit`]/[`PropagateControl`] call this assignment came
    /// from.
    pub fn is_true(&self, literal: SolverLiteral) -> Result<bool> {
        self.check_not_stopped()?;
        if !self.has_literal(literal) {
            return Err(foreign_literal(literal));
        }
        self.raw
            .is_true(literal.get())
            .map_err(|e| e.context("checking whether a literal is true"))
    }

    /// Whether `literal` is currently false.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` is not known to this
    /// assignment, or if an earlier [`Flow::Stop`] already stopped the
    /// [`PropagateInit`]/[`PropagateControl`] call this assignment came
    /// from.
    pub fn is_false(&self, literal: SolverLiteral) -> Result<bool> {
        self.check_not_stopped()?;
        if !self.has_literal(literal) {
            return Err(foreign_literal(literal));
        }
        self.raw
            .is_false(literal.get())
            .map_err(|e| e.context("checking whether a literal is false"))
    }

    /// The truth value of `literal`: `Some(true)`, `Some(false)`, or `None`
    /// while it is still free.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` is not known to this
    /// assignment, or if an earlier [`Flow::Stop`] already stopped the
    /// [`PropagateInit`]/[`PropagateControl`] call this assignment came
    /// from.
    pub fn truth_value(&self, literal: SolverLiteral) -> Result<Option<bool>> {
        self.check_not_stopped()?;
        if !self.has_literal(literal) {
            return Err(foreign_literal(literal));
        }
        self.raw
            .truth_value(literal.get())
            .map_err(|e| e.context("reading a literal's truth value"))
    }

    /// The (positive) literal at `offset`, in ascending order
    /// (`clingo_assignment_at(offset) = offset + 1`, a raw computed value,
    /// not search-order-dependent). Distinct from [`Trail::at`]'s own
    /// chronological order: see [`Assignment::trail`].
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `offset` is not less than
    /// [`Assignment::size`], or if an earlier [`Flow::Stop`] already stopped
    /// the [`PropagateInit`]/[`PropagateControl`] call this assignment came
    /// from.
    pub fn at(&self, offset: usize) -> Result<SolverLiteral> {
        self.check_not_stopped()?;
        if offset >= self.size() {
            return Err(out_of_range_offset(offset, self.size()));
        }
        self.raw
            .at(offset)
            .map_err(|e| e.context("reading the literal at an offset"))
    }

    /// The trail: every assigned literal, in the chronological order the
    /// solver assigned it, one level's worth at a time.
    ///
    /// # Examples
    ///
    /// See [`Trail`]'s own documentation.
    pub fn trail(&self) -> Trail<'_> {
        Trail {
            raw: self.raw,
            stopped: self.stopped,
        }
    }
}

impl fmt::Debug for Assignment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Assignment")
            .field("decision_level", &self.decision_level())
            .field("root_level", &self.root_level())
            .field("has_conflict", &self.has_conflict())
            .field("size", &self.size())
            .field("is_total", &self.is_total())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Trail

/// Every assigned literal, in the chronological order the solver assigned it: a
/// different view over the same underlying state [`Assignment`]'s own
/// `size`/`at` enumerate in ascending, numeric order (`H:1067-1123`; a fixture
/// exists where the two orders provably differ). Obtained from
/// [`Assignment::trail`], and tied to the same lifetime: it cannot outlive the
/// callback that produced the [`Assignment`] it came from.
///
/// # Examples
///
/// ```
/// use clingox::propagate::{CheckMode, PropagateControl, PropagateInit, Propagator};
/// use clingox::{Control, Part, Result};
///
/// struct ReadTrail;
/// impl Propagator for ReadTrail {
///     fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
///         init.set_check_mode(CheckMode::Total);
///         Ok(())
///     }
///
///     fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
///         let a = control.assignment();
///         let t = a.trail();
///         // The trail has exactly as many entries as the assignment.
///         assert_eq!(t.size()?, u32::try_from(a.size()).unwrap());
///         // Level 0's own slice is the fact `a`'s single literal.
///         let level0 = t.level(0)?;
///         assert_eq!(level0.len(), 1);
///         assert_eq!(t.begin(0)?, 0);
///         assert_eq!(t.end(0)?, 1);
///         assert_eq!(t.at(0)?, level0[0]);
///         // Iterating the trail yields the same literals, in order.
///         let via_iter: Vec<_> = (&t).into_iter().collect::<Result<_>>()?;
///         assert_eq!(via_iter[0], level0[0]);
///         Ok(())
///     }
/// }
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("a.")?;
/// ctl.ground(&[Part::base()])?;
/// ctl.register_propagator(ReadTrail)?;
/// ctl.solve(&[])?;
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy)]
pub struct Trail<'a> {
    raw: raw::RawAssignment<'a>,
    /// As [`Assignment::stopped`]: carried over from the [`Assignment`]
    /// [`Assignment::trail`] was called on.
    stopped: Option<&'a Cell<bool>>,
}

impl Trail<'_> {
    /// As [`Assignment::check_not_stopped`].
    fn check_not_stopped(&self) -> Result<()> {
        check_owner_not_stopped(self.stopped)
    }

    /// The number of literals in the trail (equal to [`Assignment::size`]
    /// once the assignment is total, but assigned incrementally during the
    /// search).
    ///
    /// # Errors
    ///
    /// No failure is known from clingo itself (`clingo_assignment_trail_
    /// size` reads a field), but the header gives it the same success-flag
    /// shape as every other method here, so this stays `Result` rather than
    /// assuming it forever; it does fail with [`ErrorKind::InvalidInput`] if
    /// an earlier [`Flow::Stop`] already stopped the [`PropagateInit`]/
    /// [`PropagateControl`] call this trail came from.
    pub fn size(&self) -> Result<u32> {
        self.check_not_stopped()?;
        self.raw
            .trail_size()
            .map_err(|e| e.context("reading the trail's size"))
    }

    /// The offset of the first literal at `level` (clingo's own note: "the
    /// first literal with a larger level than the previous literals is a
    /// decision").
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `level` is greater than [`Assignment::
    /// decision_level`], or if an earlier [`Flow::Stop`] already stopped the
    /// [`PropagateInit`]/[`PropagateControl`] call this trail came from.
    pub fn begin(&self, level: u32) -> Result<u32> {
        self.check_not_stopped()?;
        let decision_level = self.raw.decision_level();
        if level > decision_level {
            return Err(out_of_range_level(level, decision_level));
        }
        self.raw
            .trail_begin(level)
            .map_err(|e| e.context("reading the trail's beginning offset for a level"))
    }

    /// The offset following the last literal at `level`: the counterpart to
    /// [`Trail::begin`], so `begin(level)..end(level)` is exactly that level's
    /// own slice of the trail.
    ///
    /// Internally, clasp's own `trailEnd` never fails for `level` at or above
    /// the current decision level (it falls back to the trail's own size
    /// unconditionally, `clasp/libpotassco/src/clingo.cpp:26-30`, checked
    /// directly); clingox adds its own guard in front of it for API uniformity
    /// with [`Trail::begin`], not because clasp itself needs one here. `level
    /// == `[`Assignment::decision_level`] stays valid (the ordinary "end of the
    /// current level" case, still `Ok(`[`Trail::size`]`())`); only a `level`
    /// genuinely past it is refused.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `level` is greater than [`Assignment::
    /// decision_level`], or if an earlier [`Flow::Stop`] already stopped the
    /// [`PropagateInit`]/[`PropagateControl`] call this trail came from.
    pub fn end(&self, level: u32) -> Result<u32> {
        self.check_not_stopped()?;
        let decision_level = self.raw.decision_level();
        if level > decision_level {
            return Err(out_of_range_level(level, decision_level));
        }
        self.raw
            .trail_end(level)
            .map_err(|e| e.context("reading the trail's ending offset for a level"))
    }

    /// The (possibly negated) literal at `offset`, in chronological order.
    /// Distinct from [`Assignment::at`]'s own ascending order: see
    /// [`Assignment::trail`].
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `offset` is not less than
    /// [`Trail::size`] (which itself fails the same way once stopped, so
    /// this needs no separate check of its own).
    pub fn at(&self, offset: u32) -> Result<SolverLiteral> {
        let size = self.size()?;
        if offset >= size {
            return Err(out_of_range_offset(offset, size));
        }
        self.raw
            .trail_at(offset)
            .map_err(|e| e.context("reading the literal at a trail offset"))
    }

    /// The literals at `level`, as a convenience over [`Trail::begin`]/
    /// [`Trail::end`]/[`Trail::at`]: `(begin(level)..end(level)).map(|o|
    /// at(o))`, collected.
    ///
    /// # Errors
    ///
    /// As [`Trail::begin`]/[`Trail::end`].
    pub fn level(&self, level: u32) -> Result<Vec<SolverLiteral>> {
        let begin = self.begin(level)?;
        let end = self.end(level)?;
        (begin..end).map(|offset| self.at(offset)).collect()
    }

    /// As `(&self).into_iter()`: every literal, in chronological order.
    pub fn iter(&self) -> TrailIter<'_> {
        self.into_iter()
    }
}

impl fmt::Debug for Trail<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Trail").field("size", &self.size()).finish()
    }
}

impl<'a> IntoIterator for &'a Trail<'_> {
    type Item = Result<SolverLiteral>;
    type IntoIter = TrailIter<'a>;

    /// Yields [`Trail::at`] applied to every offset from `0` to [`Trail::
    /// size`], in chronological order. `Trail::at` is itself fallible, but
    /// only [`Trail::size`] (queried once, lazily, on the first `next`
    /// call) is expected to ever actually fail here: an in-range `at(offset)`
    /// call the iterator's own bookkeeping makes is not expected to fail in
    /// practice, but the item type stays `Result`, per RULES §11.1 (no
    /// swallowing a fallible C call's error inside infallible-looking Rust).
    fn into_iter(self) -> TrailIter<'a> {
        TrailIter {
            trail: *self,
            state: TrailIterState::Start,
        }
    }
}

/// The iterator `&`[`Trail`]'s own `IntoIterator` implementation returns.
pub struct TrailIter<'a> {
    trail: Trail<'a>,
    state: TrailIterState,
}

#[derive(Clone, Copy)]
enum TrailIterState {
    /// Not started: the first call to `next` asks the trail for its size.
    Start,
    Running {
        next: u32,
        end: u32,
    },
    Done,
}

impl Iterator for TrailIter<'_> {
    type Item = Result<SolverLiteral>;

    fn next(&mut self) -> Option<Result<SolverLiteral>> {
        let (next, end) = match self.state {
            TrailIterState::Done => return None,
            TrailIterState::Start => match self.trail.size() {
                Ok(size) => (0, size),
                Err(e) => {
                    self.state = TrailIterState::Done;
                    return Some(Err(e));
                }
            },
            TrailIterState::Running { next, end } => (next, end),
        };
        if next >= end {
            self.state = TrailIterState::Done;
            return None;
        }
        let item = self.trail.at(next);
        self.state = TrailIterState::Running {
            next: next + 1,
            end,
        };
        Some(item)
    }
}

impl std::iter::FusedIterator for TrailIter<'_> {}

impl fmt::Debug for TrailIter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrailIter").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// PropagateInit

/// Also used by `raw::propagate`'s `decide` trampoline, for the identical
/// hazard on the *output* side of a callback: a `Propagator::decide`
/// implementation can return a literal clingo never handed it, exactly as
/// unchecked as a caller passing one into `PropagateInit`/`PropagateControl`.
pub(crate) fn foreign_literal(literal: SolverLiteral) -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        format!("{literal:?} is not a literal of this control's own assignment"),
    )
}

fn already_stopped() -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        "clingo signalled Stop on an earlier call; no further call is made on this object",
    )
}

/// [`Assignment::check_not_stopped`]/[`Trail::check_not_stopped`]'s shared
/// logic, pulled out as a pure function of the
/// owning [`PropagateInit`]/[`PropagateControl`]'s own flag so it is
/// testable without a real assignment (Miri, no clingo call involved):
/// `None` (no owner, as for the assignment [`Propagator::decide`] receives
/// directly) never refuses.
fn check_owner_not_stopped(stopped: Option<&Cell<bool>>) -> Result<()> {
    if stopped.is_some_and(Cell::get) {
        Err(already_stopped())
    } else {
        Ok(())
    }
}

/// [`Assignment::at`]/[`Trail::at`]'s own out-of-range guard: clingox checks
/// `offset < limit` itself before calling clingo, refusing an out-of-range one
/// with [`ErrorKind::InvalidInput`], rather than letting clingo's own `Runtime`
/// (`Assignment::at`) or `Logic` (`Trail::at`) error through.
fn out_of_range_offset(offset: impl fmt::Display, limit: impl fmt::Display) -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        format!("offset {offset} is out of range: this view has {limit} literal(s)"),
    )
}

/// [`Assignment::decision`]/[`Trail::begin`]/[`Trail::end`]'s own
/// out-of-range guard: clingox checks `level <= decision_level` itself
/// before calling clingo (extended to `Trail::end` by the second review).
fn out_of_range_level(level: u32, decision_level: u32) -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        format!("level {level} is out of range: the current decision level is {decision_level}"),
    )
}

/// The one cross-control literal validation check every `PropagateInit`/
/// `PropagateControl` method that takes a `SolverLiteral` runs first
/// (`SolverLiteral`'s own rustdoc): shared so `PropagateInit::validate`/
/// `validate_all` and `PropagateControl::add_clause` check exactly the same
/// way, against whichever `assignment()` the caller's own type reports.
fn validate_literal(assignment: &Assignment<'_>, literal: SolverLiteral) -> Result<()> {
    if assignment.has_literal(literal) {
        Ok(())
    } else {
        Err(foreign_literal(literal))
    }
}

/// [`validate_literal`], for every literal in `literals`, checked in order
/// so a valid literal followed by an invalid one is still caught.
fn validate_literals(
    assignment: &Assignment<'_>,
    literals: impl IntoIterator<Item = SolverLiteral>,
) -> Result<()> {
    literals
        .into_iter()
        .try_for_each(|lit| validate_literal(assignment, lit))
}

/// Initializes a [`Propagator`] before a solving step: maps program literals to
/// solver literals, adds watches, and reads the atoms of the grounding for the
/// last time before the search starts.
///
/// Borrowed for exactly the duration of [`Propagator::init`]; it cannot be
/// stored past that call (a compile error, `clingox/tests/ui/
/// propagate_init_escapes_init.rs`).
///
/// **After [`Flow::Stop`]:** clingo's own header says no further call on this
/// object, or on the assignment, should be made once [`PropagateInit::
/// add_clause`], [`PropagateInit::add_weight_constraint`] or
/// [`PropagateInit::propagate`] reports [`Flow::Stop`]. Checked directly
/// against clingo 5.8.2: a further call is accepted silently rather than
/// erroring, so clingox adds its own runtime guard, refusing
/// [`PropagateInit::add_clause`], [`PropagateInit:: add_weight_constraint`],
/// [`PropagateInit::add_literal`], [`PropagateInit:: add_minimize`],
/// [`PropagateInit::propagate`], [`PropagateInit::add_watch`],
/// [`PropagateInit::add_watch_to_thread`], [`PropagateInit::remove_watch`],
/// [`PropagateInit::remove_watch_from_thread`], [`PropagateInit::
/// freeze_literal`], [`PropagateInit::solver_literal`], [`PropagateInit::
/// symbolic_atoms`] and [`PropagateInit::theory_atoms`] with [`ErrorKind::
/// InvalidInput`] before ever reaching clingo (the header names no basis for
/// excluding any of the watch methods from this list). The methods without a
/// `Result` return ([`PropagateInit::number_of_threads`], the check and undo
/// mode getters and setters) have no error channel and are exempt. Every
/// fallible [`Assignment`]/[`Trail`] query made through
/// [`PropagateInit::assignment`] is refused the same way; the five infallible
/// getters and [`Assignment:: has_literal`] stay callable (J-notes.md's
/// ruling).
pub struct PropagateInit<'i> {
    raw: raw::Init<'i>,
    stopped: Cell<bool>,
}

impl<'i> PropagateInit<'i> {
    pub(crate) fn from_raw(raw: raw::Init<'i>) -> PropagateInit<'i> {
        PropagateInit {
            raw,
            stopped: Cell::new(false),
        }
    }

    fn check_not_stopped(&self) -> Result<()> {
        if self.stopped.get() {
            Err(already_stopped())
        } else {
            Ok(())
        }
    }

    fn validate(&self, literal: SolverLiteral) -> Result<()> {
        validate_literal(&self.assignment(), literal)
    }

    fn validate_all(&self, literals: impl IntoIterator<Item = SolverLiteral>) -> Result<()> {
        validate_literals(&self.assignment(), literals)
    }

    /// Maps a program literal (a symbolic or theory atom's own literal, or a
    /// theory element's condition id) to its solver literal.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory;
    /// [`ErrorKind::InvalidInput`] if an earlier [`Flow::Stop`] already
    /// stopped this `init` call.
    ///
    /// # Examples
    ///
    /// See [the module's example](self), or the `propagate` guide chapter.
    pub fn solver_literal(&self, program_literal: ProgramLiteral) -> Result<SolverLiteral> {
        self.check_not_stopped()?;
        self.raw
            .solver_literal(program_literal.get())
            .map_err(|e| e.context("mapping a program literal to a solver literal"))
    }

    /// Watches `literal` on every solver thread: [`Propagator::propagate`]
    /// is called whenever it becomes true.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` is not known to this
    /// control's own assignment, or if an earlier [`Flow::Stop`] already
    /// stopped this `init` call (see the type's own documentation).
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::propagate::{PropagateInit, Propagator};
    /// use clingox::{Control, Part, Result, Signature};
    ///
    /// struct WatchA;
    /// impl Propagator for WatchA {
    ///     fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
    ///         let plit = init
    ///             .symbolic_atoms()?
    ///             .by_signature(Signature::new("a", 0)?)
    ///             .next()
    ///             .expect("a is an atom")?
    ///             .literal();
    ///         let slit = init.solver_literal(plit)?;
    ///         init.add_watch(slit)?;
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.register_propagator(WatchA)?;
    /// ctl.solve(&[])?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_watch(&mut self, literal: SolverLiteral) -> Result<()> {
        self.check_not_stopped()?;
        self.validate(literal)?;
        self.raw
            .add_watch(literal)
            .map_err(|e| e.context("adding a watch"))
    }

    /// As [`PropagateInit::add_watch`], but only on one solver thread.
    ///
    /// # Errors
    ///
    /// As [`PropagateInit::add_watch`] (this method is under the same
    /// post-`Stop` guard as every other one here, an earlier version of this
    /// doc comment claimed otherwise, citing a header sentence that does not
    /// exist).
    pub fn add_watch_to_thread(&mut self, literal: SolverLiteral, thread_id: u32) -> Result<()> {
        self.check_not_stopped()?;
        self.validate(literal)?;
        self.raw
            .add_watch_to_thread(literal, thread_id)
            .map_err(|e| e.context("adding a per-thread watch"))
    }

    /// Removes a watch added with [`PropagateInit::add_watch`].
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` is not known to this
    /// control's own assignment, or if an earlier [`Flow::Stop`] already
    /// stopped this `init` call.
    pub fn remove_watch(&mut self, literal: SolverLiteral) -> Result<()> {
        self.check_not_stopped()?;
        self.validate(literal)?;
        self.raw
            .remove_watch(literal)
            .map_err(|e| e.context("removing a watch"))
    }

    /// As [`PropagateInit::remove_watch`], for a watch added with
    /// [`PropagateInit::add_watch_to_thread`].
    ///
    /// # Errors
    ///
    /// As [`PropagateInit::remove_watch`].
    pub fn remove_watch_from_thread(
        &mut self,
        literal: SolverLiteral,
        thread_id: u32,
    ) -> Result<()> {
        self.check_not_stopped()?;
        self.validate(literal)?;
        self.raw
            .remove_watch_from_thread(literal, thread_id)
            .map_err(|e| e.context("removing a per-thread watch"))
    }

    /// Freezes `literal`, so it survives clingo's own preprocessing even
    /// though nothing watches it.
    ///
    /// Any watched literal is already frozen automatically; this is only
    /// needed for a literal a propagator uses in [`PropagateInit::add_clause`]
    /// (or the later [`PropagateControl::add_clause`]) without ever
    /// watching it.
    ///
    /// # Errors
    ///
    /// As [`PropagateInit::add_watch`].
    pub fn freeze_literal(&mut self, literal: SolverLiteral) -> Result<()> {
        self.check_not_stopped()?;
        self.validate(literal)?;
        self.raw
            .freeze_literal(literal)
            .map_err(|e| e.context("freezing a literal"))
    }

    /// The symbolic atoms of the current grounding.
    ///
    /// This is the last point they are reachable: once the search starts
    /// they are gone. The same [`SymbolicAtoms`] type
    /// [`Control::symbolic_atoms`](crate::Control::symbolic_atoms) returns,
    /// with a lifetime tied to this `init` call instead of the control.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory;
    /// [`ErrorKind::InvalidInput`] if an earlier [`Flow::Stop`] already
    /// stopped this `init` call.
    pub fn symbolic_atoms(&self) -> Result<SymbolicAtoms<'i>> {
        self.check_not_stopped()?;
        let atoms = self
            .raw
            .symbolic_atoms()
            .map_err(|e| e.context("reading the symbolic atoms"))?;
        Ok(SymbolicAtoms::from_parts(ErrorSink::None, atoms))
    }

    /// As [`PropagateInit::symbolic_atoms`], for the theory atoms.
    ///
    /// # Errors
    ///
    /// As [`PropagateInit::symbolic_atoms`].
    pub fn theory_atoms(&self) -> Result<TheoryAtoms<'i>> {
        self.check_not_stopped()?;
        let atoms = self
            .raw
            .theory_atoms()
            .map_err(|e| e.context("reading the theory atoms"))?;
        Ok(TheoryAtoms::from_parts(ErrorSink::None, atoms))
    }

    /// The number of threads the next solving step will use.
    ///
    /// Exempt from the post-`Stop` guard, like [`PropagateInit::check_mode`],
    /// [`PropagateInit::undo_mode`] and their setters: none returns a
    /// `Result`, so there is no error channel to refuse through.
    pub fn number_of_threads(&self) -> u32 {
        self.raw.number_of_threads()
    }

    /// When [`Propagator::check`] is called.
    pub fn check_mode(&self) -> CheckMode {
        self.raw.check_mode()
    }

    /// Sets when [`Propagator::check`] is called.
    pub fn set_check_mode(&mut self, mode: CheckMode) {
        self.raw.set_check_mode(mode);
    }

    /// When [`Propagator::undo`] is called.
    pub fn undo_mode(&self) -> UndoMode {
        self.raw.undo_mode()
    }

    /// Sets when [`Propagator::undo`] is called.
    pub fn set_undo_mode(&mut self, mode: UndoMode) {
        self.raw.set_undo_mode(mode);
    }

    /// The assignment at the start of the solving step.
    ///
    /// Every fallible query made through it is refused with [`ErrorKind::
    /// InvalidInput`] once an earlier [`Flow::Stop`] stopped this `init`
    /// call; the five infallible getters and
    /// [`Assignment::has_literal`] stay callable.
    pub fn assignment(&self) -> Assignment<'_> {
        self.raw.assignment().with_stopped(&self.stopped)
    }

    /// Adds a fresh literal to the solver.
    ///
    /// Frozen (`freeze: true`) if it must survive to be used in a later
    /// [`PropagateInit::add_clause`] within the same `init` call, or watched
    /// afterward; otherwise it may be preprocessed away.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if an earlier [`Flow::Stop`] already
    /// stopped this `init` call.
    pub fn add_literal(&mut self, freeze: bool) -> Result<SolverLiteral> {
        self.check_not_stopped()?;
        self.raw
            .add_literal(freeze)
            .map_err(|e| e.context("adding a literal"))
    }

    /// Adds `clause` to the solver.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for any literal not known to this
    /// control's own assignment, or if an earlier [`Flow::Stop`] already
    /// stopped this `init` call.
    pub fn add_clause(&mut self, clause: &[SolverLiteral]) -> Result<Flow> {
        self.check_not_stopped()?;
        self.validate_all(clause.iter().copied())?;
        let flow = self
            .raw
            .add_clause(clause)
            .map_err(|e| e.context("adding a clause"))?;
        if flow.is_stop() {
            self.stopped.set(true);
        }
        Ok(flow)
    }

    /// Adds a weight constraint `literal <=> { l=w | (l,w) in literals } >=
    /// bound` (or `<=` when `compare_equal` is set) to the solver, with `kind`
    /// choosing the direction of the `<=>`.
    ///
    /// `weight`s and `bound` are `i32`: `clingo_weight_t` is `int32_t`
    /// (`clingo.h:127`), confirmed directly for this function too (an `i64`
    /// would not match the header).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` or any weighted literal is not
    /// known to this control's own assignment, or if an earlier [`Flow::Stop`]
    /// already stopped this `init` call.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::propagate::{PropagateInit, Propagator, WeightConstraintKind};
    /// use clingox::{Control, Part, Result, Signature};
    ///
    /// struct Wc;
    /// impl Propagator for Wc {
    ///     fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
    ///         let lit = |init: &PropagateInit<'_>, name: &str| {
    ///             let sig = Signature::new(name, 0).unwrap();
    ///             let plit = init
    ///                 .symbolic_atoms()
    ///                 .unwrap()
    ///                 .by_signature(sig)
    ///                 .next()
    ///                 .unwrap()
    ///                 .unwrap()
    ///                 .literal();
    ///             init.solver_literal(plit).unwrap()
    ///         };
    ///         let (la, lb, lc) = (lit(init, "a"), lit(init, "b"), lit(init, "c"));
    ///         init.add_weight_constraint(lc, &[(la, 1), (lb, 1)], 1, WeightConstraintKind::Equivalence, false)?;
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("{a;b;c}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.register_propagator(Wc)?;
    /// ctl.solve(&[])?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_weight_constraint(
        &mut self,
        literal: SolverLiteral,
        literals: &[(SolverLiteral, i32)],
        bound: i32,
        kind: WeightConstraintKind,
        compare_equal: bool,
    ) -> Result<Flow> {
        self.check_not_stopped()?;
        self.validate(literal)?;
        self.validate_all(literals.iter().map(|&(lit, _)| lit))?;
        let flow = self
            .raw
            .add_weight_constraint(literal, literals, bound, kind, compare_equal)
            .map_err(|e| e.context("adding a weight constraint"))?;
        if flow.is_stop() {
            self.stopped.set(true);
        }
        Ok(flow)
    }

    /// Extends the solver's own minimize constraint with `literal`, as a
    /// weak constraint `:~ literal. [weight@priority]`.
    ///
    /// `weight`/`priority` are `i32` (`clingo_weight_t`).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` is not known to this
    /// control's own assignment, or if an earlier [`Flow::Stop`] already
    /// stopped this `init` call.
    pub fn add_minimize(
        &mut self,
        literal: SolverLiteral,
        weight: i32,
        priority: i32,
    ) -> Result<()> {
        self.check_not_stopped()?;
        self.validate(literal)?;
        self.raw
            .add_minimize(literal, weight, priority)
            .map_err(|e| e.context("extending the minimize constraint"))
    }

    /// Propagates the consequences of the clauses added so far, before
    /// solving proper starts.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if an earlier [`Flow::Stop`] already
    /// stopped this `init` call.
    pub fn propagate(&mut self) -> Result<Flow> {
        self.check_not_stopped()?;
        let flow = self
            .raw
            .propagate()
            .map_err(|e| e.context("propagating during init"))?;
        if flow.is_stop() {
            self.stopped.set(true);
        }
        Ok(flow)
    }
}

impl fmt::Debug for PropagateInit<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PropagateInit")
            .field("stopped", &self.stopped.get())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// PropagateControl

/// Lets [`Propagator::propagate`], [`Propagator::undo`] and
/// [`Propagator::check`] read the assignment, add clauses and literals, and
/// manage watches on the current solver thread while solving.
///
/// **`undo` receives `&PropagateControl<'_>`, every other callback receives
/// `&mut PropagateControl<'_>`** ([`Propagator::undo`]'s own signature):
/// clingo's header itself takes a `const` pointer only for `undo`
/// (`clingo_propagator_undo_callback_t`), matching its own rule for that
/// callback ("No clauses must be propagated in this callback"). Since
/// [`PropagateControl::add_clause`] takes `&mut self`, the compiler itself
/// rejects calling it from `undo`.
///
/// **Reentrancy, concretely.** [`PropagateControl::add_clause`],
/// [`PropagateControl::propagate`], [`PropagateControl::add_watch`] and
/// [`PropagateControl::add_literal`] each release clasp's own internal lock
/// for the duration of the call (`clasp/src/clingo.cpp`'s `ScopedUnlock`).
/// A conflicting `add_clause`, or a `propagate` that reaches the same
/// state, can still force a backtrack past the propagator's own watched
/// literals, calling [`Propagator::undo`] one or more times — but, checked
/// directly against the vendored clasp 5.8.2 (`clasp/src/clingo.cpp`'s own
/// `ClingoPropagator::Control` always sets `state_ctrl`, which makes
/// `add_clause`'s and `propagate`'s own backtrack resolve only after
/// control returns to clasp, never inside the call itself), that `undo`
/// never runs while the call that triggered it is still on the same
/// thread's own stack: the backjump is deferred to clasp's ordinary,
/// outer search loop. **This crate's soundness never depended on which way
/// that went.** `Propagator`'s methods take `&self`, are `Send + Sync`, and
/// no lock of clingox's own is ever held across a call into
/// `PropagateControl` — a design that stays sound whether `undo` is ever
/// called back synchronously or not, and costs nothing when it is not.
/// **Still never hold a lock of your own across a call into any of the four
/// methods above**: a future clasp version could change this, and a
/// `Mutex` your propagator locked in `propagate` and tried to lock again
/// from a reentrant `undo` would deadlock on the same thread, rather than
/// merely race. `clingox/tests/propagator_reentrancy.rs` pins the currently
/// observed, non-reentrant behaviour directly, under `TSan`.
pub struct PropagateControl<'c> {
    raw: raw::RawPropagateControl<'c>,
    stopped: Cell<bool>,
}

impl<'c> PropagateControl<'c> {
    pub(crate) fn from_raw(raw: raw::RawPropagateControl<'c>) -> PropagateControl<'c> {
        PropagateControl {
            raw,
            stopped: Cell::new(false),
        }
    }

    /// The id of the solver thread running this callback.
    ///
    /// Consecutive numbers from zero; matches [`PropagateInit::
    /// number_of_threads`]'s own count.
    pub fn thread_id(&self) -> u32 {
        self.raw.thread_id()
    }

    /// The assignment of the solver thread running this callback.
    ///
    /// Every fallible query made through it is refused with [`ErrorKind::
    /// InvalidInput`] once an earlier [`Flow::Stop`] stopped this
    /// `propagate`/`undo`/`check` call; the five
    /// infallible getters and [`Assignment::has_literal`] stay callable.
    pub fn assignment(&self) -> Assignment<'_> {
        self.raw.assignment().with_stopped(&self.stopped)
    }

    fn check_not_stopped(&self) -> Result<()> {
        if self.stopped.get() {
            Err(already_stopped())
        } else {
            Ok(())
        }
    }

    fn validate(&self, literal: SolverLiteral) -> Result<()> {
        validate_literal(&self.assignment(), literal)
    }

    /// Adds `clause` to the solver, with `kind` choosing how long it
    /// survives.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for any literal not known to this
    /// control's own assignment (the same cross-control validation rule as
    /// every literal-taking [`PropagateInit`] method), or if an earlier
    /// [`Flow::Stop`] already stopped this `propagate`/`undo`/`check` call.
    ///
    /// # Examples
    ///
    /// See the [module documentation](self) for a propagator that does
    /// nothing; a propagator that calls `add_clause` from `propagate`
    /// mirrors pyclingo's own `AIFFB` example, ported as `clingox/tests/
    /// api_propagator_control.rs::aiffb_forces_a_and_b_to_agree`.
    pub fn add_clause(&mut self, clause: &[SolverLiteral], kind: ClauseType) -> Result<Flow> {
        self.check_not_stopped()?;
        validate_literals(&self.assignment(), clause.iter().copied())?;
        let flow = self
            .raw
            .add_clause(clause, kind)
            .map_err(|e| e.context("adding a clause"))?;
        if flow.is_stop() {
            self.stopped.set(true);
        }
        Ok(flow)
    }

    /// Adds a new volatile literal to the underlying solver thread.
    ///
    /// This literal is valid only for the current solving step and solver
    /// thread: unlike [`PropagateInit::add_literal`]'s own literal (whose
    /// `freeze` parameter can make it survive further), there is no way to keep
    /// this one usable past the step that created it. Reusing it in a later
    /// step is a real validity boundary, not merely a performance note:
    /// unguarded, it segfaults the process (checked directly against clingo
    /// 5.8.2). No extra guard is needed to close this off, though: every other
    /// literal-taking method on this type validates its own literal against a
    /// *fresh* `assignment()` first, and a stale literal from an earlier step
    /// reports `false` there, exactly like a literal from a different control.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, or
    /// [`ErrorKind::InvalidInput`] if an earlier [`Flow::Stop`] already stopped
    /// this `propagate`/`undo`/`check` call. (clingo's own header also lists
    /// `ErrorKind::Logic` "if the assignment is conflicting," but the
    /// post-`Stop` guard here refuses the call before that case is ever
    /// reached.)
    ///
    /// # Examples
    ///
    /// See [`PropagateControl::add_watch`] for `add_literal` used together with
    /// `add_watch`/`has_watch`/`remove_watch`, the ported
    /// `libpyclingo::test_propagator` shape.
    pub fn add_literal(&mut self) -> Result<SolverLiteral> {
        self.check_not_stopped()?;
        self.raw
            .add_literal()
            .map_err(|e| e.context("adding a literal"))
    }

    /// Watches `literal` on the current solver thread only.
    ///
    /// Unlike [`PropagateInit::add_watch`], which by default watches on
    /// every solver thread, a watch added here affects only the thread
    /// running this call: watching the same literal from two different
    /// threads' own `propagate` calls needs two separate calls, one per
    /// thread, and removing one (see [`PropagateControl::remove_watch`])
    /// never affects another thread's own watch.
    ///
    /// **A known, benign clasp-internal data race (U28,
    /// `docs/dev/UPSTREAM-ISSUES.md`).** Calling this (or [`PropagateControl::
    /// has_watch`]/[`PropagateControl::remove_watch`]) from more than one
    /// solver thread of a propagator registered with [`Control::
    /// register_propagator`] (not `_sequential`) makes clasp read its master
    /// solver's own assignment word with no lock, which can race a write
    /// from that same thread's ordinary decision-making. `cargo xtask
    /// sanitize` suppresses this one, narrowly: the bit actually read (a
    /// variable's elimination mark) is fixed before the search starts and
    /// never changes again, so the value observed is always correct in
    /// practice, but it is a genuine data race in C++'s own formal sense.
    /// [`Control::register_propagator_sequential`] avoids it entirely, by
    /// serialising every call into the propagator.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` is not known to this
    /// control's own assignment, or if an earlier [`Flow::Stop`] already
    /// stopped this `propagate`/`undo`/`check` call. Neither failure mode is
    /// forced by a crash risk here (clingo itself already reports a clean
    /// error for both, `clasp/src/clingo.cpp:138-155`); this method
    /// validates anyway, for the same "checked once, in one place" rule
    /// every literal-taking method in this module follows.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::propagate::{PropagateControl, PropagateInit, Propagator};
    /// use clingox::{Control, Part, Result};
    /// use std::sync::Mutex;
    ///
    /// // A fresh literal from `add_literal` is itself unassigned, so it
    /// // stays free of every model's own atoms; running this only once
    /// // (`added`) matters here, not just for tidiness: `check` fires again
    /// // whenever the assignment is total, and a *second* call would add
    /// // another free literal, making the assignment total again and
    /// // firing `check` again, forever.
    /// #[derive(Default)]
    /// struct AddedLiteralRoundTrip {
    ///     added: Mutex<bool>,
    /// }
    /// impl Propagator for AddedLiteralRoundTrip {
    ///     fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
    ///         let mut added = self.added.lock().unwrap();
    ///         if !*added {
    ///             *added = true;
    ///             let lit = control.add_literal()?;
    ///             assert!(!control.has_watch(lit));
    ///             control.add_watch(lit)?;
    ///             assert!(control.has_watch(lit));
    ///             control.remove_watch(lit)?;
    ///             assert!(!control.has_watch(lit));
    ///         }
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.register_propagator(AddedLiteralRoundTrip::default())?;
    /// ctl.solve(&[])?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_watch(&mut self, literal: SolverLiteral) -> Result<()> {
        self.check_not_stopped()?;
        self.validate(literal)?;
        self.raw
            .add_watch(literal)
            .map_err(|e| e.context("adding a watch"))
    }

    /// Whether `literal` is watched on the current solver thread.
    ///
    /// `false` for a literal not known to this control's own assignment,
    /// validated first and never reaching clingo for that case. This is
    /// observably identical to clingo's own behaviour for a foreign literal
    /// here (checked directly), so
    /// no black-box test can tell the two apart; the guard exists only so
    /// this method never hands clingo an unvalidated literal, the same
    /// "checked once, in one place" rule every literal-taking method here
    /// follows. Unlike every other fallible method on this type, this one
    /// stays a plain `bool`, matching `clingo_propagate_control_has_watch`'s
    /// own `bool`-returning-directly shape (never a success flag).
    ///
    /// `false`, unconditionally and without reaching clingo, once an
    /// earlier [`Flow::Stop`] already stopped this `propagate`/`undo`/
    /// `check` call: unlike every other guarded
    /// method here, this refuses by returning `false` rather than
    /// `Err(ErrorKind::InvalidInput)`, since the method itself is
    /// infallible. clingo does not clear watches on `Stop`, so its own
    /// answer for a literal watched before `Stop` would otherwise still be
    /// `true`; this guard is the only thing that makes the observable
    /// answer `false` here.
    ///
    /// # Examples
    ///
    /// See [`PropagateControl::add_watch`].
    pub fn has_watch(&self, literal: SolverLiteral) -> bool {
        !self.stopped.get() && self.validate(literal).is_ok() && self.raw.has_watch(literal)
    }

    /// Removes a watch added with [`PropagateControl::add_watch`] on the
    /// current solver thread; a watch added through [`PropagateInit::
    /// add_watch`] on every thread is unaffected on every *other* thread.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if `literal` is not known to this
    /// control's own assignment, or if an earlier [`Flow::Stop`] already
    /// stopped this `propagate`/`undo`/`check` call. As with
    /// [`PropagateControl::add_watch`], clingo itself is already safe for
    /// either case (it runs to completion with no observable effect,
    /// `clasp/src/clingo.cpp:134-161`); this method still validates, for the
    /// same uniformity reason.
    ///
    /// # Examples
    ///
    /// See [`PropagateControl::add_watch`].
    pub fn remove_watch(&mut self, literal: SolverLiteral) -> Result<()> {
        self.check_not_stopped()?;
        self.validate(literal)?;
        self.raw.remove_watch(literal);
        Ok(())
    }

    /// Propagates the consequences of the clauses added so far during this
    /// call, before the outer `propagate`/`undo`/`check` call returns.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, or
    /// [`ErrorKind::InvalidInput`] if an earlier [`Flow::Stop`] already stopped
    /// this call.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::propagate::{Flow, PropagateControl, PropagateInit, Propagator};
    /// use clingox::{Control, Part, Result};
    ///
    /// struct PropagatesEagerly;
    /// impl Propagator for PropagatesEagerly {
    ///     fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
    ///         // Nothing was added this call, so this succeeds immediately.
    ///         assert_eq!(control.propagate()?, Flow::Continue);
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.register_propagator(PropagatesEagerly)?;
    /// ctl.solve(&[])?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn propagate(&mut self) -> Result<Flow> {
        self.check_not_stopped()?;
        let flow = self.raw.propagate().map_err(|e| e.context("propagating"))?;
        if flow.is_stop() {
            self.stopped.set(true);
        }
        Ok(flow)
    }
}

impl fmt::Debug for PropagateControl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PropagateControl")
            .field("thread_id", &self.thread_id())
            .field("stopped", &self.stopped.get())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Propagator

/// A custom theory extending the search.
///
/// Every method takes `&self`, never `&mut self`: see the [module
/// documentation](self)'s reentrancy section for why, and keep any state a
/// method needs to update in an interior-mutable container chosen to
/// tolerate it (an atomic, a `OnceLock`, or, for state private to one
/// solver thread, a `Cell`/`RefCell` inside a value read through
/// [`PropagateControl::thread_id`]).
///
/// `Send + Sync` because two different solver threads can call `propagate`
/// on the very same registered propagator's `&self` at once when more than
/// one thread is solving (DESIGN S11, S12); a compile-fail case pins this
/// (`clingox/tests/ui/propagator_not_sync_is_rejected.rs`).
///
/// Every method is defaulted to a no-op, so a propagator that only needs
/// `init` implements just that one; `propagate`, `undo`, `check` and
/// `decide` are all declared here, with the dispatch wired for every
/// one of them, so a type implementing any subset compiles and runs
/// correctly.
///
/// # Examples
///
/// See the [module documentation](self).
pub trait Propagator: Send + Sync {
    /// Runs once before each solving step: the place to map program
    /// literals to solver literals, add watches, and read the last state of
    /// the symbolic/theory atoms.
    ///
    /// # Errors
    ///
    /// Any error this implementation returns poisons the whole `Control`
    /// unconditionally and is reported by whichever call started the
    /// solving step.
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let _ = init;
        Ok(())
    }

    /// Runs during propagation with a non-empty change set: the watched
    /// solver literals that became true since the last call. Add clauses,
    /// literals and watches through `control`.
    ///
    /// # Errors
    ///
    /// Any error this implementation returns stops the solving step and is
    /// reported by whichever call is running it.
    fn propagate(
        &self,
        control: &mut PropagateControl<'_>,
        changes: &[SolverLiteral],
    ) -> Result<()> {
        let _ = (control, changes);
        Ok(())
    }

    /// Runs whenever the solver undoes assignments to watched literals, to
    /// update assignment-dependent state. Infallible at the C level
    /// (`clingo_propagator_undo_callback_t` returns nothing): a panic here
    /// is still caught and resumed once the outer call that led here
    /// returns, never unwinding into clingo's C++ frames.
    fn undo(&self, control: &PropagateControl<'_>, changes: &[SolverLiteral]) {
        let _ = (control, changes);
    }

    /// Runs on a propagation fixpoint or a total assignment, as
    /// [`PropagateInit::set_check_mode`] configures; called even if no
    /// watches were added.
    ///
    /// # Errors
    ///
    /// As [`Propagator::propagate`].
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        let _ = control;
        Ok(())
    }

    /// Implements a domain-specific decision heuristic: called whenever
    /// propagation reaches a fixpoint, to choose a free solver literal to
    /// assign true. `None` (the default) declines: clingo asks the next
    /// registered propagator with a heuristic, in registration order, and
    /// falls back to its own choice, `fallback`, only once every
    /// registered propagator has declined
    /// (`clingo_propagator_t::decide`'s own documented "return 0 to let a
    /// propagator registered later make a decision").
    ///
    /// **Returning `Some(fallback)` is a real, deliberate choice, not a
    /// decline**, even though the literal is the same one clingo would
    /// have picked anyway: it stops the chain there, and no
    /// later-registered propagator's own `decide` is consulted for this
    /// decision point. A propagator that genuinely has no opinion must
    /// return `None`, never `Some(fallback)`, or it silently blocks every
    /// propagator registered after it (returning the bare `fallback`
    /// literal is indistinguishable, at the C level, from choosing it).
    ///
    /// # Errors
    ///
    /// As [`Propagator::propagate`].
    fn decide(
        &self,
        thread_id: u32,
        assignment: &Assignment<'_>,
        fallback: SolverLiteral,
    ) -> Result<Option<SolverLiteral>> {
        let _ = (thread_id, assignment, fallback);
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Registration

impl ScopedControl<'_> {
    /// Registers a propagator, run concurrently across solver threads when more
    /// than one is used (the default; see
    /// [`Control::register_propagator_sequential`] for the alternative).
    ///
    /// Every registered propagator runs, in registration order (checked
    /// directly against clingo 5.8.2: two `register_propagator` calls both had
    /// their `init` invoked, in the order they were registered); clingo never
    /// removes a registration on its own, so it lasts for the rest of the
    /// control's life.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    ///
    /// A failing or panicking `init` (from this or any other registered
    /// propagator) poisons the control unconditionally, like a ground callback
    /// (DESIGN S3): `solve` then reports the error (with its own kind
    /// unchanged) or resumes the panic, and every later call answers
    /// [`ErrorKind::Poisoned`]. Checked directly: a propagator registered
    /// *before* the failing one has already completed `init` and stays that
    /// way; one registered *after* never runs.
    ///
    /// # Examples
    ///
    /// See the [module documentation](self).
    pub fn register_propagator(&mut self, propagator: impl Propagator + 'static) -> Result<()> {
        self.core.guarded(
            || "registering a propagator".to_owned(),
            |handle| handle.register_propagator(propagator, false),
        )
    }

    /// As [`Control::register_propagator`], but the propagator's callbacks
    /// are called sequentially even when solving with several threads:
    /// clasp takes a lock of its own around every call into it, trading
    /// propagation throughput for simpler code that does not need its own
    /// thread-safe state. Prefer plain [`Control::register_propagator`]
    /// unless a propagator genuinely needs shared, non-per-thread state.
    ///
    /// # Errors
    ///
    /// As [`Control::register_propagator`].
    pub fn register_propagator_sequential(
        &mut self,
        propagator: impl Propagator + 'static,
    ) -> Result<()> {
        self.core.guarded(
            || "registering a propagator (sequential)".to_owned(),
            |handle| handle.register_propagator(propagator, true),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solver_literal_debug_matches_program_literal_style() {
        let lit = SolverLiteral::from_raw_valid(3);
        assert_eq!(format!("{lit:?}"), "SolverLiteral(3)");
        assert_eq!(format!("{:?}", lit.negate()), "SolverLiteral(-3)");
    }

    #[test]
    fn flow_is_stop() {
        assert!(Flow::Stop.is_stop());
        assert!(!Flow::Continue.is_stop());
        assert_eq!(Flow::from_continue(true), Flow::Continue);
        assert_eq!(Flow::from_continue(false), Flow::Stop);
    }

    // The out-of-range guards, and the post-Stop guard for
    // `Assignment`/`Trail`, run entirely in the safe layer, before any C
    // call: the helpers that build their errors are pure functions of a
    // value and a limit (or a shared flag), testable without a real
    // assignment (a call into raw clingo needs a real `clingo_assignment_t`,
    // unlike `raw::trampoline`'s own fake error state, so this is as close
    // to a Miri-testable unit as this module's own logic gets).

    #[test]
    fn out_of_range_offset_is_invalid_input() {
        let err = out_of_range_offset(5_usize, 3_usize);
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn out_of_range_level_is_invalid_input() {
        let err = out_of_range_level(5, 3);
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn foreign_literal_is_invalid_input() {
        let err = foreign_literal(SolverLiteral::from_raw_valid(7));
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn check_owner_not_stopped_reads_the_shared_flag() {
        assert!(
            check_owner_not_stopped(None).is_ok(),
            "no owner (decide's own assignment): never refused"
        );

        let flag = Cell::new(false);
        assert!(check_owner_not_stopped(Some(&flag)).is_ok());

        flag.set(true);
        assert_eq!(
            check_owner_not_stopped(Some(&flag)).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );

        // The flag is shared, not copied: flipping it back is seen too, exactly
        // as a live `&Cell<bool>` threaded through `Assignment`/ `Trail` must
        // (a query made after the flag flips must see the *current* value, not
        // the one at construction).
        flag.set(false);
        assert!(check_owner_not_stopped(Some(&flag)).is_ok());
    }
}

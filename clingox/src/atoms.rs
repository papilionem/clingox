//! The symbolic atoms of a grounding, their program literals, and
//! [`Model::is_true`].

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::fmt;

use crate::control::ErrorSink;
use crate::convert::Predicate;
use crate::error::Result;
use crate::model::Model;
use crate::raw::{self, AtomData, AtomIterator, Atoms};
use crate::signature::Signature;
use crate::symbol::Symbol;

/// A program literal: the number clingo gives an atom of the ground program.
///
/// It is a separate id space from solver literals (DESIGN S17). The literals
/// of symbolic atoms are positive and distinct; [`Model::is_true`] reads one
/// in a model.
///
/// A literal is only meaningful for the control and the grounding it came
/// from. Another control numbers its atoms independently, so there it names
/// an unrelated atom or none. clingox cannot detect that; the result is
/// wrong but never unsafe ([`Model::is_true`] reads a literal clingo does not
/// know as false).
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part, Symbol};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("{a}.")?;
/// ctl.ground(&[Part::base()])?;
/// let a = Symbol::function("a", &[])?;
/// let atom = ctl.symbolic_atoms()?.find(a)?.expect("a is an atom");
/// assert!(atom.literal().get() > 0);
/// # Ok::<(), clingox::Error>(())
/// ```
///
/// `#[repr(transparent)]` over `clingo_literal_t` (i32) lets a ground
/// program observer's trampolines (`raw::observer`) hand a caller's own
/// [`GroundProgramObserver`](crate::observer::GroundProgramObserver) clingo's
/// own literal arrays (a rule's body, an element's condition, and the rest)
/// as a borrowed `&[ProgramLiteral]`, without a copy, the same way
/// [`Symbol`] is `repr(transparent)` for its own arrays. **Not**
/// [`TheoryElement::condition`](crate::TheoryElement::condition) itself:
/// that one copies, because clingo fills every element's condition into one
/// scratch buffer it reuses and reallocates for any element, so a borrow of
/// it could not outlive the next call that reads a different element's
/// condition.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProgramLiteral(i32);

impl ProgramLiteral {
    /// The number clingo uses for the literal.
    pub fn get(self) -> i32 {
        self.0
    }

    /// The largest magnitude a literal can have: clasp keeps a *variable* below
    /// 2^30 (`varMax` in clasp's `literal.h`), but as soon as a value is used
    /// as a *literal* rather than a bare variable, clasp treats 2^28 and above
    /// as a body id instead of an atom id (`clasp/src/logic_program.cpp`). A
    /// literal at or beyond that narrower bound reached clasp's own body-id
    /// numbering and failed there instead -- `Model::is_true` with
    /// `ErrorKind::Logic`, `Backend::add_rule` with `ErrorKind::Runtime`,
    /// checked directly against clingo 5.8.2 -- so clingox now matches clasp's
    /// atom range, not its wider variable range.
    pub const MAX_MAGNITUDE: i32 = (1 << 28) - 1;

    /// Builds a literal from a raw, signed clingo literal value.
    ///
    /// The value is accepted when its magnitude is between 1 and
    /// [`MAX_MAGNITUDE`](ProgramLiteral::MAX_MAGNITUDE):
    /// - `0` is never a literal, because the sign carries the truth value;
    /// - clasp cannot represent an atom id of 2^28 or more as a literal (a
    ///   value that large collides with clasp's body-id numbering instead),
    ///   so a larger magnitude can never name an atom. A value beyond even
    ///   the old, wider 2^30 - 1 bound made clingo allocate about 19 GB
    ///   before reporting "Id out of range" (clingo 5.8.2,
    ///   `docs/dev/UPSTREAM-ISSUES.md` U22). Rejecting both keeps
    ///   [`ProgramLiteral::negate`] and [`Neg`](std::ops::Neg) total.
    ///
    /// A value inside the range need not name a real atom: a literal is only
    /// meaningful for the control and grounding it came from, and clingo gives
    /// some unused literals a meaning (see the type's documentation). Note
    /// that clingo's memory use grows with the magnitude of a literal that is
    /// out of the program's range, at roughly 9 bytes per unit, so a literal
    /// far beyond the program's atoms is expensive even when it is accepted.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ProgramLiteral;
    ///
    /// assert!(ProgramLiteral::from_raw(0).is_none());
    /// assert!(ProgramLiteral::from_raw(1 << 30).is_none());
    /// assert!(ProgramLiteral::from_raw(i32::MIN).is_none());
    /// assert_eq!(ProgramLiteral::from_raw(-7).unwrap().get(), -7);
    /// ```
    #[must_use]
    pub fn from_raw(raw: i32) -> Option<ProgramLiteral> {
        (raw != 0 && raw.unsigned_abs() <= Self::MAX_MAGNITUDE.unsigned_abs())
            .then_some(ProgramLiteral(raw))
    }

    /// [`ProgramLiteral::from_raw`] for a value the caller already knows is
    /// valid, such as a backend atom's own id: an atom id is never `0` and
    /// clasp keeps every atom and literal below
    /// [`ProgramLiteral::MAX_MAGNITUDE`], so no caller within the crate needs
    /// to handle failure here. The invariant is still checked in debug builds.
    pub(crate) fn from_valid(raw: i32) -> ProgramLiteral {
        debug_assert!(
            raw != 0 && raw.unsigned_abs() <= Self::MAX_MAGNITUDE.unsigned_abs(),
            "a value that should already be a valid program literal is not: {raw}"
        );
        ProgramLiteral(raw)
    }

    /// As [`ProgramLiteral::from_raw`], but without the
    /// [`MAX_MAGNITUDE`](ProgramLiteral::MAX_MAGNITUDE) check: only for
    /// [`TheoryElement::condition_id`](crate::TheoryElement::condition_id),
    /// whose own value is documented as "not necessarily an aspif literal" and,
    /// checked directly against clingo 5.8.2, is a real id from clasp's body-id
    /// range (`2^28` and above) -- exactly the range
    /// [`MAX_MAGNITUDE`](ProgramLiteral::MAX_MAGNITUDE) now excludes for an
    /// ordinary literal, since it is narrowed to clasp's atom range. Rejects
    /// only `0`, `ProgramLiteral`'s own unconditional invariant (the sign still
    /// carries a truth value for an ordinary literal, even though a condition
    /// id is not one).
    pub(crate) fn from_nonzero(raw: i32) -> Option<ProgramLiteral> {
        (raw != 0).then_some(ProgramLiteral(raw))
    }

    /// Whether the literal is positive.
    ///
    /// Literals of symbolic atoms are always positive
    /// ([`SymbolicAtom::literal`]); a literal built with
    /// [`ProgramLiteral::from_raw`] or [`ProgramLiteral::negate`] can be
    /// either sign.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ProgramLiteral;
    ///
    /// assert!(ProgramLiteral::from_raw(7).unwrap().is_positive());
    /// assert!(!ProgramLiteral::from_raw(-7).unwrap().is_positive());
    /// ```
    pub fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// The literal with the opposite sign.
    ///
    /// It is its own inverse: `l.negate().negate() == l`. `i32::MIN` can
    /// never appear here, because nothing that produces a `ProgramLiteral`
    /// (a symbolic atom's literal, [`ProgramLiteral::from_raw`], or this
    /// method itself) can produce it (see [`ProgramLiteral::from_raw`]), so
    /// the negation never overflows.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ProgramLiteral;
    ///
    /// let lit = ProgramLiteral::from_raw(7).unwrap();
    /// assert_eq!(lit.negate().get(), -7);
    /// assert_eq!(lit.negate().negate(), lit);
    /// ```
    #[must_use]
    pub fn negate(self) -> ProgramLiteral {
        // The invariant `self.0 != i32::MIN` (see `from_raw`) makes this
        // negation total; a broken invariant would panic here in a debug
        // build rather than silently wrap.
        debug_assert_ne!(self.0, i32::MIN, "a ProgramLiteral is never i32::MIN");
        ProgramLiteral(-self.0)
    }
}

impl std::ops::Neg for ProgramLiteral {
    type Output = ProgramLiteral;

    /// Same as [`ProgramLiteral::negate`].
    fn neg(self) -> ProgramLiteral {
        self.negate()
    }
}

/// Converts the raw literals of an unsat core
/// (`raw::ControlHandle::solve_core`) into [`ProgramLiteral`]s, for
/// [`SolveHandle::core`] and [`AsyncSolveHandle::core`].
///
/// Every literal an unsat core reports is one of the literals passed to
/// `solve_yield`/`solve_async` as an assumption, so it is already a valid
/// [`ProgramLiteral`] (nonzero and within [`ProgramLiteral::MAX_MAGNITUDE`]):
/// `assumption_literals` only ever produces such literals.
///
/// [`SolveHandle::core`]: crate::SolveHandle::core
/// [`AsyncSolveHandle::core`]: crate::AsyncSolveHandle::core
pub(crate) fn literals_from_core(core: Vec<i32>) -> Vec<ProgramLiteral> {
    core.into_iter()
        .filter_map(|raw| {
            let literal = ProgramLiteral::from_raw(raw);
            debug_assert!(
                literal.is_some(),
                "clingo reported an out-of-range literal in the unsat core: {raw}"
            );
            literal
        })
        .collect()
}

/// One atom of the grounding: its symbol, its program literal, and whether it
/// is a fact or an external.
///
/// It is a plain value read when the atom was listed or found, and describes
/// the grounding at that time: after a later part adds `p.`, an atom `p` from
/// a choice rule reads as a fact, as clingo reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SymbolicAtom {
    symbol: Symbol,
    literal: ProgramLiteral,
    fact: bool,
    external: bool,
}

impl SymbolicAtom {
    fn from_data(data: AtomData) -> SymbolicAtom {
        SymbolicAtom {
            symbol: Symbol::from_clingo(data.symbol),
            literal: ProgramLiteral(data.literal),
            fact: data.fact,
            external: data.external,
        }
    }

    /// The atom's symbol.
    pub fn symbol(&self) -> Symbol {
        self.symbol
    }

    /// The atom's program literal.
    pub fn literal(&self) -> ProgramLiteral {
        self.literal
    }

    /// Whether clingo knows the atom to be a fact.
    pub fn is_fact(&self) -> bool {
        self.fact
    }

    /// Whether the atom is an external (`#external`).
    pub fn is_external(&self) -> bool {
        self.external
    }
}

/// A view of the atoms of a control's current grounding, in clingo's order.
///
/// It and its iterators borrow the control (DESIGN S5), so the control cannot
/// ground while one is alive. [`Control::symbolic_atoms`] makes one.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part, Signature};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("p(1). p(2). {q}.")?;
/// ctl.ground(&[Part::base()])?;
/// let atoms = ctl.symbolic_atoms()?;
/// assert_eq!(atoms.len()?, 3);
/// let mut facts = 0;
/// for atom in atoms.by_signature(Signature::new("p", 1)?) {
///     facts += usize::from(atom?.is_fact());
/// }
/// assert_eq!(facts, 2);
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct SymbolicAtoms<'c> {
    control: ErrorSink<'c>,
    atoms: Atoms<'c>,
}

impl<'c> SymbolicAtoms<'c> {
    /// Builds one from an already-obtained raw view, for a source other than
    /// [`Control::symbolic_atoms`]: [`PropagateInit::symbolic_atoms`] has no
    /// `Control` to poison through (`ErrorSink::None`), only the raw atoms view
    /// from `clingo_propagate_init_symbolic_atoms`.
    ///
    /// [`PropagateInit::symbolic_atoms`]: crate::propagate::PropagateInit::symbolic_atoms
    pub(crate) fn from_parts(control: ErrorSink<'c>, atoms: Atoms<'c>) -> SymbolicAtoms<'c> {
        SymbolicAtoms { control, atoms }
    }
}

impl ScopedControl<'_> {
    /// A view of the atoms of the current grounding.
    ///
    /// A search left open by a forgotten handle is closed first (DESIGN S4).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`](crate::ErrorKind::Poisoned) if an earlier
    ///   error poisoned the control;
    /// - [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs
    ///   out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a. b :- a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// assert_eq!(ctl.symbolic_atoms()?.len()?, 2);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn symbolic_atoms(&self) -> Result<SymbolicAtoms<'_>> {
        let atoms = self.core.observed(
            || "reading the symbolic atoms".to_owned(),
            raw::ControlHandle::symbolic_atoms,
        )?;
        Ok(SymbolicAtoms {
            control: ErrorSink::Control(&self.core),
            atoms,
        })
    }
}

impl<'c> SymbolicAtoms<'c> {
    /// The number of atoms.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`](crate::ErrorKind::Poisoned) if an earlier
    ///   error poisoned the control;
    /// - [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs
    ///   out of memory.
    #[expect(
        clippy::len_without_is_empty,
        reason = "the count comes from clingo and can fail, so it is a Result, \
                  and `iter().next().is_none()` already tells emptiness"
    )]
    pub fn len(&self) -> Result<usize> {
        self.read("counting the symbolic atoms", Atoms::size)
    }

    /// Iterates over every atom. Each item is read as the iterator advances;
    /// an error is yielded as an `Err` item, after which the iterator ends.
    pub fn iter(&self) -> SymbolicAtomIter<'c> {
        SymbolicAtomIter::new(self.control, self.atoms, None)
    }

    /// Iterates over the atoms of one predicate. The sign is part of the
    /// signature, so `n/1` and `-n/1` list different atoms.
    pub fn by_signature(&self, signature: Signature) -> SymbolicAtomIter<'c> {
        SymbolicAtomIter::new(self.control, self.atoms, Some(signature))
    }

    /// Every atom of the predicate `T` in the grounding, as values of `T`: the
    /// typed counterpart of [`by_signature`](SymbolicAtoms::by_signature), as
    /// [`Model::atoms`] is for a model.
    ///
    /// The atoms are those with the name [`T::NAME`](Predicate::NAME), the
    /// arity [`T::ARITY`](Predicate::ARITY) and the positive sign, facts,
    /// choices and derived atoms alike, whatever their truth value. The values
    /// come in [`Symbol`]'s order.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Conversion`](crate::ErrorKind::Conversion) for an atom
    ///   that does not convert to `T`, such as `p(a)` for a `T` that takes a
    ///   number; nothing is skipped;
    /// - [`ErrorKind::Nul`](crate::ErrorKind::Nul) if `T::NAME` has a NUL
    ///   byte;
    /// - otherwise as [`SymbolicAtoms::len`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, FromSymbol, Part};
    ///
    /// #[derive(FromSymbol, Debug, PartialEq)]
    /// struct Edge(i32, i32);
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("edge(2,3). {edge(1,2)}. node(1).")?;
    /// ctl.ground(&[Part::base()])?;
    /// let edges = ctl.symbolic_atoms()?.of::<Edge>()?;
    /// assert_eq!(edges, [Edge(1, 2), Edge(2, 3)]);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn of<T: Predicate>(&self) -> Result<Vec<T>> {
        let signature = Signature::new(T::NAME, T::ARITY)?;
        let mut symbols = Vec::new();
        for atom in self.by_signature(signature) {
            symbols.push(atom?.symbol());
        }
        symbols.sort_unstable();
        symbols.into_iter().map(T::from_symbol).collect()
    }

    /// The atom for `symbol`, or `None` if it is not an atom of the grounding
    /// (a number, for example).
    ///
    /// # Errors
    ///
    /// As [`SymbolicAtoms::len`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#external e.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let atoms = ctl.symbolic_atoms()?;
    /// let e = atoms.find(Symbol::function("e", &[])?)?.expect("e is an atom");
    /// assert!(e.is_external());
    /// assert_eq!(atoms.find(Symbol::number(1))?, None);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn find(&self, symbol: Symbol) -> Result<Option<SymbolicAtom>> {
        self.read("finding a symbolic atom", |atoms| {
            atoms
                .find(symbol.raw())
                .map(|found| found.map(SymbolicAtom::from_data))
        })
    }

    /// Every predicate signature clingo knows, including predicates without
    /// atoms, such as one whose rules never fire.
    ///
    /// # Errors
    ///
    /// As [`SymbolicAtoms::len`].
    pub fn signatures(&self) -> Result<Vec<Signature>> {
        self.read("listing the signatures", |atoms| {
            atoms
                .signatures()
                .map(|all| all.into_iter().map(Signature::from_clingo).collect())
        })
    }

    fn read<T>(&self, context: &str, f: impl FnOnce(Atoms<'c>) -> Result<T>) -> Result<T> {
        self.control
            .observed(|| context.to_owned(), || f(self.atoms))
    }
}

impl<'c> IntoIterator for &SymbolicAtoms<'c> {
    type Item = Result<SymbolicAtom>;
    type IntoIter = SymbolicAtomIter<'c>;

    fn into_iter(self) -> SymbolicAtomIter<'c> {
        self.iter()
    }
}

impl fmt::Debug for SymbolicAtoms<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SymbolicAtoms").finish_non_exhaustive()
    }
}

/// An iterator over symbolic atoms, from [`SymbolicAtoms::iter`] or
/// [`SymbolicAtoms::by_signature`]. It borrows the control.
pub struct SymbolicAtomIter<'c> {
    control: ErrorSink<'c>,
    atoms: Atoms<'c>,
    state: IterState,
}

#[derive(Clone, Copy)]
enum IterState {
    /// Not started: the first call to `next` asks clingo for the positions.
    Start(Option<Signature>),
    Running {
        current: AtomIterator,
        end: AtomIterator,
    },
    Done,
}

impl<'c> SymbolicAtomIter<'c> {
    fn new(control: ErrorSink<'c>, atoms: Atoms<'c>, signature: Option<Signature>) -> Self {
        SymbolicAtomIter {
            control,
            atoms,
            state: IterState::Start(signature),
        }
    }

    /// Reads the atom at the current position and advances, or returns `None`
    /// at the end.
    fn step(&mut self) -> Result<Option<SymbolicAtom>> {
        let atoms = self.atoms;
        let (current, end) = match self.state {
            IterState::Done => return Ok(None),
            IterState::Start(signature) => {
                (atoms.begin(signature.map(Signature::raw))?, atoms.end()?)
            }
            IterState::Running { current, end } => (current, end),
        };
        if atoms.is_equal(current, end)? {
            self.state = IterState::Done;
            return Ok(None);
        }
        let data = atoms.read(current)?;
        self.state = IterState::Running {
            current: atoms.next(current)?,
            end,
        };
        Ok(Some(SymbolicAtom::from_data(data)))
    }
}

impl Iterator for SymbolicAtomIter<'_> {
    type Item = Result<SymbolicAtom>;

    fn next(&mut self) -> Option<Result<SymbolicAtom>> {
        let control = self.control;
        let item = control
            .refusal()
            .and_then(|()| self.step())
            .map_err(|err| control.note(err.context("reading the symbolic atoms")));
        if item.is_err() {
            self.state = IterState::Done;
        }
        item.transpose()
    }
}

impl std::iter::FusedIterator for SymbolicAtomIter<'_> {}

impl fmt::Debug for SymbolicAtomIter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SymbolicAtomIter").finish_non_exhaustive()
    }
}

impl Model {
    /// Whether a program literal is true in the model.
    ///
    /// For the literal of a symbolic atom it agrees with [`Model::contains`] on
    /// the atom's symbol.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs out
    ///   of memory;
    /// - [`ErrorKind::Logic`](crate::ErrorKind::Logic), which poisons the
    ///   control, for a literal within [`ProgramLiteral::MAX_MAGNITUDE`] but
    ///   still in clasp's body-id range (see `ProgramLiteral::
    ///   MAX_MAGNITUDE`'s own doc comment).
    ///
    /// # Examples
    ///
    /// ```
    /// use std::ops::ControlFlow;
    ///
    /// use clingox::{Control, Part, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let a = Symbol::function("a", &[])?;
    /// let literal = ctl.symbolic_atoms()?.find(a)?.expect("a is an atom").literal();
    /// ctl.for_each_model(&[], |model| {
    ///     assert!(model.is_true(literal)?);
    ///     Ok(ControlFlow::Continue(()))
    /// })?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn is_true(&self, literal: ProgramLiteral) -> Result<bool> {
        raw::model_is_true(self, literal.get())
            .map_err(|e| e.context(format!("reading literal {}", literal.get())))
    }
}

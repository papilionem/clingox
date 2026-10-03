//! Authoring ground directives directly, bypassing the grounder.
//!
//! [`Control::with_backend`] opens a [`Backend`] for the duration of a closure
//! and finishes it when the closure returns, whether it succeeds, fails or
//! panics (DESIGN S4, S5). A backend adds already-ground rules, constraints,
//! projection, external, assumption, heuristic and edge directives, and theory
//! terms, elements and atoms, straight to the program: the same primitives the
//! grounder itself builds an ordinary logic program from.
//!
//! This reproduces a typical backend example: a fact, a choice rule and a
//! constraint, authored entirely through the backend instead of program text.
//!
//! **An in-range [`ProgramLiteral`]/[`Atom`] far beyond the program's real
//! atoms still costs memory in proportion to its magnitude**
//! (`docs/dev/UPSTREAM-ISSUES.md` U22): clasp allocates its internal tables up
//! to that value, whatever a backend call is actually about, before reporting
//! anything wrong. A raw value straight from
//! [`ProgramLiteral::from_raw`](crate::ProgramLiteral::from_raw)/[`Atom`], not
//! one clingox itself handed back from an existing grounding, is the risk this
//! applies to.
//!
//! ```
//! use clingox::backend::Head;
//! use clingox::{Control, Part};
//!
//! let mut ctl = Control::new()?;
//! ctl.add_base("").unwrap();
//! let (a, aux) = ctl.with_backend(|backend| {
//!     let a = backend.add_atom(Some("a".parse()?))?;
//!     let aux = backend.add_aux_atom()?;
//!     backend.add_rule(Head::Normal(&[a]), &[])?; // a.
//!     backend.add_rule(Head::Choice(&[aux]), &[a.pos()])?; // { aux } :- a.
//!     backend.add_rule(Head::Constraint, &[aux.neg()])?; // :- not aux.
//!     Ok((a, aux))
//! })?;
//! ctl.ground(&[Part::base()])?;
//! let result = ctl.solve(&[])?;
//! assert!(result.is_sat());
//! # Ok::<(), clingox::Error>(())
//! ```

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::fmt;
use std::path::Path;

use crate::atoms::ProgramLiteral;
use crate::control::ControlCore;
use crate::error::{Error, ErrorKind, Result};
use crate::raw;
use crate::symbol::Symbol;
use crate::theory::Id;

/// An atom of the ground program (`clingo_atom_t`).
///
/// It is a separate id space from [`ProgramLiteral`] (DESIGN S17): a rule's
/// head lists `Atom`s, and its body lists [`ProgramLiteral`]s;
/// [`Atom::pos`] and [`Atom::neg`] convert one atom to the literal a body or
/// an assumption takes, choosing the sign.
///
/// # Examples
///
/// ```
/// use clingox::Control;
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("").unwrap();
/// let (positive, negative) = ctl.with_backend(|backend| {
///     let a = backend.add_atom(Some("a".parse()?))?;
///     Ok((a.pos(), a.neg()))
/// })?;
/// assert_ne!(positive, negative);
/// # Ok::<(), clingox::Error>(())
/// ```
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Atom(u32);

impl Atom {
    pub(crate) fn from_raw(raw: u32) -> Atom {
        Atom(raw)
    }

    pub(crate) fn raw(self) -> u32 {
        self.0
    }

    /// The atom's positive literal: true exactly when the atom is.
    ///
    /// aspif numbers an atom's positive literal with the atom's own id
    /// (`clingo.hh`'s own backend helpers rely on the same fact), so this
    /// never calls into clingo.
    #[must_use]
    pub fn pos(self) -> ProgramLiteral {
        // clasp's variables (and so its atoms and literals) stay below 2^30
        // (`ProgramLiteral::MAX_MAGNITUDE`), and an atom's id is never 0, so
        // this conversion is always a valid program literal (checked in
        // debug builds by `from_valid`).
        ProgramLiteral::from_valid(i32::try_from(self.0).unwrap_or(i32::MAX))
    }

    /// The atom's negative literal: true exactly when the atom is false.
    ///
    /// It is always distinct from [`Atom::pos`]'s result: negating a nonzero
    /// value never produces the same value back.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the name pairs with `pos`, the same head/body \
                  terminology `clingo_backend_rule` uses; `ProgramLiteral` keeps std::ops::Neg \
                  separate from its own `negate` for the same reason"
    )]
    pub fn neg(self) -> ProgramLiteral {
        self.pos().negate()
    }
}

/// The head of a rule, for [`Backend::add_rule`] and
/// [`Backend::add_weight_rule`].
///
/// `clingo_backend_rule` and `_weight_rule` share one `(choice, head,
/// head_size)` triple (`H:1670-1697`): [`Head::Normal`] is a disjunction
/// (`choice = false`, atoms given), [`Head::Choice`] is a choice
/// (`choice = true`), and [`Head::Constraint`] is `choice = false` with an
/// empty head, `:- body.`.
///
/// # Examples
///
/// ```
/// use clingox::backend::Head;
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::with_args(["--models=0"])?;
/// ctl.add_base("").unwrap();
/// ctl.with_backend(|backend| {
///     let a = backend.add_atom(Some("a".parse()?))?;
///     backend.add_rule(Head::Choice(&[a]), &[])
/// })?;
/// ctl.ground(&[Part::base()])?;
/// let (_, models) = ctl.solve_all()?;
/// assert_eq!(models.len(), 2);
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Head<'a> {
    /// A disjunctive head: `a1 | a2 | ... :- body.` (a fact when `body` is
    /// empty and there is exactly one atom).
    Normal(&'a [Atom]),
    /// A choice head: `{ a1; a2; ... } :- body.`.
    Choice(&'a [Atom]),
    /// No head at all: `:- body.`.
    Constraint,
}

/// `Head`'s `(choice, atoms)` pair, as `clingo_backend_rule`/`_weight_rule`
/// take it.
fn head_parts(head: Head<'_>) -> (bool, Vec<u32>) {
    match head {
        Head::Normal(atoms) => (false, atoms.iter().map(|a| a.raw()).collect()),
        Head::Choice(atoms) => (true, atoms.iter().map(|a| a.raw()).collect()),
        Head::Constraint => (false, Vec::new()),
    }
}

/// The kind of a domain heuristic modification (`clingo_heuristic_type_e`),
/// for [`Backend::add_heuristic`].
///
/// # Examples
///
/// ```
/// use clingox::backend::HeuristicKind;
/// use clingox::{Control, Outcome, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.configuration().set("solver.heuristic", "domain")?;
/// ctl.add_base("{a;b}.").unwrap();
/// ctl.with_backend(|backend| {
///     let a = backend.add_atom(Some("a".parse()?))?;
///     let b = backend.add_atom(Some("b".parse()?))?;
///     backend.add_heuristic(a, HeuristicKind::True, 1, 1, &[])?;
///     backend.add_heuristic(b, HeuristicKind::False, 1, 1, &[])
/// })?;
/// ctl.ground(&[Part::base()])?;
/// // `solve_first` reports the search's actual first model; unlike
/// // `solve_all`, it never lifts the configured model limit, so the
/// // heuristic's effect on search order is visible here.
/// let Outcome::Sat(model, _) = ctl.solve_first()? else {
///     panic!("the program has a model");
/// };
/// assert_eq!(model.symbols().len(), 1);
/// # Ok::<(), clingox::Error>(())
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeuristicKind {
    /// Sets the level (the priority among heuristics that apply at once).
    Level,
    /// Modifies the sign the heuristic assigns the atom first.
    Sign,
    /// Scales the heuristic's weight by a factor.
    Factor,
    /// Sets the atom's initial score.
    Init,
    /// Makes the atom the heuristic's first choice, true.
    True,
    /// Makes the atom the heuristic's first choice, false.
    False,
}

/// The kind of an external statement (`clingo_external_type_e`), for
/// [`Backend::add_external`].
///
/// It mirrors the four states `clingo_backend_external` accepts, and is kept
/// separate from [`TruthValue`](crate::TruthValue): that type is
/// [`Control::assign_external`](crate::Control::assign_external)'s own
/// three-state runtime call, by symbol, and this one is the backend's raw
/// four-state directive, by atom, which also includes `Release`.
///
/// # Examples
///
/// ```
/// use clingox::backend::ExternalKind;
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::with_args(["--models=0"])?;
/// ctl.add_base("b :- e.").unwrap();
/// ctl.with_backend(|backend| {
///     let e = backend.add_atom(Some("e".parse()?))?;
///     backend.add_external(e, ExternalKind::True)
/// })?;
/// ctl.ground(&[Part::base()])?;
/// let (_, models) = ctl.solve_all()?;
/// assert_eq!(models.len(), 1);
/// # Ok::<(), clingox::Error>(())
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExternalKind {
    /// The atom's value is left open: both truth values are considered.
    Free,
    /// The atom is forced true.
    True,
    /// The atom is forced false.
    False,
    /// The atom becomes permanently false and is no longer external, as
    /// [`Control::release_external`](crate::Control::release_external) does.
    Release,
}

/// The kind of a theory sequence term (`clingo_theory_sequence_type_e`), for
/// [`Backend::add_theory_sequence`]. Shared with the ground program observer.
///
/// `#[non_exhaustive]`: clingo's own enum could grow a fourth sequence kind in
/// a future release, and a caller who matched every variant today would
/// otherwise have that match stop compiling the moment clingox added one,
/// rather than being asked to add a wildcard arm now.
///
/// # Examples
///
/// ```
/// use clingox::backend::TheorySequenceKind;
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("#theory t { term { }; &a/0 : term, any }. &a { (1,2) }.").unwrap();
/// ctl.ground(&[Part::base()])?;
/// let seq = ctl.with_backend(|backend| {
///     let one = backend.add_theory_number(1)?;
///     let two = backend.add_theory_number(2)?;
///     backend.add_theory_sequence(TheorySequenceKind::Tuple, &[one, two])
/// })?;
/// assert_eq!(ctl.theory_atoms()?.term(seq)?.to_string(), "(1,2)");
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TheorySequenceKind {
    /// A tuple term, `(t1,...,tn)`.
    Tuple,
    /// A set term, `{t1,...,tn}`.
    Set,
    /// A list term, `[t1,...,tn]`.
    List,
}

/// Which atom a theory atom gets (`clingo_backend_theory_atom`'s `atom`
/// parameter), for [`Backend::add_theory_atom`] and
/// [`Backend::add_theory_atom_with_guard`].
///
/// The header encodes three meanings in one integer ("if atom is set to
/// zero, the theory atom is a directive, if atom is set to `UINT32_MAX`, the
/// theory atom receives a fresh atom, and otherwise the theory atom receives
/// the given atom id"); this enum makes the choice explicit instead of a
/// sentinel a caller could pass wrong.
///
/// `#[non_exhaustive]`, for the same reason as [`TheorySequenceKind`].
///
/// # Examples
///
/// ```
/// use clingox::backend::TheoryAtomTarget;
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("#theory t { term { }; &a/0 : term, any }.").unwrap();
/// ctl.ground(&[Part::base()])?;
/// let fresh = ctl.with_backend(|backend| {
///     let term = backend.add_theory_symbol("c".parse()?)?;
///     backend.add_theory_atom(TheoryAtomTarget::Fresh, term, &[])
/// })?;
/// let atoms = ctl.theory_atoms()?;
/// let atom = atoms.iter().next().unwrap()?;
/// assert_eq!(atom.literal()?, Some(fresh.pos()));
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TheoryAtomTarget {
    /// A directive: the theory atom gets no literal at all and is never part
    /// of a rule (the header's `0`).
    Directive,
    /// A fresh atom, with its own new program literal (the header's
    /// `UINT32_MAX`).
    Fresh,
    /// An existing atom, which keeps its own id.
    Atom(Atom),
}

/// `TheoryAtomTarget`'s raw atom id, as `clingo_backend_theory_atom` takes it.
fn raw_atom_target(target: TheoryAtomTarget) -> u32 {
    match target {
        TheoryAtomTarget::Directive => 0,
        TheoryAtomTarget::Fresh => u32::MAX,
        TheoryAtomTarget::Atom(atom) => atom.raw(),
    }
}

/// Which of clingo's internal backends [`Control::register_backend_writer`]
/// selects (`clingo_backend_type_e`, clingo.h:2958-2968), a hand-written
/// bitset (S17, the [`ShowType`](crate::ShowType) precedent): the header
/// documents it as "a mix between enum and bit set: bits 0 and 1 are used to
/// configure the reify backend", so [`BackendWriterKind::reify_sccs`] and
/// [`BackendWriterKind::reify_steps`] set those two bits and only mean
/// anything combined with [`BackendWriterKind::REIFY`].
///
/// # Examples
///
/// ```
/// use clingox::backend::BackendWriterKind;
///
/// let reify = BackendWriterKind::REIFY.reify_sccs().reify_steps();
/// assert_ne!(reify, BackendWriterKind::REIFY);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BackendWriterKind(u32);

impl BackendWriterKind {
    /// The reify backend: dumps the ground program as reified facts
    /// (`atom_tuple/2`, `rule/2`, ...).
    pub const REIFY: BackendWriterKind = BackendWriterKind(0);
    /// The aspif backend: dumps the ground program in aspif text format,
    /// readable back with [`Control::load_aspif`].
    pub const ASPIF: BackendWriterKind = BackendWriterKind(4);
    /// The smodels backend. In the clingo 5.8.2 build this crate tests
    /// against, its output is byte-for-byte identical to
    /// [`BackendWriterKind::ASPIF`]'s, not the legacy numeric smodels format
    /// (checked directly); a future
    /// clingo release may differ.
    pub const SMODELS: BackendWriterKind = BackendWriterKind(5);

    /// With [`BackendWriterKind::REIFY`], also reifies strongly connected
    /// components. Meaningless combined with any other kind.
    ///
    /// An unpatched clingo 5.8 also turns on step numbers for this bit, as
    /// [`BackendWriterKind::reify_steps`] should; the vendored build is
    /// patched (U53).
    #[must_use]
    pub fn reify_sccs(self) -> BackendWriterKind {
        BackendWriterKind(self.0 | 1)
    }

    /// With [`BackendWriterKind::REIFY`], also reifies each incremental step
    /// individually. Meaningless combined with any other kind.
    ///
    /// An unpatched clingo 5.8 ignores this bit on its own and passes the SCC
    /// bit in its place; the vendored build is patched (U53).
    #[must_use]
    pub fn reify_steps(self) -> BackendWriterKind {
        BackendWriterKind(self.0 | 2)
    }

    pub(crate) fn bits(self) -> u32 {
        self.0
    }
}

impl ScopedControl<'_> {
    /// Tells clingo to write the ground program to `file`, in the format `kind`
    /// selects (`clingo_control_register_backend`, clingo.h:3336-3349).
    ///
    /// This is a different mechanism from [`Control::with_backend`]: it does
    /// not hand the caller a [`Backend`] to write to, it tells clingo to dump
    /// its own ground output, and it shares its registration engine with
    /// [`Control::register_observer`] (the header itself draws the comparison),
    /// so `replace` has the same meaning: `false` writes the file and still
    /// sends the program to the solver; `true` writes the file and sends
    /// nothing to the solver (like [`Control::register_observer`]'s `replace:
    /// true`, [`Control::solve`] afterward reports neither satisfiable nor
    /// unsatisfiable).
    ///
    /// Like an observer, a backend writer cannot be unregistered once
    /// registered, and stays active for every later grounding on this control.
    ///
    /// **A failed write is not reported.** clingo never checks the file after
    /// opening it, so on a full disk grounding and solving succeed and the
    /// file is short or empty (U52). Check the file's size or content
    /// afterwards when it matters.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidInput`] if `file` is not valid UTF-8;
    /// - [`ErrorKind::Runtime`] if the file cannot be opened. This does not
    ///   poison the control, the same shape as [`Control::load`]'s missing-file
    ///   case;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    ///   the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::backend::BackendWriterKind;
    /// use clingox::{Control, Part};
    ///
    /// let path = std::env::temp_dir()
    ///     .join(format!("clingox_doctest_register_backend_writer_{}.aspif", std::process::id()));
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.register_backend_writer(BackendWriterKind::ASPIF, &path, false)?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// assert!(ctl.solve(&[])?.is_sat());
    /// assert!(std::fs::read_to_string(&path).unwrap().starts_with("asp 1 0 0"));
    /// std::fs::remove_file(&path).ok();
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn register_backend_writer(
        &mut self,
        kind: BackendWriterKind,
        file: impl AsRef<Path>,
        replace: bool,
    ) -> Result<()> {
        let file = file.as_ref();
        self.core.guarded(
            || format!("registering a backend writer to `{}`", file.display()),
            |handle| handle.register_backend_writer(kind, file, replace),
        )
    }
}

impl ScopedControl<'_> {
    /// Runs `f` with a [`Backend`] to add ground directives directly, bypassing
    /// the grounder.
    ///
    /// `clingo_backend_begin` and `clingo_backend_end` (`H:1656-1669`) run
    /// around `f`: `f`'s `Backend<'_>` argument borrows the control mutably
    /// (DESIGN S5), so grounding, solving or opening a second backend cannot
    /// happen while it is alive, and `Backend<'_>` itself cannot leave `f`,
    /// which the borrow checker enforces statically
    /// (`clingox/tests/ui/backend_used_while_control_grounds.rs`,
    /// `clingox/tests/ui/backend_escapes_the_closure.rs`). A value `f` computes
    /// and returns, such as an [`Atom`], is a plain owned `Copy` value and can
    /// be used freely afterward
    /// (`clingox/tests/ui_pass/backend_atom_outlives_the_closure.rs`).
    ///
    /// **The backend still closes if `f` returns `Err`.** A panic is different: it is not
    /// caught here (there is no C callback between `f` and clingo, unlike a
    /// ground callback or the observer), so it unwinds through this call as
    /// ordinary Rust code instead, leaving the backend recorded as open rather
    /// than closing it on the spot. The next entry point on this control,
    /// whichever it is, finishes it before doing anything else, the same "a
    /// leftover search is closed first" recovery every other method already
    /// gives a forgotten [`SolveHandle`](crate::SolveHandle) (DESIGN S4). Under
    /// `panic = "abort"` the process ends at the panic, and no clingo call is
    /// on the stack at that point, so nothing is left half done.
    ///
    /// # Errors
    ///
    /// - Whatever `f` returns, unchanged, if `f` returns `Err`, except that
    ///   it still poisons the control by its own kind, exactly as any other
    ///   returned error does (S3): an [`ErrorKind::Logic`] or
    ///   [`ErrorKind::Unknown`] from `f` is not exempt just because it passes
    ///   through unchanged;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory opening or
    ///   closing the backend, which poisons the control;
    /// - a failed `clingo_backend_end` poisons the control whatever its kind,
    ///   keeping `f`'s own error, if any, as the one reported, with the close
    ///   failure as its cause (mirrors a failed solve-handle close, S3, S7);
    /// - a registered
    ///   [`GroundProgramObserver`](crate::observer::GroundProgramObserver)'s
    ///   own error, unchanged, whether it fired synchronously inside `f` (a
    ///   plain directive) or at the backend's own close (a fact's delayed
    ///   notification): this always poisons the control, whatever its kind,
    ///   taking priority over `f`'s own result and over a close failure (see
    ///   the poisoning note on [`Control`]). A panic in the observer is
    ///   resumed here the same way, after poisoning;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::backend::Head;
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("").unwrap();
    /// ctl.with_backend(|backend| {
    ///     let a = backend.add_atom(Some("a".parse()?))?;
    ///     backend.add_rule(Head::Normal(&[a]), &[])
    /// })?;
    /// ctl.ground(&[Part::base()])?;
    /// assert!(ctl.solve(&[])?.is_sat());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    ///
    /// The backend still finishes when the closure fails:
    ///
    /// ```
    /// use clingox::{Control, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.").unwrap();
    /// let err = ctl.with_backend(|backend| {
    ///     let _ = Symbol::function("bad\0name", &[])?;
    ///     backend.add_aux_atom()
    /// });
    /// assert!(err.is_err());
    /// // A later call succeeds normally.
    /// ctl.with_backend(|backend| backend.add_aux_atom())?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn with_backend<R>(&mut self, f: impl FnOnce(&mut Backend<'_>) -> Result<R>) -> Result<R> {
        self.core.refusal()?;
        self.core.finish_search()?;
        self.core
            .handle
            .open_backend()
            .map_err(|err| self.core.note(err.context("opening the backend")))?;
        let result = {
            let mut backend = Backend::new(&mut self.core);
            f(&mut backend)
        };
        let closed = self
            .core
            .handle
            .close_backend()
            .map_err(|err| err.context("closing the backend"));
        self.core.handle.resume_logger_panic();
        // A registered observer's own panic
        // or error, if either fired (synchronously inside `f` for a plain
        // directive, or at `clingo_backend_end` just above for a fact's
        // delayed `output_atom`), takes priority over both `result` and
        // `closed` and poisons unconditionally, exactly as it already does
        // for `Control::ground`/`Control::ground_with`.
        // The panic carries the observer's own payload untouched, matching
        // `ground`'s panic path.
        if let Some(payload) = self.core.handle.take_observer_panic() {
            self.core.poison_after_panic("using the backend");
            std::panic::resume_unwind(payload);
        }
        if let Some(err) = self.core.handle.take_observer_error() {
            return Err(self.core.note(err.context("using the backend").poisoning()));
        }
        raw::settle(result, closed).map_err(|err| self.core.note(err))
    }
}

/// A handle to add ground directives to the program, from
/// [`Control::with_backend`].
///
/// It borrows the control mutably (DESIGN S5) for exactly the duration of
/// the closure `with_backend` runs it in.
pub struct Backend<'c> {
    control: &'c mut ControlCore,
}

impl<'c> Backend<'c> {
    fn new(control: &'c mut ControlCore) -> Backend<'c> {
        Backend { control }
    }

    /// Runs one call on the open backend, naming `context` in any error.
    fn call<T>(
        &self,
        context: &str,
        f: impl FnOnce(&raw::ControlHandle) -> Result<T, Error>,
    ) -> Result<T> {
        let control = &*self.control;
        control
            .refusal()
            .and_then(|()| f(&control.handle))
            .map_err(|err| control.note(err.context(context.to_owned())))
    }

    /// Gets a fresh atom, optionally associated with `symbol`.
    ///
    /// `add_atom` interns by symbol: calling it again with a symbol that is,
    /// or will be, grounded from program text returns the same atom, not a
    /// duplicate.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    /// the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::backend::Head;
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("").unwrap();
    /// ctl.with_backend(|backend| {
    ///     let p1 = backend.add_atom(Some("p(1)".parse()?))?;
    ///     backend.add_rule(Head::Normal(&[p1]), &[])
    /// })?;
    /// ctl.ground(&[Part::base()])?;
    /// assert!(ctl.symbolic_atoms()?.find("p(1)".parse()?)?.is_some());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_atom(&mut self, symbol: Option<Symbol>) -> Result<Atom> {
        self.call("adding an atom", |handle| {
            handle
                .backend_add_atom(symbol.map(Symbol::raw))
                .map(Atom::from_raw)
        })
    }

    /// [`Backend::add_atom`] with no symbol: `add_atom(None)`.
    ///
    /// The atom never shows up in [`SymbolicAtoms`](crate::SymbolicAtoms), and
    /// can only be observed by literal, through
    /// [`Model::is_true`](crate::Model::is_true).
    ///
    /// # Errors
    ///
    /// As [`Backend::add_atom`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::backend::Head;
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("").unwrap();
    /// let aux = ctl.with_backend(|backend| {
    ///     let aux = backend.add_aux_atom()?;
    ///     backend.add_rule(Head::Normal(&[aux]), &[])?;
    ///     Ok(aux)
    /// })?;
    /// ctl.ground(&[Part::base()])?;
    /// assert!(ctl.solve(&[aux.pos().into()])?.is_sat());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_aux_atom(&mut self) -> Result<Atom> {
        self.add_atom(None)
    }

    /// Adds a rule.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    /// the control.
    ///
    /// # Examples
    ///
    /// See the module documentation.
    pub fn add_rule(&mut self, head: Head<'_>, body: &[ProgramLiteral]) -> Result<()> {
        let (choice, head_atoms) = head_parts(head);
        let raw_body: Vec<i32> = body.iter().copied().map(ProgramLiteral::get).collect();
        self.call("adding a rule", |handle| {
            handle.backend_rule(choice, &head_atoms, &raw_body)
        })
    }

    /// Adds a weight rule: the head is derived once the sum of the weights of
    /// the true body literals meets `lower_bound`.
    ///
    /// The header requires the bound and every weight to be positive
    /// (`H:1678`). clingox checks both before calling clingo:
    /// - clingo accepts a weight of `0` and a non-positive bound silently, and
    ///   the rule then cannot do what its shape suggests: a zero-weight literal
    ///   never adds to the sum, and a non-positive bound is always met, so the
    ///   head is derived unconditionally;
    /// - clingo rejects a negative weight only as a logic error, which would
    ///   poison the control, although the rule never took effect.
    ///
    /// The bound is checked first, then the weights in body order; the first
    /// violation is reported.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidInput`] if `lower_bound` or any weight is zero or
    ///   negative. It does not poison the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    ///   the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::backend::Head;
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.add_base("").unwrap();
    /// ctl.with_backend(|backend| {
    ///     let x = backend.add_atom(Some("x".parse()?))?;
    ///     let y = backend.add_atom(Some("y".parse()?))?;
    ///     let head = backend.add_atom(Some("head".parse()?))?;
    ///     backend.add_rule(Head::Choice(&[x]), &[])?;
    ///     backend.add_rule(Head::Choice(&[y]), &[])?;
    ///     backend.add_weight_rule(Head::Normal(&[head]), 3, &[(x.pos(), 2), (y.pos(), 3)])
    /// })?;
    /// ctl.ground(&[Part::base()])?;
    /// let (_, models) = ctl.solve_all()?;
    /// assert_eq!(models.len(), 4);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_weight_rule(
        &mut self,
        head: Head<'_>,
        lower_bound: i32,
        body: &[(ProgramLiteral, i32)],
    ) -> Result<()> {
        if lower_bound <= 0 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "a weight rule's lower bound must be positive, got {lower_bound} \
                     (clingo accepts it silently but the head would then be derived \
                     unconditionally)"
                ),
            ));
        }
        if let Some(&(_, weight)) = body.iter().find(|&&(_, weight)| weight <= 0) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("a weight rule's weights must be positive, got {weight}"),
            ));
        }
        let (choice, head_atoms) = head_parts(head);
        let raw_body: Vec<raw::WeightedLiteral> = body
            .iter()
            .map(|&(lit, weight)| raw::weighted_literal(lit.get(), weight))
            .collect();
        self.call("adding a weight rule", |handle| {
            handle.backend_weight_rule(choice, &head_atoms, lower_bound, &raw_body)
        })
    }

    /// Adds a minimize (or weak) constraint at `priority`: `:~ ...
    /// [w1@priority, w2@priority, ...]`.
    ///
    /// Priorities are compared lexicographically, highest first: a search
    /// minimizes every literal at the highest priority before considering a
    /// lower one at all, never a priority-blind sum of every weight.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons the
    /// control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::backend::Head;
    /// use clingox::{Control, Outcome};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("").unwrap();
    /// ctl.with_backend(|backend| {
    ///     let p = backend.add_atom(Some("p".parse()?))?;
    ///     let q = backend.add_atom(Some("q".parse()?))?;
    ///     backend.add_rule(Head::Choice(&[p]), &[])?;
    ///     backend.add_rule(Head::Choice(&[q]), &[])?;
    ///     backend.add_rule(Head::Constraint, &[p.neg(), q.neg()])?;
    ///     backend.add_rule(Head::Constraint, &[p.pos(), q.pos()])?;
    ///     backend.add_minimize(1, &[(p.pos(), 1)])?;
    ///     backend.add_minimize(0, &[(q.pos(), 100)])
    /// })?;
    /// ctl.ground(&[clingox::Part::base()])?;
    /// let Outcome::Sat(model, _) = ctl.solve_optimal()? else {
    ///     panic!("the program is satisfiable");
    /// };
    /// assert_eq!(model.cost(), [0, 100]);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_minimize(
        &mut self,
        priority: i32,
        literals: &[(ProgramLiteral, i32)],
    ) -> Result<()> {
        let raw_literals: Vec<raw::WeightedLiteral> = literals
            .iter()
            .map(|&(lit, weight)| raw::weighted_literal(lit.get(), weight))
            .collect();
        self.call("adding a minimize constraint", |handle| {
            handle.backend_minimize(priority, &raw_literals)
        })
    }

    /// Adds a projection directive: the given atoms become (part of) the set
    /// solving projects onto.
    ///
    /// This only changes anything once projection is enabled for solving
    /// (clingo's `--project`, or `configuration.solve.project`); calling it
    /// alone does not turn projection on.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    /// the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::backend::Head;
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.configuration().set("solve.project", "auto")?;
    /// ctl.add_base("").unwrap();
    /// ctl.with_backend(|backend| {
    ///     let a = backend.add_atom(Some("a".parse()?))?;
    ///     let b = backend.add_atom(Some("b".parse()?))?;
    ///     backend.add_rule(Head::Choice(&[a]), &[])?;
    ///     backend.add_rule(Head::Choice(&[b]), &[])?;
    ///     backend.add_project([a])
    /// })?;
    /// ctl.ground(&[Part::base()])?;
    /// let (_, models) = ctl.solve_all()?;
    /// assert_eq!(models.len(), 2);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_project<I: IntoIterator<Item = Atom>>(&mut self, atoms: I) -> Result<()> {
        let raw_atoms: Vec<u32> = atoms.into_iter().map(Atom::raw).collect();
        self.call("adding a projection directive", |handle| {
            handle.backend_project(&raw_atoms)
        })
    }

    /// Adds an external statement, as `#external` would in program text.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    /// the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::backend::ExternalKind;
    /// use clingox::Control;
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.add_base("b :- e.").unwrap();
    /// ctl.with_backend(|backend| {
    ///     let e = backend.add_atom(Some("e".parse()?))?;
    ///     backend.add_external(e, ExternalKind::Free)
    /// })?;
    /// ctl.ground(&[clingox::Part::base()])?;
    /// let (_, models) = ctl.solve_all()?;
    /// assert_eq!(models.len(), 2);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_external(&mut self, atom: Atom, kind: ExternalKind) -> Result<()> {
        self.call("adding an external", |handle| {
            handle.backend_external(atom.raw(), kind)
        })
    }

    /// Adds an assumption directive: the literals are assumed true or false
    /// (by their own sign) for the next solve call only, exactly like an
    /// argument to [`Control::solve`](crate::Control::solve) but authored
    /// from the backend.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    /// the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.add_base("{a;b}.").unwrap();
    /// ctl.ground(&[Part::base()])?;
    /// ctl.with_backend(|backend| {
    ///     let a = backend.add_atom(Some("a".parse()?))?;
    ///     let b = backend.add_atom(Some("b".parse()?))?;
    ///     backend.add_assumptions([a.neg(), b.pos()])
    /// })?;
    /// let (_, first) = ctl.solve_all()?;
    /// assert_eq!(first.len(), 1);
    /// let (_, second) = ctl.solve_all()?;
    /// assert_eq!(second.len(), 4, "the assumption does not persist");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_assumptions<I: IntoIterator<Item = ProgramLiteral>>(
        &mut self,
        literals: I,
    ) -> Result<()> {
        let raw_literals: Vec<i32> = literals.into_iter().map(ProgramLiteral::get).collect();
        self.call("adding an assumption directive", |handle| {
            handle.backend_assume(&raw_literals)
        })
    }

    /// Adds a domain heuristic directive, steering which value the solver's
    /// heuristic tries for `atom` first, under `condition`.
    ///
    /// This only changes anything with a domain heuristic enabled for
    /// solving (`configuration.solver.heuristic = "domain"`).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    /// the control.
    ///
    /// # Examples
    ///
    /// See [`HeuristicKind`].
    pub fn add_heuristic(
        &mut self,
        atom: Atom,
        kind: HeuristicKind,
        bias: i32,
        priority: u32,
        condition: &[ProgramLiteral],
    ) -> Result<()> {
        let raw_condition: Vec<i32> = condition.iter().copied().map(ProgramLiteral::get).collect();
        self.call("adding a heuristic directive", |handle| {
            handle.backend_heuristic(atom.raw(), kind, bias, priority, &raw_condition)
        })
    }

    /// Adds an edge of the acyclicity graph, from `u` to `v`, active under
    /// `condition`: a set of edges that would complete a cycle is forbidden.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    /// the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.add_base("{a;b}.").unwrap();
    /// ctl.with_backend(|backend| {
    ///     let a = backend.add_atom(Some("a".parse()?))?;
    ///     let b = backend.add_atom(Some("b".parse()?))?;
    ///     backend.add_edge(1, 2, &[a.pos()])?;
    ///     backend.add_edge(2, 1, &[b.pos()])
    /// })?;
    /// ctl.ground(&[Part::base()])?;
    /// let (_, models) = ctl.solve_all()?;
    /// assert_eq!(models.len(), 3, "{{a,b}} forms a cycle and is forbidden");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_edge(&mut self, u: i32, v: i32, condition: &[ProgramLiteral]) -> Result<()> {
        let raw_condition: Vec<i32> = condition.iter().copied().map(ProgramLiteral::get).collect();
        self.call("adding an edge directive", |handle| {
            handle.backend_acyc_edge(u, v, &raw_condition)
        })
    }

    /// Adds a numeric theory term.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    /// the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, TheoryTermKind};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#theory t { term { }; &a/0 : term, any }.").unwrap();
    /// ctl.ground(&[Part::base()])?;
    /// let id = ctl.with_backend(|backend| backend.add_theory_number(42))?;
    /// assert_eq!(ctl.theory_atoms()?.term_kind(id)?, TheoryTermKind::Number);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_theory_number(&mut self, number: i32) -> Result<Id> {
        self.call("adding a theory number term", |handle| {
            handle.backend_theory_number(number).map(Id::from_raw)
        })
    }

    /// Adds a string theory term. clingo represents it internally the same way
    /// as a symbolic term (checked directly against clingo 5.8.2): its kind
    /// reads back as [`TheoryTermKind::Symbol`](crate::TheoryTermKind::Symbol),
    /// not a kind of its own.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Nul`] if `s` contains a NUL byte;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    ///   the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, TheoryTermKind};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#theory t { term { }; &a/0 : term, any }.").unwrap();
    /// ctl.ground(&[Part::base()])?;
    /// let id = ctl.with_backend(|backend| backend.add_theory_string("hi"))?;
    /// assert_eq!(ctl.theory_atoms()?.term_kind(id)?, TheoryTermKind::Symbol);
    /// assert_eq!(ctl.theory_atoms()?.term(id)?.to_string(), "hi");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_theory_string(&mut self, s: &str) -> Result<Id> {
        let c_s = raw::c_str(s)?;
        self.call("adding a theory string term", |handle| {
            handle.backend_theory_string(&c_s).map(Id::from_raw)
        })
    }

    /// Adds a sequence theory term (a tuple, set or list).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    /// the control.
    ///
    /// # Examples
    ///
    /// See [`TheorySequenceKind`].
    pub fn add_theory_sequence(&mut self, kind: TheorySequenceKind, terms: &[Id]) -> Result<Id> {
        let raw_terms: Vec<u32> = terms.iter().copied().map(Id::raw).collect();
        self.call("adding a theory sequence term", |handle| {
            handle
                .backend_theory_sequence(kind, &raw_terms)
                .map(Id::from_raw)
        })
    }

    /// Adds a function theory term.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Nul`] if `name` contains a NUL byte;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    ///   the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#theory t { term { }; &a/0 : term, any }.").unwrap();
    /// ctl.ground(&[Part::base()])?;
    /// let id = ctl.with_backend(|backend| {
    ///     let n = backend.add_theory_number(42)?;
    ///     backend.add_theory_function("f", &[n])
    /// })?;
    /// assert_eq!(ctl.theory_atoms()?.term(id)?.to_string(), "f(42)");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_theory_function(&mut self, name: &str, arguments: &[Id]) -> Result<Id> {
        let c_name = raw::c_str(name)?;
        let raw_args: Vec<u32> = arguments.iter().copied().map(Id::raw).collect();
        self.call("adding a theory function term", |handle| {
            handle
                .backend_theory_function(&c_name, &raw_args)
                .map(Id::from_raw)
        })
    }

    /// Converts a symbol into a theory term.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons the
    /// control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, TheoryTerm};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#theory t { term { }; &a/0 : term, any }.").unwrap();
    /// ctl.ground(&[Part::base()])?;
    /// let id = ctl.with_backend(|backend| backend.add_theory_symbol("c".parse()?))?;
    /// assert_eq!(ctl.theory_atoms()?.term(id)?, TheoryTerm::Symbol("c".parse()?));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_theory_symbol(&mut self, symbol: Symbol) -> Result<Id> {
        self.call("adding a theory symbol term", |handle| {
            handle.backend_theory_symbol(symbol.raw()).map(Id::from_raw)
        })
    }

    /// Adds a theory atom element: a tuple of terms and a condition.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons the
    /// control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::backend::TheoryAtomTarget;
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#theory t { term { }; &a/0 : term, any }.").unwrap();
    /// ctl.ground(&[Part::base()])?;
    /// ctl.with_backend(|backend| {
    ///     let n = backend.add_theory_number(1)?;
    ///     let term = backend.add_theory_symbol("a".parse()?)?;
    ///     let element = backend.add_theory_element(&[n], &[])?;
    ///     backend.add_theory_atom(TheoryAtomTarget::Directive, term, &[element])
    /// })?;
    /// let atoms = ctl.theory_atoms()?;
    /// let atom = atoms.iter().next().unwrap()?;
    /// assert_eq!(atom.elements()?.len(), 1);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_theory_element(&mut self, tuple: &[Id], condition: &[ProgramLiteral]) -> Result<Id> {
        let raw_tuple: Vec<u32> = tuple.iter().copied().map(Id::raw).collect();
        let raw_condition: Vec<i32> = condition.iter().copied().map(ProgramLiteral::get).collect();
        self.call("adding a theory element", |handle| {
            handle
                .backend_theory_element(&raw_tuple, &raw_condition)
                .map(Id::from_raw)
        })
    }

    /// Adds a theory atom without a guard.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    /// the control.
    ///
    /// # Examples
    ///
    /// See [`TheoryAtomTarget`].
    pub fn add_theory_atom(
        &mut self,
        atom: TheoryAtomTarget,
        term: Id,
        elements: &[Id],
    ) -> Result<Atom> {
        let raw_atom = raw_atom_target(atom);
        let raw_elements: Vec<u32> = elements.iter().copied().map(Id::raw).collect();
        self.call("adding a theory atom", |handle| {
            handle
                .backend_theory_atom(raw_atom, term.raw(), &raw_elements)
                .map(Atom::from_raw)
        })
    }

    /// Adds a theory atom with a guard: a connective and a right-hand-side
    /// term, as `&a { ... } <connective> <right_hand_side>` would read.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Nul`] if `operator` contains a NUL byte;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    ///   the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::backend::TheoryAtomTarget;
    /// use clingox::{Control, Part, TheoryTerm};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#theory t { term { }; &a/0 : term, {>=}, term, directive }.").unwrap();
    /// ctl.ground(&[Part::base()])?;
    /// ctl.with_backend(|backend| {
    ///     let n = backend.add_theory_number(42)?;
    ///     let term = backend.add_theory_function("g", &[n])?;
    ///     let guard = backend.add_theory_number(7)?;
    ///     backend.add_theory_atom_with_guard(TheoryAtomTarget::Fresh, term, &[], ">=", guard)
    /// })?;
    /// let atoms = ctl.theory_atoms()?;
    /// let atom = atoms.iter().next().unwrap()?;
    /// let (connective, term) = atom.guard()?.expect("the atom has a guard");
    /// assert_eq!((connective, term), (">=", TheoryTerm::Number(7)));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_theory_atom_with_guard(
        &mut self,
        atom: TheoryAtomTarget,
        term: Id,
        elements: &[Id],
        operator: &str,
        right_hand_side: Id,
    ) -> Result<Atom> {
        let raw_atom = raw_atom_target(atom);
        let raw_elements: Vec<u32> = elements.iter().copied().map(Id::raw).collect();
        let c_operator = raw::c_str(operator)?;
        self.call("adding a theory atom with a guard", |handle| {
            handle
                .backend_theory_atom_with_guard(
                    raw_atom,
                    term.raw(),
                    &raw_elements,
                    &c_operator,
                    right_hand_side.raw(),
                )
                .map(Atom::from_raw)
        })
    }
}

impl fmt::Debug for Backend<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Backend").finish_non_exhaustive()
    }
}

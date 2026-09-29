//! Models: the lent [`Model`] and the owned [`OwnedModel`] (DESIGN S6).

use std::fmt;
use std::ops::Deref;

use crate::atoms::{ProgramLiteral, SymbolicAtoms};
use crate::control::ErrorSink;
use crate::convert::Predicate;
use crate::error::{Error, ErrorKind, Result};
use crate::raw::{self, ModelData};
use crate::symbol::{Sign, Symbol};

/// A model of the program, lent by the search while it is current.
///
/// A `Model` exists only as `&Model`: [`SolveHandle::next_model`] and the
/// closure of [`Control::for_each_model`] lend it, and it cannot be used once
/// the search moves on (DESIGN S6). To keep a model, take a
/// [`snapshot`](Model::snapshot).
///
/// [`Display`](fmt::Display) prints it on one line in clingox's own format:
/// `Answer`, the model's number, and the shown symbols sorted in [`Symbol`]'s
/// order, as in `Answer 1: 42 p(1) t(1)`. It resembles the output of the
/// `clingo` program, which prints the number and the symbols on two lines and
/// in clingo's order, but it is not that output.
///
/// [`SolveHandle::next_model`]: crate::SolveHandle::next_model
/// [`Control::for_each_model`]: crate::Control::for_each_model
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part, ShowType, Symbol};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("p(1). hidden. #show p/1.")?;
/// ctl.ground(&[Part::base()])?;
/// let mut handle = ctl.solve_yield(&[])?;
/// let model = handle.next_model()?.expect("the program has a model");
/// assert_eq!(model.to_string(), "Answer 1: p(1)");
/// assert_eq!(model.symbols(ShowType::ATOMS)?.len(), 2);
/// assert!(model.contains(Symbol::function("hidden", &[])?)?);
/// # Ok::<(), clingox::Error>(())
/// ```
#[repr(transparent)]
pub struct Model(ModelData);

/// The kind of a [`Model`]: an ordinary stable model, or a running union or
/// intersection while clingo enumerates brave or cautious consequences (see
/// [`Model::kind`]).
///
/// # Examples
///
/// ```
/// use clingox::{Control, ModelKind, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("a.")?;
/// ctl.ground(&[Part::base()])?;
/// let mut handle = ctl.solve_yield(&[])?;
/// let model = handle.next_model()?.expect("the program has a model");
/// assert_eq!(model.kind()?, ModelKind::StableModel);
/// # Ok::<(), clingox::Error>(())
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelKind {
    /// An ordinary answer set.
    StableModel,
    /// A running union of every answer set enumerated so far, while clingo
    /// enumerates brave consequences.
    BraveConsequences,
    /// A running intersection of every answer set enumerated so far, while
    /// clingo enumerates cautious consequences.
    CautiousConsequences,
}

/// Whether a literal is a consequence of the program, as
/// [`Model::is_consequence`] reports it.
///
/// Deliberately not `#[non_exhaustive]`, unlike clingox's other public enums
/// (RULES §4): the three values are `clingo_consequence_e`'s complete set
/// (`H:2304-2310`), a `match` on it should not need a wildcard arm to guard
/// against a fourth one, and the acceptance tests
/// (`clingox/tests/api_model_extras.rs`) rely on matching it exhaustively.
///
/// # Examples
///
/// ```
/// use clingox::{Consequence, Control, Part, Symbol};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("a.")?;
/// ctl.ground(&[Part::base()])?;
/// let a = Symbol::function("a", &[])?;
/// let literal = ctl.symbolic_atoms()?.find(a)?.expect("a is an atom").literal();
/// let mut handle = ctl.solve_yield(&[])?;
/// let model = handle.next_model()?.expect("the program has a model");
/// assert_eq!(model.is_consequence(literal)?, Consequence::True);
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Consequence {
    /// The literal is not a consequence; outside brave or cautious
    /// enumeration, it is false in the model.
    False,
    /// The literal is a consequence; outside brave or cautious enumeration,
    /// it is true in the model.
    True,
    /// Not yet decided. Only possible while clingo enumerates brave or
    /// cautious consequences, before enough models have been seen to settle
    /// it (see [`Model::is_consequence`]).
    Unknown,
}

impl Model {
    /// The running number of the model, starting at 1 in each solve call.
    ///
    /// Under `--opt-mode=optN`, clingo numbers the models of its two phases
    /// separately: the models found while searching for the optimum count
    /// from 1, and the optimal models it then enumerates count from 1 again.
    pub fn number(&self) -> u64 {
        raw::model_number(self)
    }

    /// The cost of the model, one entry per priority level, highest priority
    /// first. It is empty for a program without optimisation statements.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs out
    /// of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a. b. :~ a. [2@2] :~ b. [5@1]")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let model = handle.next_model()?.expect("the program has a model");
    /// assert_eq!(model.cost()?, [2, 5]);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn cost(&self) -> Result<Vec<i64>> {
        raw::model_cost(self).map_err(|e| e.context(self.reading()))
    }

    /// The priority level behind each entry of [`Model::cost`], in the same
    /// order (highest priority first). Empty for a program without
    /// optimisation statements, exactly as [`Model::cost`] is.
    ///
    /// [`Model::cost`] already lists its levels from highest to lowest
    /// priority without this: `priorities` mainly exists for completeness,
    /// and for callers who read clingo's own level numbers directly.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs out
    /// of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a. b. :~ a. [1@1] :~ b. [1@2]")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let model = handle.next_model()?.expect("the program has a model");
    /// assert_eq!(model.cost()?, [1, 1]);
    /// assert_eq!(model.priorities()?, [2, 1]);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn priorities(&self) -> Result<Vec<i32>> {
        raw::model_priority(self).map_err(|e| e.context(self.reading()))
    }

    /// The kind of model this is.
    ///
    /// Every model of an ordinary solve is [`ModelKind::StableModel`]. While
    /// clingo enumerates brave or cautious consequences
    /// (`Configuration::set("solve.enum_mode", "brave")` or `"cautious"`),
    /// every model it lends is [`ModelKind::BraveConsequences`] or
    /// [`ModelKind::CautiousConsequences`] instead: see
    /// [`Model::is_consequence`] for what its symbols then mean.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unknown`](crate::ErrorKind::Unknown) if clingo reports a
    /// model kind this version of clingox does not know, or if clingo runs
    /// out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, ModelKind, Part};
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.configuration().set("solve.enum_mode", "brave")?;
    /// ctl.add_base("1{a;b}1.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let model = handle.next_model()?.expect("the program has a model");
    /// assert_eq!(model.kind()?, ModelKind::BraveConsequences);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn kind(&self) -> Result<ModelKind> {
        match raw::model_type(self).map_err(|e| e.context(self.reading()))? {
            raw::ModelType::StableModel => Ok(ModelKind::StableModel),
            raw::ModelType::BraveConsequences => Ok(ModelKind::BraveConsequences),
            raw::ModelType::CautiousConsequences => Ok(ModelKind::CautiousConsequences),
            raw::ModelType::Other(value) => Err(Error::new(
                ErrorKind::Unknown,
                format!("clingo reported an unrecognized model kind ({value})"),
            )
            .context(self.reading())),
        }
    }

    /// Whether `literal` is a consequence of the program.
    ///
    /// Outside brave or cautious enumeration, this is never
    /// [`Consequence::Unknown`] (clingo's own fallback: "the function just
    /// returns whether a literal is true or false in the current model"), but
    /// it agrees with [`Model::is_true`] on the same literal only for a literal
    /// clingo itself would show or project.
    ///
    /// **A literal that is not shown or projected disagrees.** clingo's
    /// implementation (`libclingo/clingo/clingocontrol.hh:427-439`) forces
    /// `Consequence::False` for one, whether or not it is true in the model, in
    /// two distinct ways:
    /// - **Explicit projection** (clingo's `--project`, or
    ///   [`Control::update_project`](crate::Control::update_project)): with `a.
    ///   b. #project a.` solved under `--project`, the model has `is_true(b)`
    ///   true (`b` is in the answer set) but `is_consequence(b)` false (`b` is
    ///   not projected). See `clingox/tests/
    ///   theory_and_load_regressions.rs::is_consequence_disagrees_with_is_true_under_projection`.
    /// - **A negative literal, even without `--project` at all.** Without
    ///   explicit projection clingo falls back to whether the literal is
    ///   *shown*, and only a positive literal of a shown atom ever is: the
    ///   negation of a shown atom is not itself shown. So `is_consequence` on a
    ///   negative literal that is true (its atom is false in the model) is
    ///   `Consequence::False`, disagreeing with `is_true`, for an ordinary
    ///   program with no projection involved at all. Checked directly against
    ///   the Python module `clingo` 5.8.2, 2026-09-27.
    ///
    /// During brave or cautious enumeration it instead reports partial
    /// information about the running union (brave) or intersection (cautious)
    /// of every model seen so far in this search: a literal already decided,
    /// because it is true in some model (brave) or false in some model
    /// (cautious), is [`Consequence::True`] or [`Consequence::False`] at once,
    /// and one not yet decided is [`Consequence::Unknown`] until a later model
    /// settles it.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unknown`](crate::ErrorKind::Unknown) if clingo reports a
    /// consequence value this version of clingox does not know, or if clingo
    /// runs out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Consequence, Control, Part};
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.configuration().set("solve.enum_mode", "cautious")?;
    /// ctl.add_base("1{a;b}1. c.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let c = ctl
    ///     .symbolic_atoms()?
    ///     .find(clingox::Symbol::function("c", &[])?)?
    ///     .expect("c is an atom")
    ///     .literal();
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let model = handle.next_model()?.expect("the program has a model");
    /// assert_eq!(model.is_consequence(c)?, Consequence::True);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn is_consequence(&self, literal: ProgramLiteral) -> Result<Consequence> {
        let context = || format!("reading literal {}", literal.get());
        match raw::model_is_consequence(self, literal.get()).map_err(|e| e.context(context()))? {
            raw::ConsequenceValue::False => Ok(Consequence::False),
            raw::ConsequenceValue::True => Ok(Consequence::True),
            raw::ConsequenceValue::Unknown => Ok(Consequence::Unknown),
            raw::ConsequenceValue::Other(value) => Err(Error::new(
                ErrorKind::Unknown,
                format!("clingo reported an unrecognized consequence value ({value})"),
            )
            .context(context())),
        }
    }

    /// Whether clingo has proven the model optimal.
    ///
    /// Under the default `--opt-mode=opt`, models lent during a search report
    /// `false`, even an optimal one: clingo sets the flag only on the model it
    /// returns after the search, which is what
    /// [`Control::solve_optimal`](crate::Control::solve_optimal) returns. Under
    /// `--opt-mode=optN`, clingo first searches for the optimum, whose models
    /// report `false`, then enumerates the optimal models, which report
    /// `true` (and are numbered from 1 again, see [`Model::number`]).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unknown`](crate::ErrorKind::Unknown) if clingo fails.
    pub fn optimality_proven(&self) -> Result<bool> {
        raw::model_optimality_proven(self).map_err(|e| e.context(self.reading()))
    }

    /// The symbols `show` selects, in clingo's order.
    ///
    /// clingo's order is not [`Symbol`]'s order; sort the result if the order
    /// matters.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs out
    /// of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, ShowType};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("p(1). q. #show p/1. #show 42.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let model = handle.next_model()?.expect("the program has a model");
    /// assert_eq!(model.symbols(ShowType::SHOWN)?.len(), 2);
    /// assert_eq!(model.symbols(ShowType::ATOMS)?.len(), 2);
    /// assert_eq!(model.symbols(ShowType::TERMS)?.len(), 1);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn symbols(&self, show: ShowType) -> Result<Vec<Symbol>> {
        let symbols =
            raw::model_symbols(self, show.bits()).map_err(|e| e.context(self.reading()))?;
        Ok(symbols.into_iter().map(Symbol::from_clingo).collect())
    }

    /// Whether `atom` is an atom true in the model, shown or not.
    ///
    /// It is false for a shown term, since a term is not an atom, and for a
    /// symbol that is not an atom of the program.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unknown`](crate::ErrorKind::Unknown) if clingo fails.
    pub fn contains(&self, atom: Symbol) -> Result<bool> {
        raw::model_contains(self, atom.raw()).map_err(|e| e.context(self.reading()))
    }

    /// Copies the model into an [`OwnedModel`], which outlives the search.
    ///
    /// # Errors
    ///
    /// As [`Model::symbols`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("p(2). p(1).")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let snapshot = handle.next_model()?.expect("a model").snapshot()?;
    /// drop(handle);
    /// assert_eq!(snapshot.to_string(), "Answer 1: p(1) p(2)");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn snapshot(&self) -> Result<OwnedModel> {
        Ok(OwnedModel {
            number: self.number(),
            cost: self.cost()?,
            priorities: self.priorities()?,
            optimality_proven: self.optimality_proven()?,
            symbols: sorted(self.symbols(ShowType::SHOWN)?),
            all_atoms: sorted(self.symbols(ShowType::ATOMS)?),
        })
    }

    /// Every atom of the predicate `T` true in the model, shown or not, as
    /// values of `T`.
    ///
    /// It reads [`ShowType::ATOMS`], so a `#show` elsewhere in the program
    /// cannot hide the atoms. A symbol matches when it is a positive function
    /// with the name [`T::NAME`](Predicate::NAME) and the arity
    /// [`T::ARITY`](Predicate::ARITY); other symbols, including the classical
    /// negation `-p(1)` of a predicate `p/1`, are skipped. The values come in
    /// [`Symbol`]'s order of their atoms, so a model and its
    /// [`snapshot`](Model::snapshot) give the same list.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Conversion`](crate::ErrorKind::Conversion) if a matching
    ///   atom does not convert: `p(a)` read as `struct P(i32)` is an error,
    ///   never skipped. It is the error of [`FromSymbol::from_symbol`], whose
    ///   message contains the atom;
    /// - the errors of [`Model::symbols`], with their kind.
    ///
    /// [`FromSymbol::from_symbol`]: crate::FromSymbol::from_symbol
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, FromSymbol, Part};
    ///
    /// #[derive(FromSymbol, Debug, PartialEq)]
    /// struct P(i32);
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("p(2). p(1). -p(3). #show.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let model = handle.next_model()?.expect("the program has a model");
    /// assert_eq!(model.atoms::<P>()?, [P(1), P(2)]);
    /// assert_eq!(model.shown::<P>()?, []);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn atoms<T: Predicate>(&self) -> Result<Vec<T>> {
        read(&self.symbols(ShowType::ATOMS)?)
    }

    /// The shown symbols of the predicate `T`, as values of `T`.
    ///
    /// It reads [`ShowType::SHOWN`]: the atoms `#show` selects, and shown
    /// terms that are not atoms, such as `t(1)` from `#show t(X) : p(X).`
    /// Symbols match as in [`Model::atoms`], and come in the same order.
    ///
    /// # Errors
    ///
    /// As [`Model::atoms`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, FromSymbol, Part};
    ///
    /// #[derive(FromSymbol, Debug, PartialEq)]
    /// struct T(i32);
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("p(1). p(2). #show t(X) : p(X).")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let model = handle.next_model()?.expect("the program has a model");
    /// assert_eq!(model.shown::<T>()?, [T(1), T(2)]);
    /// // `t/1` is a shown term, not an atom.
    /// assert_eq!(model.atoms::<T>()?, []);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn shown<T: Predicate>(&self) -> Result<Vec<T>> {
        read(&self.symbols(ShowType::SHOWN)?)
    }

    /// The id of the solver thread that found the model.
    ///
    /// Consecutive numbers from zero, the same `clingo_id_t` space
    /// [`PropagateControl::thread_id`](crate::propagate::PropagateControl::thread_id)
    /// reports (checked directly against clingo 5.8.2 at four threads).
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let model = handle.next_model()?.expect("a. has a model");
    /// assert_eq!(model.thread_id(), 0);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn thread_id(&self) -> u32 {
        raw::model_thread_id(self)
    }

    /// The solve control of this model: lets a caller narrow the rest of the
    /// current solving step's enumeration, with [`SolveControl::add_clause`],
    /// without registering a [`Propagator`](crate::propagate::Propagator) at
    /// all.
    ///
    /// Reachable with no propagator registered anywhere, from every path a
    /// model is delivered: a plain
    /// [`SolveHandle::next_model`](crate::SolveHandle::next_model) loop,
    /// [`Control::for_each_model`](crate::Control::for_each_model), and
    /// [`ExtendableModel`] inside a
    /// [`SolveEventHandler::on_model`](crate::SolveEventHandler::on_model)
    /// (checked directly against clingo 5.8.2 on all three);
    /// `clingo_model_context` is a bare reinterpretation of whatever
    /// `clingo_model_t*` a caller already holds, not a genuinely different
    /// object.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::with_args(["--models=0"])?;
    /// ctl.add_base("{a;b}.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let mut handle = ctl.solve_yield(&[])?;
    /// let mut seen = 0;
    /// while let Some(model) = handle.next_model()? {
    ///     seen += 1;
    ///     let a = clingox::Symbol::function("a", &[])?;
    ///     if model.contains(a)? {
    ///         // Negate `a`'s own literal: no later model in this solving
    ///         // step may have `a` true again.
    ///         let lit = model.context().symbolic_atoms()?.find(a)?.expect("a is an atom").literal();
    ///         model.context().add_clause(&[-lit])?;
    ///     }
    /// }
    /// let _ = handle.close()?;
    /// assert_eq!(seen, 3, "{{}}, {{b}}, {{a}}: {{a,b}} is excluded once {{a}} is seen");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn context(&self) -> SolveControl<'_> {
        SolveControl::from_raw(raw::model_context(self))
    }

    /// The context for errors: which model was being read.
    fn reading(&self) -> String {
        format!("reading model {}", self.number())
    }
}

/// The solve control of a [`Model`], letting a caller narrow the current
/// solving step's own enumeration without a registered
/// [`Propagator`](crate::propagate::Propagator) ([`Model::context`]).
///
/// `Model` and `SolveControl` are both views over the same opaque clingo
/// object (`clingo_model_context`'s own C body reinterprets the model's own
/// pointer): `Model`
/// is zero-sized, and `SolveControl` never forms a Rust reference into
/// clingo's object either, only ever handing its raw pointer to a C
/// function. A live `&Model` and a `SolveControl` borrowed from it
/// therefore coexist safely, including reading the model again after
/// [`SolveControl::add_clause`] ran in the same callback: proven directly
/// under `ASan` (`clingox/tests/api_model_context.rs::reading_the_model_after_
/// add_clause_in_the_same_callback_works`) and, for this type's own pointer
/// handling, under Miri (`clingox/src/raw/model.rs`'s `#[cfg(test)] mod
/// tests`), since Miri cannot call into clingo itself.
///
/// Borrowed for exactly the duration of the [`Model`] it came from: keeping
/// one past its callback is a compile error (`clingox/tests/ui/
/// solve_control_escapes_the_model_callback.rs`), the same shape
/// [`ExtendableModel`]'s own lifetime already has.
///
/// # Examples
///
/// See [`Model::context`].
pub struct SolveControl<'m> {
    raw: raw::RawSolveControl<'m>,
}

impl<'m> SolveControl<'m> {
    pub(crate) fn from_raw(raw: raw::RawSolveControl<'m>) -> SolveControl<'m> {
        SolveControl { raw }
    }

    /// The symbolic atoms of this model's own control, the same view
    /// [`Control::symbolic_atoms`](crate::Control::symbolic_atoms) returns
    /// (checked directly against clingo 5.8.2).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs out
    /// of memory.
    pub fn symbolic_atoms(&self) -> Result<SymbolicAtoms<'_>> {
        let atoms = self
            .raw
            .symbolic_atoms()
            .map_err(|e| e.context("reading the symbolic atoms of a model's solve control"))?;
        Ok(SymbolicAtoms::from_parts(ErrorSink::None, atoms))
    }

    /// Adds `clause` to the solver: no later model of the *current* solving
    /// step may violate it. It does not persist to a later, separate
    /// [`Control::solve`](crate::Control::solve) call on the same control
    /// (confirmed directly, the header's own text, "applies to the current
    /// solving step during model enumeration, " `H:2461-2462`).
    ///
    /// **`clause` takes plain [`ProgramLiteral`]s, and clingox adds no
    /// validation of its own before handing them to clingo**, unlike every
    /// [`SolverLiteral`](crate::propagate::SolverLiteral)-taking method of
    /// [`PropagateInit`](crate::propagate::PropagateInit)/
    /// [`PropagateControl`](crate::propagate::PropagateControl): a literal
    /// naming an atom of a *different* program is accepted silently, never
    /// rejected and never unsafe, matching [`ProgramLiteral`]'s own,
    /// already-published contract ("clingox cannot detect that; the result is
    /// wrong but never unsafe"). Reproduced directly against clingo 5.8.2, one
    /// process per magnitude tried: no value crashed the process, and every
    /// magnitude reachable through [`ProgramLiteral::from_raw`] is silently
    /// accepted (checked directly against clingo 5.8.2); only a magnitude
    /// already unreachable through the typed API reports
    /// [`ErrorKind::Logic`](crate::ErrorKind::Logic). A literal outside the
    /// program's own range still makes clingo allocate memory in proportion to
    /// it (`docs/dev/UPSTREAM-ISSUES.md`'s U22).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs out
    ///   of memory;
    /// - [`ErrorKind::Runtime`](crate::ErrorKind::Runtime) if adding the clause
    ///   fails.
    pub fn add_clause(&self, clause: &[ProgramLiteral]) -> Result<()> {
        let raw_clause: Vec<i32> = clause.iter().map(|l| l.get()).collect();
        self.raw
            .add_clause(&raw_clause)
            .map_err(|e| e.context("adding a clause during model enumeration"))
    }
}

impl fmt::Debug for SolveControl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SolveControl").finish_non_exhaustive()
    }
}

/// A model that may still be extended with symbols, lent to
/// [`SolveEventHandler::on_model`](crate::SolveEventHandler::on_model) (DESIGN
/// S6).
///
/// It derefs to [`Model`], so every ordinary reading method
/// ([`symbols`](Model::symbols), [`contains`](Model::contains),
/// [`cost`](Model::cost), and the rest) applies unchanged; [`extend`] is the
/// one method only this type has, because [`clingo_model_extend`] needs a
/// non-const model pointer, unlike every other model-reading function, and only
/// the model delivered to a solve event is ever non-const
/// (`clingo.h:2431-2434`: "Only models passed to the
/// `clingo_solve_event_callback_t` are extendable").
///
/// **Nothing here can outlive the callback that received it.** The borrow's
/// lifetime is exactly the duration of one `on_model` call, the same as every
/// other `SolveEvent` payload;
/// `tests/ui/extendable_model_escapes_the_callback.rs` pins the compile error.
///
/// [`extend`]: ExtendableModel::extend
/// [`clingo_model_extend`]: https://potassco.org/clingo/c-api/current/structclingo__model.html
///
/// # Examples
///
/// ```
/// use std::ops::ControlFlow;
///
/// use clingox::{Control, ExtendableModel, Part, ShowType, SolveEventHandler, SolveOptions, Symbol};
///
/// struct AddSeventeen;
///
/// impl SolveEventHandler for AddSeventeen {
///     fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> clingox::Result<ControlFlow<()>> {
///         model.extend([Symbol::number(17)])?;
///         // The header's own caveat: an extension only shows up in clingo's
///         // *output*, so it is read back inside this same callback.
///         assert!(model.symbols(ShowType::THEORY)?.contains(&Symbol::number(17)));
///         Ok(ControlFlow::Continue(()))
///     }
/// }
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("a.")?;
/// ctl.ground(&[Part::base()])?;
/// ctl.solve_with_events(SolveOptions::new(), AddSeventeen)?;
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct ExtendableModel<'m> {
    model: &'m mut Model,
}

impl<'m> ExtendableModel<'m> {
    /// Wraps an already-formed mutable model reference. The `unsafe` cast
    /// from clingo's raw, non-const model pointer happens once, in
    /// `raw::extendable_model`, called only from the composed solve-event
    /// trampoline (`raw::events`); this constructor itself needs no
    /// `unsafe`.
    pub(crate) fn from_raw(model: &'m mut Model) -> ExtendableModel<'m> {
        ExtendableModel { model }
    }

    /// Adds `symbols` to the model.
    ///
    /// These symbols appear in clingo's own printed output, which means
    /// this is only meaningful to an application that prints models;
    /// clingox is a library and prints nothing on its own, so the only way
    /// to observe an extension is to read it back through this same
    /// model's own reading methods, inside the callback that received it
    /// (`clingo.h:2431-2434`).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs
    /// out of memory.
    pub fn extend<I>(&mut self, symbols: I) -> Result<()>
    where
        I: IntoIterator<Item = Symbol>,
    {
        let symbols: Vec<raw::RawSymbol> = symbols.into_iter().map(Symbol::raw).collect();
        raw::model_extend(self.model, &symbols)
            .map_err(|e| e.context(format!("extending model {}", self.model.number())))
    }
}

impl Deref for ExtendableModel<'_> {
    type Target = Model;

    fn deref(&self) -> &Model {
        self.model
    }
}

impl fmt::Debug for ExtendableModel<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ExtendableModel").field(self.model).finish()
    }
}

fn sorted(mut symbols: Vec<Symbol>) -> Vec<Symbol> {
    symbols.sort_unstable();
    symbols
}

/// Writes clingox's one-line form of a model: `Answer <number>: <symbols>`.
fn write_answer(f: &mut fmt::Formatter<'_>, number: u64, symbols: &[Symbol]) -> fmt::Result {
    write!(f, "Answer {number}:")?;
    for symbol in symbols {
        write!(f, " {symbol}")?;
    }
    Ok(())
}

impl fmt::Display for Model {
    /// clingo fails here only when it runs out of memory, which is reported
    /// as [`fmt::Error`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let symbols = self.symbols(ShowType::SHOWN).map_err(|_| fmt::Error)?;
        write_answer(f, self.number(), &sorted(symbols))
    }
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// A value, or the error that kept clingo from reporting it.
        struct Read<T>(Result<T>);
        impl<T: fmt::Debug> fmt::Debug for Read<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match &self.0 {
                    Ok(value) => value.fmt(f),
                    Err(err) => write!(f, "<{err}>"),
                }
            }
        }
        f.debug_struct("Model")
            .field("number", &self.number())
            .field("cost", &Read(self.cost()))
            .field("symbols", &Read(self.symbols(ShowType::SHOWN).map(sorted)))
            .finish()
    }
}

/// A copy of a model that outlives the search, made by [`Model::snapshot`].
///
/// Its symbols are sorted in [`Symbol`]'s order. It is `Send`, `Sync` and
/// `'static`, and prints as its [`Model`] does, in clingox's one-line format
/// `Answer 1: p(1) p(2)`.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part, Symbol};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("p(2). p(1). hidden. #show p/1.")?;
/// ctl.ground(&[Part::base()])?;
/// let (_, models) = ctl.solve_all()?;
/// let model = &models[0];
/// assert_eq!(model.symbols(), ["p(1)".parse::<Symbol>()?, "p(2)".parse()?]);
/// assert_eq!(model.all_atoms().len(), 3);
/// assert!(model.contains(Symbol::function("hidden", &[])?));
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OwnedModel {
    number: u64,
    cost: Vec<i64>,
    priorities: Vec<i32>,
    optimality_proven: bool,
    symbols: Vec<Symbol>,
    all_atoms: Vec<Symbol>,
}

impl OwnedModel {
    /// The running number the model had in its solve call. Under
    /// `--opt-mode=optN`, the enumerated optimal models are numbered from 1
    /// again (see [`Model::number`]).
    pub fn number(&self) -> u64 {
        self.number
    }

    /// The cost of the model, highest priority first; empty without
    /// optimisation statements.
    pub fn cost(&self) -> &[i64] {
        &self.cost
    }

    /// The priority level of each entry of [`cost`](OwnedModel::cost), highest
    /// first, as [`Model::priorities`] reads them from a borrowed model; empty
    /// without optimisation statements.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a. b. :~ a. [3@5] :~ b. [7@1]")?;
    /// ctl.ground(&[Part::base()])?;
    /// let (_, models) = ctl.solve_all()?;
    /// assert_eq!(models[0].cost(), [3, 7]);
    /// assert_eq!(models[0].priorities(), [5, 1]);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn priorities(&self) -> &[i32] {
        &self.priorities
    }

    /// Whether clingo had proven the model optimal when it was copied: under
    /// `--opt-mode=opt` only for the model
    /// [`Control::solve_optimal`](crate::Control::solve_optimal) returns after
    /// a finished search, under `--opt-mode=optN` also for every enumerated
    /// optimal model (see [`Model::optimality_proven`]).
    pub fn optimality_proven(&self) -> bool {
        self.optimality_proven
    }

    /// The shown symbols ([`ShowType::SHOWN`]), sorted.
    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    /// Every atom true in the model ([`ShowType::ATOMS`]), shown or not,
    /// sorted.
    pub fn all_atoms(&self) -> &[Symbol] {
        &self.all_atoms
    }

    /// Whether `atom` is an atom true in the model, shown or not, as
    /// [`Model::contains`] answered for the model it was copied from.
    pub fn contains(&self, atom: Symbol) -> bool {
        self.all_atoms.binary_search(&atom).is_ok()
    }
}

impl OwnedModel {
    /// Every atom of the predicate `T` true in the model, shown or not, as
    /// [`Model::atoms`] reads them from the model this was copied from.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Conversion`](crate::ErrorKind::Conversion) if a matching
    /// atom does not convert, as for [`Model::atoms`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, FromSymbol, Outcome, Part};
    ///
    /// #[derive(FromSymbol, Debug, PartialEq)]
    /// struct Edge(i32, i32);
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("edge(1,2). edge(2,3). #show.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let Outcome::Sat(model, _) = ctl.solve_first()? else {
    ///     panic!("the program has a model");
    /// };
    /// assert_eq!(model.atoms::<Edge>()?, [Edge(1, 2), Edge(2, 3)]);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn atoms<T: Predicate>(&self) -> Result<Vec<T>> {
        read(&self.all_atoms)
    }

    /// The shown symbols of the predicate `T`, as values of `T`, as
    /// [`Model::shown`] reads them.
    ///
    /// # Errors
    ///
    /// As [`OwnedModel::atoms`].
    pub fn shown<T: Predicate>(&self) -> Result<Vec<T>> {
        read(&self.symbols)
    }
}

/// The values of the symbols of predicate `T`, in [`Symbol`]'s order. A
/// matching symbol that does not convert is an error, never skipped.
fn read<T: Predicate>(symbols: &[Symbol]) -> Result<Vec<T>> {
    let arity = usize::try_from(T::ARITY).unwrap_or(usize::MAX);
    let mut matching: Vec<Symbol> = symbols
        .iter()
        .copied()
        .filter(|symbol| {
            symbol
                .function_parts()
                .is_some_and(|(name, arguments, sign)| {
                    sign == Sign::Positive && name == T::NAME && arguments.len() == arity
                })
        })
        .collect();
    matching.sort_unstable();
    matching.into_iter().map(T::from_symbol).collect()
}

impl fmt::Display for OwnedModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_answer(f, self.number, &self.symbols)
    }
}

/// Which symbols of a model [`Model::symbols`] selects.
///
/// The flags combine with `|`: `ShowType::ATOMS | ShowType::TERMS` selects
/// both. [`ShowType::ALL`] does not include [`ShowType::COMPLEMENT`], as in
/// clingo. A `ShowType` holds only clingo's flags; it cannot be built from
/// other bits.
///
/// # Examples
///
/// ```
/// use clingox::ShowType;
///
/// let both = ShowType::ATOMS | ShowType::TERMS;
/// assert!(both.contains(ShowType::ATOMS));
/// assert!(!ShowType::ALL.contains(ShowType::COMPLEMENT));
/// assert_eq!(format!("{both:?}"), "ShowType(ATOMS | TERMS)");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShowType(u32);

impl ShowType {
    /// The shown atoms and terms, as `#show` selects them.
    pub const SHOWN: ShowType = ShowType(raw::show::SHOWN);
    /// Every atom true in the model, shown or not.
    pub const ATOMS: ShowType = ShowType(raw::show::ATOMS);
    /// Every shown term, such as `42` from `#show 42.`.
    pub const TERMS: ShowType = ShowType(raw::show::TERMS);
    /// The symbols added by theories.
    pub const THEORY: ShowType = ShowType(raw::show::THEORY);
    /// With [`ShowType::ATOMS`] or [`ShowType::TERMS`], the false atoms or
    /// terms instead of the true ones.
    pub const COMPLEMENT: ShowType = ShowType(raw::show::COMPLEMENT);
    /// `SHOWN | ATOMS | TERMS | THEORY`.
    ///
    /// clingo's own `clingo_show_type_all` also sets the bit of a removed CSP
    /// flag; this value holds the four flags only, as Python clingo's `all`
    /// passes them. The symbols selected are the same.
    pub const ALL: ShowType =
        ShowType(raw::show::SHOWN | raw::show::ATOMS | raw::show::TERMS | raw::show::THEORY);

    /// The flags in the order `Debug` names them.
    const NAMED: [(ShowType, &'static str); 5] = [
        (ShowType::SHOWN, "SHOWN"),
        (ShowType::ATOMS, "ATOMS"),
        (ShowType::TERMS, "TERMS"),
        (ShowType::THEORY, "THEORY"),
        (ShowType::COMPLEMENT, "COMPLEMENT"),
    ];

    /// Whether every flag of `other` is set in `self`.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ShowType;
    ///
    /// assert!(ShowType::ALL.contains(ShowType::ATOMS | ShowType::TERMS));
    /// assert!(!ShowType::ATOMS.contains(ShowType::ATOMS | ShowType::TERMS));
    /// ```
    pub const fn contains(self, other: ShowType) -> bool {
        self.0 & other.0 == other.0
    }

    /// clingo's bitset for these flags (clingo.h:2290-2297).
    pub(crate) fn bits(self) -> u32 {
        self.0
    }
}

impl std::ops::BitOr for ShowType {
    type Output = ShowType;

    fn bitor(self, other: ShowType) -> ShowType {
        ShowType(self.0 | other.0)
    }
}

impl std::ops::BitOrAssign for ShowType {
    fn bitor_assign(&mut self, other: ShowType) {
        self.0 |= other.0;
    }
}

impl fmt::Debug for ShowType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShowType(")?;
        let mut first = true;
        for (flag, name) in ShowType::NAMED {
            if self.contains(flag) {
                if !first {
                    f.write_str(" | ")?;
                }
                first = false;
                f.write_str(name)?;
            }
        }
        f.write_str(")")
    }
}

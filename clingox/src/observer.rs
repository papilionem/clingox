//! Watching the ground program as it is built: [`GroundProgramObserver`] and
//! [`Control::register_observer`], plus the grounding-size guard
//! ([`GroundingLimit`] and [`LimitedObserver`]).
//!
//! An observer is the read side of what [`Backend`](crate::backend::Backend)
//! writes: the backend hands clingo new ground directives, and an observer is
//! told about every directive clingo already has, from any source (program
//! text, the ground callback, or the backend itself), as it reaches the solver.
//! `clingo_control_register_backend`
//! ([`Control::register_backend_writer`](crate::Control::register_backend_writer),
//! defined in [`crate::backend`]) is a third, narrower mechanism: it tells
//! clingo to dump its own ground output to a file in a fixed format, rather
//! than handing it to arbitrary Rust code.
//!
//! ```
//! use clingox::observer::GroundProgramObserver;
//! use clingox::{Control, Part};
//!
//! #[derive(Default)]
//! struct CountRules(u32);
//!
//! impl GroundProgramObserver for CountRules {
//!     fn rule(
//!         &mut self,
//!         _choice: bool,
//!         _head: &[clingox::backend::Atom],
//!         _body: &[clingox::ProgramLiteral],
//!     ) -> clingox::Result<()> {
//!         self.0 += 1;
//!         Ok(())
//!     }
//! }
//!
//! let mut ctl = Control::new()?;
//! ctl.register_observer(CountRules::default(), false)?;
//! ctl.add_base("a. b :- a. c :- a, b.")?;
//! ctl.ground(&[Part::base()])?;
//! # Ok::<(), clingox::Error>(())
//! ```

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::collections::HashSet;

use crate::backend::{Atom, ExternalKind, HeuristicKind};
use crate::control::Part;
use crate::error::{Error, ErrorKind, Result};
use crate::symbol::Symbol;
use crate::theory::Id;

/// Watches the ground program as clingo builds it (`clingo_ground_program_
/// observer_t`, clingo.h:2648-2848), one method per callback.
///
/// Every method has a default `Ok(())` body: implement only the ones a given
/// observer cares about, exactly as the header's own "not all callbacks have to
/// be implemented" note describes for the C struct
/// ([`Control::register_observer`] builds one with every field set, since there
/// is no meaningful "unset" state for a Rust trait method the way `NULL` is for
/// a C function pointer; a default body is the same thing at this layer).
///
/// **Ownership and lifetime.** Unlike the ground callback
/// ([`Control::ground_with`], borrowed and scoped to one call), an observer is
/// `Send + 'static` and owned by the [`Control`] for as long as it stays
/// registered (DESIGN S10's table: "observer | caller's thread during ground
/// | `Send + 'static`, owned"). clingo itself has no way to unregister one:
/// [`Control::register_observer`] composes repeated registrations rather than
/// replacing them (unless `replace` is set), so an observer registered once
/// keeps seeing every later grounding on that control until it is dropped.
///
/// **Reentrancy.** Every callback runs on the thread that called whichever
/// entry point triggered it (below), while that call still holds `&mut Control`
/// (DESIGN S5). Nothing in the public API lets a callback obtain a second
/// handle to the *same* control (`Control` is not `Clone` and never appears
/// behind a public `Rc`/`Arc`), so this can only be reached through a caller's
/// own `unsafe` code smuggling one in; the rule is documented, not statically
/// enforced, in the same spirit as the propagator's "never hold your own lock
/// across a call into `PropagateControl`" (DESIGN S11). Never call back into
/// the control that owns this observer from inside one of its own callbacks.
///
/// **Every entry point through which clingo can call a registered observer**.
/// `resolve_ground` was, before that review, the only place clingox ever read
/// an observer's recorded error or panic (`raw::ControlHandle`); every other
/// one now reads it too, right after its own call, giving it priority over
/// anything else that call would otherwise report:
///
/// - [`Control::ground`] and [`Control::ground_with`]: a directive from program
///   text or a ground callback (`clingo_control_ground`, clingo.h:3066).
/// - [`Control::with_backend`], twice over: synchronously inside its own
///   closure, for a directive such as `add_rule` (`clingo_backend_rule` ->
///   `outputRule`, `libclingo/src/control.cc:1230-1237`), and once more at the
///   backend's own close, for a fact's delayed notification
///   (`clingo_backend_end` -> `ClingoControl::endAddBackend`, which grounds the
///   accumulated backend program into the same observer chain,
///   `libclingo/src/clingocontrol.cc:880-894`).
/// - [`Control::load_aspif`]: `clingo_control_load_aspif` -> `ClingoControl::
///   load_aspif` -> `NonGroundParser::parse_aspif`, which feeds the parsed
///   directives into the same output pipeline `ground`/`ground_with` use
///   (`libclingo/src/clingocontrol.cc:379-390`).
/// - [`Control::assign_external`]/[`Control::release_external`]:
///   `clingo_control_assign_external`/`_release_external` ->
///   `ClingoControl::assignExternal` -> `out_->backend()->external(...)`, the
///   observer's own `external` callback
///   (`libclingo/src/clingocontrol.cc:671-678`).
/// - [`Control::update_project`]: `clingo_control_update_project` ->
///   `ClingoControl::updateProject` -> `out_->backend()->project(...)`, the
///   observer's own `project` callback (`clingocontrol.cc:680-688`).
/// - Every entry point that starts a search: [`Control::solve`],
///   [`Control::solve_yield`], [`Control::solve_yield_with_events`],
///   [`Control::solve_async`], [`Control::solve_async_with_events`] and
///   [`Control::solve_with_events`]. `end_step` fires when a solve starts, not
///   only when grounding finishes: `clingo_control_solve` calls
///   `ClingoControl::solve`, which calls `prepare`
///   (`libclingo/src/clingocontrol.cc:437-438`), and `prepare` calls
///   `out_->endStep`, reaching `Observer::endStep` -> `call(obs_.end_step)`
///   (`clingocontrol.cc:558-561`, `control.cc:2221`), before any handle is even
///   returned to the caller -- so a failing `end_step` can make the solve call
///   itself fail outright, exactly as a failing directive can make
///   `clingo_backend_rule` fail.
///
/// **Errors stop grounding, or the entry point above, and poison the control.**
/// Returning `Err` from any method stops grounding (or the backend directive,
/// or the solve) at once: no later callback of this or any other grounding on
/// this control's observers fires, and the entry point returns the same error,
/// unchanged, poisoning the control whatever its kind (this holds for every
/// path above; see the poisoning note on [`Control`]). A panic is caught,
/// recorded, and resumed on the caller's thread once the call returns,
/// poisoning the control the same way, with the panic's own payload untouched
/// -- no entry point adds its own context to it.
///
/// # Examples
///
/// See the module documentation for a minimal observer, and [`LimitedObserver`]
/// for the grounding-size guard built on this trait.
pub trait GroundProgramObserver: Send + 'static {
    /// Called once at the start of grounding; `incremental` is whether the
    /// program may be grounded and solved more than once.
    ///
    /// # Errors
    ///
    /// Whatever should stop grounding; see the trait documentation's
    /// "Errors stop grounding" note.
    fn init_program(&mut self, incremental: bool) -> Result<()> {
        let _ = incremental;
        Ok(())
    }
    /// Marks the start of a step: the span from the previous
    /// [`GroundProgramObserver::end_step`] (or the very start) up to the next
    /// [`Control::solve`].
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn begin_step(&mut self) -> Result<()> {
        Ok(())
    }
    /// Marks the end of a step, right before solving starts. Several
    /// [`Control::ground`] calls in a row, with no [`Control::solve`] between
    /// them, are one step: this fires once for the whole span, not once per
    /// `ground` call.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn end_step(&mut self) -> Result<()> {
        Ok(())
    }
    /// A rule: a disjunctive or choice head over `body`.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn rule(&mut self, choice: bool, head: &[Atom], body: &[crate::ProgramLiteral]) -> Result<()> {
        let (_, _, _) = (choice, head, body);
        Ok(())
    }
    /// A weight rule: `head` is derived once the body's true literals meet
    /// `lower_bound`.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn weight_rule(
        &mut self,
        choice: bool,
        head: &[Atom],
        lower_bound: i32,
        body: &[(crate::ProgramLiteral, i32)],
    ) -> Result<()> {
        let (_, _, _, _) = (choice, head, lower_bound, body);
        Ok(())
    }
    /// A minimize (or weak) constraint at `priority`.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn minimize(&mut self, priority: i32, literals: &[(crate::ProgramLiteral, i32)]) -> Result<()> {
        let (_, _) = (priority, literals);
        Ok(())
    }
    /// A projection directive.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn project(&mut self, atoms: &[Atom]) -> Result<()> {
        let _ = atoms;
        Ok(())
    }
    /// A shown atom. `atom` is `None` for a fact: clingo gives a fact's
    /// aspif atom as zero (`H:2732-2736`), since a fact needs no atom of its
    /// own to be true.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn output_atom(&mut self, symbol: Symbol, atom: Option<Atom>) -> Result<()> {
        let (_, _) = (symbol, atom);
        Ok(())
    }
    /// A shown term (`#show term: condition.`), true under `condition`.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn output_term(&mut self, symbol: Symbol, condition: &[crate::ProgramLiteral]) -> Result<()> {
        let (_, _) = (symbol, condition);
        Ok(())
    }
    /// An external statement.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn external(&mut self, atom: Atom, kind: ExternalKind) -> Result<()> {
        let (_, _) = (atom, kind);
        Ok(())
    }
    /// An assumption directive: only ever reported for one authored through
    /// [`Backend::add_assumptions`](crate::backend::Backend::add_assumptions),
    /// since `#assume` has no program-text syntax.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn assume(&mut self, literals: &[crate::ProgramLiteral]) -> Result<()> {
        let _ = literals;
        Ok(())
    }
    /// A domain heuristic directive.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn heuristic(
        &mut self,
        atom: Atom,
        kind: HeuristicKind,
        bias: i32,
        priority: u32,
        condition: &[crate::ProgramLiteral],
    ) -> Result<()> {
        let (_, _, _, _, _) = (atom, kind, bias, priority, condition);
        Ok(())
    }
    /// An acyclicity edge directive.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn acyc_edge(&mut self, u: i32, v: i32, condition: &[crate::ProgramLiteral]) -> Result<()> {
        let (_, _, _) = (u, v, condition);
        Ok(())
    }
    /// A numeric theory term.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn theory_term_number(&mut self, term: Id, number: i32) -> Result<()> {
        let (_, _) = (term, number);
        Ok(())
    }
    /// A string (or symbolic constant) theory term.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn theory_term_string(&mut self, term: Id, name: &str) -> Result<()> {
        let (_, _) = (term, name);
        Ok(())
    }
    /// A compound theory term: a tuple, set, list or function.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn theory_term_compound(
        &mut self,
        term: Id,
        kind: TheoryCompoundKind,
        arguments: &[Id],
    ) -> Result<()> {
        let (_, _, _) = (term, kind, arguments);
        Ok(())
    }
    /// A theory atom element: a term tuple under a condition.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn theory_element(
        &mut self,
        element: Id,
        terms: &[Id],
        condition: &[crate::ProgramLiteral],
    ) -> Result<()> {
        let (_, _, _) = (element, terms, condition);
        Ok(())
    }
    /// A theory atom without a guard. `atom` is `None` for a directive:
    /// clingo gives a directive theory atom's id as zero, the same convention
    /// [`GroundProgramObserver::output_atom`] uses for a fact. Unlike
    /// [`Backend::add_theory_atom`](crate::backend::Backend::add_theory_atom)'s
    /// `TheoryAtomTarget`, the observer never reports a fresh-atom sentinel:
    /// clingo always resolves it to a real atom id (or zero) before this
    /// fires.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn theory_atom(&mut self, atom: Option<Atom>, term: Id, elements: &[Id]) -> Result<()> {
        let (_, _, _) = (atom, term, elements);
        Ok(())
    }
    /// A theory atom with a guard: a connective and a right-hand-side term.
    ///
    /// # Errors
    ///
    /// As [`GroundProgramObserver::init_program`].
    fn theory_atom_with_guard(
        &mut self,
        atom: Option<Atom>,
        term: Id,
        elements: &[Id],
        operator: &str,
        right_hand_side: Id,
    ) -> Result<()> {
        let (_, _, _, _, _) = (atom, term, elements, operator, right_hand_side);
        Ok(())
    }
}

/// The kind of a compound theory term (`clingo_theory_atoms_term_type_e`'s
/// compound cases), for [`GroundProgramObserver::theory_term_compound`].
///
/// The observer reports one merged callback for every compound shape
/// (`name_id_or_type`, `H:2801-2805`), where the backend
/// ([`Backend::add_theory_sequence`](crate::backend::Backend::add_theory_sequence),
/// [`Backend::add_theory_function`](crate::backend::Backend::add_theory_function))
/// has two separate methods: an observer reports what clingo emits, one shape
/// at a time, while the backend builds input, so their shapes need not match.
///
/// # Examples
///
/// ```
/// use clingox::observer::{GroundProgramObserver, TheoryCompoundKind};
/// use clingox::{Control, Id, Part};
///
/// #[derive(Default)]
/// struct Kinds(Vec<TheoryCompoundKind>);
///
/// impl GroundProgramObserver for Kinds {
///     fn theory_term_compound(
///         &mut self,
///         _term: Id,
///         kind: TheoryCompoundKind,
///         _arguments: &[Id],
///     ) -> clingox::Result<()> {
///         self.0.push(kind);
///         Ok(())
///     }
/// }
///
/// let mut ctl = Control::new()?;
/// ctl.register_observer(Kinds::default(), false)?;
/// ctl.add_base("#theory t { term { }; &a/0 : term, any }. x :- &a { (1,) }.")?;
/// ctl.ground(&[Part::base()])?;
/// # Ok::<(), clingox::Error>(())
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TheoryCompoundKind {
    /// A tuple term, `(t1,...,tn)`.
    Tuple,
    /// A set term, `{t1,...,tn}`.
    Set,
    /// A list term, `[t1,...,tn]`.
    List,
    /// A function term, with the [`Id`] of its name (itself a string term).
    Function(Id),
}

/// A cap on how large a grounding may grow (DESIGN S15): the atom or rule count
/// clingo grounds before [`LimitedObserver`] fails the next callback.
///
/// Grounding cannot be interrupted (DESIGN S13), so this guard is cooperative:
/// it can only stop grounding at the boundary between two callbacks, never in
/// the middle of one, and it works entirely by counting what an observer
/// already sees, with no help from clingo itself.
///
/// `max_atoms` counts distinct atom ids reported by any callback (rule heads
/// and bodies, `output_atom`, externals and so on), each counted once however
/// many callbacks mention it; `max_rules` counts one per `rule` or
/// `weight_rule` call, regardless of head size. Neither field enabled (`None`)
/// means no limit on that count.
///
/// `#[non_exhaustive]`: a future field (a limit on some other count grounding
/// tracks, say) must not break every existing struct-literal caller the moment
/// it is added. [`GroundingLimit::new`] builds one from outside the crate;
/// within it, a struct literal (`GroundingLimit { max_atoms, max_rules }`)
/// still works exactly as before, `#[non_exhaustive]` only reaches past the
/// crate boundary.
///
/// # Examples
///
/// ```
/// use clingox::observer::{GroundingLimit, LimitedObserver};
/// use clingox::{Control, ErrorKind, Part};
///
/// #[derive(Default)]
/// struct NoOp;
/// impl clingox::observer::GroundProgramObserver for NoOp {}
///
/// let mut ctl = Control::new()?;
/// let limit = GroundingLimit::new(None, Some(1));
/// ctl.register_observer(LimitedObserver::new(NoOp, limit), false)?;
/// ctl.add_base("a. b.")?;
/// let err = ctl.ground(&[Part::base()]).unwrap_err();
/// assert_eq!(err.kind(), ErrorKind::GroundingLimit);
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct GroundingLimit {
    /// The most distinct atom ids grounding may use, or `None` for no limit.
    pub max_atoms: Option<u64>,
    /// The most `rule`/`weight_rule` calls grounding may make, or `None` for
    /// no limit.
    pub max_rules: Option<u64>,
}

impl GroundingLimit {
    /// Builds a limit from its two counts, either or both `None` for no
    /// limit on that count. The only way to build one from outside the
    /// crate, now that `#[non_exhaustive]` rules out a struct literal there.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::observer::GroundingLimit;
    ///
    /// let limit = GroundingLimit::new(Some(1000), None);
    /// assert_eq!(limit.max_atoms, Some(1000));
    /// assert_eq!(limit.max_rules, None);
    /// ```
    #[must_use]
    pub fn new(max_atoms: Option<u64>, max_rules: Option<u64>) -> GroundingLimit {
        GroundingLimit {
            max_atoms,
            max_rules,
        }
    }
}

/// Wraps a [`GroundProgramObserver`] with a [`GroundingLimit`]: every callback
/// still reaches the wrapped observer, in full, for as long as its own
/// contribution keeps neither count over its limit; the callback whose own
/// contribution pushes one over fails at once, with
/// [`ErrorKind::GroundingLimit`], before reaching the wrapped observer at all
/// (counting happens before checking, so the offending callback is refused
/// itself, not only the next one after it -- a version that checked first left
/// the last callback of a grounding step free to push a count over the limit
/// with no later callback ever refused, and `ground_with_limit` returned `Ok`).
///
/// This is a combinator, not a separate registration path, so it composes with
/// a caller's own observer: wrap it in a `LimitedObserver` and register that
/// instead of registering both separately. [`Control::ground_with_limit`] is a
/// convenience for the common case of no observer of one's own.
///
/// # Examples
///
/// ```
/// use clingox::observer::{GroundProgramObserver, GroundingLimit, LimitedObserver};
/// use clingox::{Control, Part};
///
/// #[derive(Default)]
/// struct CountRules(u32);
/// impl GroundProgramObserver for CountRules {
///     fn rule(
///         &mut self, _choice: bool, _head: &[clingox::backend::Atom],
///         _body: &[clingox::ProgramLiteral],
///     ) -> clingox::Result<()> {
///         self.0 += 1;
///         Ok(())
///     }
/// }
///
/// let mut ctl = Control::new()?;
/// let limit = GroundingLimit::new(None, Some(100));
/// ctl.register_observer(LimitedObserver::new(CountRules::default(), limit), false)?;
/// ctl.add_base("a. b.")?;
/// ctl.ground(&[Part::base()])?;
/// // The wrapped observer still saw every rule: the limit was never hit.
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Debug)]
pub struct LimitedObserver<O> {
    observer: O,
    limit: GroundingLimit,
    atoms: HashSet<u32>,
    rules: u64,
}

impl<O> LimitedObserver<O> {
    /// Wraps `observer` with `limit`.
    pub fn new(observer: O, limit: GroundingLimit) -> LimitedObserver<O> {
        LimitedObserver {
            observer,
            limit,
            atoms: HashSet::new(),
            rules: 0,
        }
    }
}

impl<O> LimitedObserver<O> {
    /// Fails with [`ErrorKind::GroundingLimit`] if either count, including this
    /// callback's own contribution, is over its limit. Called after this
    /// callback's own contribution is added, so the callback that pushes a
    /// count over the limit is refused at once, whether or not clingo ever
    /// calls another one after it: a violation on the last callback of a
    /// grounding step used to reach `ground_with_limit`'s own return as a
    /// silent `Ok`, since nothing afterward in that same call would ever check
    /// again, and surfaced only as a poisoning error on the next, unrelated
    /// call instead.
    fn check(&self) -> Result<()> {
        let over_atoms = self
            .limit
            .max_atoms
            .is_some_and(|max| u64::try_from(self.atoms.len()).unwrap_or(u64::MAX) > max);
        let over_rules = self.limit.max_rules.is_some_and(|max| self.rules > max);
        if over_atoms || over_rules {
            return Err(Error::new(
                ErrorKind::GroundingLimit,
                "the grounding-size guard's limit was exceeded",
            ));
        }
        Ok(())
    }

    fn add_atom(&mut self, atom: Atom) {
        self.atoms.insert(atom_id(atom));
    }

    fn add_atoms<'a>(&mut self, atoms: impl IntoIterator<Item = &'a Atom>) {
        for &atom in atoms {
            self.add_atom(atom);
        }
    }

    /// A rule body or condition's atoms, read from the sign-carrying
    /// literals (a program literal's magnitude is the atom's own id, the
    /// same fact [`Atom::pos`](crate::backend::Atom::pos) relies on).
    fn add_literals(&mut self, literals: &[crate::ProgramLiteral]) {
        for literal in literals {
            self.atoms.insert(literal.get().unsigned_abs());
        }
    }

    fn add_rule(&mut self) {
        self.rules += 1;
    }
}

/// An atom's raw id, the same way [`crate::ProgramLiteral::get`]'s magnitude
/// already gives one (`Atom::pos`'s own reasoning): read through
/// [`Atom::pos`](crate::backend::Atom::pos), whose sign carries no
/// information for a plain atom id and is dropped with `unsigned_abs`, since
/// `Atom` has no public raw accessor (DESIGN S17), matching the treatment
/// `conformance_libclingo.rs`'s observer tests already give this exact
/// problem.
fn atom_id(atom: Atom) -> u32 {
    atom.pos().get().unsigned_abs()
}

impl<O: GroundProgramObserver> GroundProgramObserver for LimitedObserver<O> {
    fn init_program(&mut self, incremental: bool) -> Result<()> {
        self.check()?;
        self.observer.init_program(incremental)
    }

    fn begin_step(&mut self) -> Result<()> {
        self.check()?;
        self.observer.begin_step()
    }

    fn end_step(&mut self) -> Result<()> {
        self.check()?;
        self.observer.end_step()
    }

    fn rule(&mut self, choice: bool, head: &[Atom], body: &[crate::ProgramLiteral]) -> Result<()> {
        self.add_atoms(head);
        self.add_literals(body);
        self.add_rule();
        self.check()?;
        self.observer.rule(choice, head, body)
    }

    fn weight_rule(
        &mut self,
        choice: bool,
        head: &[Atom],
        lower_bound: i32,
        body: &[(crate::ProgramLiteral, i32)],
    ) -> Result<()> {
        self.add_atoms(head);
        for &(literal, _) in body {
            self.atoms.insert(literal.get().unsigned_abs());
        }
        self.add_rule();
        self.check()?;
        self.observer.weight_rule(choice, head, lower_bound, body)
    }

    fn minimize(&mut self, priority: i32, literals: &[(crate::ProgramLiteral, i32)]) -> Result<()> {
        for &(literal, _) in literals {
            self.atoms.insert(literal.get().unsigned_abs());
        }
        self.check()?;
        self.observer.minimize(priority, literals)
    }

    fn project(&mut self, atoms: &[Atom]) -> Result<()> {
        self.add_atoms(atoms);
        self.check()?;
        self.observer.project(atoms)
    }

    fn output_atom(&mut self, symbol: Symbol, atom: Option<Atom>) -> Result<()> {
        if let Some(atom) = atom {
            self.add_atom(atom);
        }
        self.check()?;
        self.observer.output_atom(symbol, atom)
    }

    fn output_term(&mut self, symbol: Symbol, condition: &[crate::ProgramLiteral]) -> Result<()> {
        self.add_literals(condition);
        self.check()?;
        self.observer.output_term(symbol, condition)
    }

    fn external(&mut self, atom: Atom, kind: ExternalKind) -> Result<()> {
        self.add_atom(atom);
        self.check()?;
        self.observer.external(atom, kind)
    }

    fn assume(&mut self, literals: &[crate::ProgramLiteral]) -> Result<()> {
        self.add_literals(literals);
        self.check()?;
        self.observer.assume(literals)
    }

    fn heuristic(
        &mut self,
        atom: Atom,
        kind: HeuristicKind,
        bias: i32,
        priority: u32,
        condition: &[crate::ProgramLiteral],
    ) -> Result<()> {
        self.add_atom(atom);
        self.add_literals(condition);
        self.check()?;
        self.observer
            .heuristic(atom, kind, bias, priority, condition)
    }

    fn acyc_edge(&mut self, u: i32, v: i32, condition: &[crate::ProgramLiteral]) -> Result<()> {
        self.add_literals(condition);
        self.check()?;
        self.observer.acyc_edge(u, v, condition)
    }

    fn theory_term_number(&mut self, term: Id, number: i32) -> Result<()> {
        self.check()?;
        self.observer.theory_term_number(term, number)
    }

    fn theory_term_string(&mut self, term: Id, name: &str) -> Result<()> {
        self.check()?;
        self.observer.theory_term_string(term, name)
    }

    fn theory_term_compound(
        &mut self,
        term: Id,
        kind: TheoryCompoundKind,
        arguments: &[Id],
    ) -> Result<()> {
        self.check()?;
        self.observer.theory_term_compound(term, kind, arguments)
    }

    fn theory_element(
        &mut self,
        element: Id,
        terms: &[Id],
        condition: &[crate::ProgramLiteral],
    ) -> Result<()> {
        self.add_literals(condition);
        self.check()?;
        self.observer.theory_element(element, terms, condition)
    }

    fn theory_atom(&mut self, atom: Option<Atom>, term: Id, elements: &[Id]) -> Result<()> {
        if let Some(atom) = atom {
            self.add_atom(atom);
        }
        self.check()?;
        self.observer.theory_atom(atom, term, elements)
    }

    fn theory_atom_with_guard(
        &mut self,
        atom: Option<Atom>,
        term: Id,
        elements: &[Id],
        operator: &str,
        right_hand_side: Id,
    ) -> Result<()> {
        if let Some(atom) = atom {
            self.add_atom(atom);
        }
        self.check()?;
        self.observer
            .theory_atom_with_guard(atom, term, elements, operator, right_hand_side)
    }
}

/// An observer that does nothing, used by [`Control::ground_with_limit`] so
/// it needs no observer of the caller's own.
struct NoopObserver;
impl GroundProgramObserver for NoopObserver {}

impl ScopedControl<'_> {
    /// Registers a ground program observer (`clingo_control_register_observer`,
    /// clingo.h:3326-3335). `replace` matches the header's own wording, "just
    /// pass the grounding to the observer but not the solver": with `false`,
    /// the program still reaches the solver as usual; with `true`, the
    /// observer sees every call exactly as before, but nothing reaches the
    /// solver, so [`Control::solve`] afterward reports neither satisfiable
    /// nor unsatisfiable.
    ///
    /// Once registered, an observer cannot be unregistered: clingo has no
    /// such operation, only registration, which composes (a later
    /// registration adds another observer rather than replacing this one,
    /// unless its own `replace` is `true`). The observer stays registered,
    /// and keeps seeing every later grounding on this control, until the
    /// control itself is dropped.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    ///   the control.
    ///
    /// # Examples
    ///
    /// See the module documentation.
    pub fn register_observer(
        &mut self,
        observer: impl GroundProgramObserver,
        replace: bool,
    ) -> Result<()> {
        self.core.guarded(
            || "registering an observer".to_owned(),
            |handle| handle.register_observer(observer, replace),
        )
    }

    /// Grounds program parts as [`Control::ground`] does, with `limit`
    /// enforced by an internal [`LimitedObserver`] that wraps no observer of
    /// its own.
    ///
    /// Since clingo has no way to unregister an observer, the guard this
    /// registers stays active for every later grounding on this control too,
    /// not only this call: calling `ground_with_limit` once is equivalent to
    /// `register_observer(LimitedObserver::new(NoOp, limit), false)` followed
    /// by `ground`, and every later plain [`Control::ground`] call on the
    /// same control is still subject to it. Register a
    /// [`LimitedObserver`] directly for more control over when the guard
    /// applies, or over composing it with an observer of one's own.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::GroundingLimit`] if `limit` is exceeded. This poisons
    ///   the control, like any other error that stops grounding partway
    ///   (see [`Control::ground`]);
    /// - otherwise as [`Control::ground`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::observer::GroundingLimit;
    /// use clingox::{Control, ErrorKind, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a. b.")?;
    /// let limit = GroundingLimit::new(None, Some(1));
    /// let err = ctl.ground_with_limit(&[Part::base()], limit).unwrap_err();
    /// assert_eq!(err.kind(), ErrorKind::GroundingLimit);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn ground_with_limit(&mut self, parts: &[Part], limit: GroundingLimit) -> Result<()> {
        self.register_observer(LimitedObserver::new(NoopObserver, limit), false)?;
        self.ground(parts)
    }
}

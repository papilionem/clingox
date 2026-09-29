//! External atoms: assigning, freeing and releasing them between solves.

use crate::atoms::ProgramLiteral;
#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use crate::error::{Error, ErrorKind, Result};
use crate::symbol::Symbol;

/// The truth value assigned to an external atom.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part, Symbol, TruthValue};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("#external e. a :- e.")?;
/// ctl.ground(&[Part::base()])?;
/// ctl.assign_external(Symbol::function("e", &[])?, TruthValue::True)?;
/// # Ok::<(), clingox::Error>(())
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TruthValue {
    /// The external is true.
    True,
    /// The external is false, which is also its value until it is assigned,
    /// unless its declaration gives one (`#external e. [true]`).
    False,
    /// The external is open: models with and without it exist.
    Free,
}

impl ScopedControl<'_> {
    /// Assigns a truth value to an external atom, for this and the following
    /// solve calls.
    ///
    /// The value persists across solve calls and across grounding further
    /// parts, until it is assigned again or released. On a symbol that is not
    /// an external (an ordinary atom, a released external, or a symbol that is
    /// not an atom of the program) it does nothing, as clingo does; use
    /// [`Control::try_assign_external`] to be told.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::Logic`], [`ErrorKind::BadAlloc`] or [`ErrorKind::Unknown`],
    ///   which poison the control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Outcome, Part, Symbol, TruthValue};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#external e. a :- e.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let e = Symbol::function("e", &[])?;
    /// ctl.assign_external(e, TruthValue::True)?;
    /// let Outcome::Sat(model, _) = ctl.solve_first()? else {
    ///     panic!("the program has a model");
    /// };
    /// assert!(model.contains(e));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn assign_external(&mut self, atom: Symbol, value: TruthValue) -> Result<()> {
        self.core.guarded(
            || format!("assigning a truth value to `{atom}`"),
            |handle| match handle.atom_literal(atom.raw())? {
                Some(literal) => handle.assign_external(literal, value),
                None => Ok(()),
            },
        )
    }

    /// Assigns a truth value to an external atom, as
    /// [`Control::assign_external`], but fails if `atom` is not an external of
    /// the current grounding.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Runtime`] if `atom` is not an external of the current
    ///   grounding; the message names it, and the control is not poisoned;
    /// - otherwise as [`Control::assign_external`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, ErrorKind, Part, Symbol, TruthValue};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#external e. fact.")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.try_assign_external(Symbol::function("e", &[])?, TruthValue::Free)?;
    /// let fact = Symbol::function("fact", &[])?;
    /// let err = ctl.try_assign_external(fact, TruthValue::True).unwrap_err();
    /// assert_eq!(err.kind(), ErrorKind::Runtime);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn try_assign_external(&mut self, atom: Symbol, value: TruthValue) -> Result<()> {
        self.core.guarded(
            || format!("assigning a truth value to `{atom}`"),
            |handle| match handle.external_literal(atom.raw())? {
                Some(literal) => handle.assign_external(literal, value),
                // Runtime, not one of the kinds that poison: whether a symbol is
                // an external is a fact about the program, and the control is
                // unharmed.
                None => Err(Error::new(
                    ErrorKind::Runtime,
                    format!("`{atom}` is not an external atom of the current grounding"),
                )),
            },
        )
    }

    /// Releases an external atom: it becomes permanently false and is no
    /// longer an external.
    ///
    /// On a symbol that is not an external it does nothing, as clingo does.
    ///
    /// # Errors
    ///
    /// As [`Control::assign_external`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, Symbol};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#external e.")?;
    /// ctl.ground(&[Part::base()])?;
    /// ctl.release_external(Symbol::function("e", &[])?)?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn release_external(&mut self, atom: Symbol) -> Result<()> {
        self.core.guarded(
            || format!("releasing `{atom}`"),
            |handle| match handle.atom_literal(atom.raw())? {
                Some(literal) => handle.release_external(literal),
                None => Ok(()),
            },
        )
    }

    /// Assigns a truth value to the external atom named by a program literal,
    /// as [`Control::assign_external`], but by literal instead of by symbol.
    ///
    /// This is the only way to assign a literal that names no symbol, such as
    /// an auxiliary atom the backend introduced. `literal` must be positive:
    /// clingo documents a negative literal here as exactly equivalent to the
    /// same positive literal with the opposite [`TruthValue`]
    /// (`clingo.h:3112-3114`), and exposing both at once would let a caller
    /// combine them and get a double negation wrong, so clingox rejects a
    /// negative literal itself, before calling clingo.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidInput`] if `literal` is negative; it does not
    ///   poison the control, since clingox rejects it before calling clingo;
    /// - otherwise as [`Control::assign_external`].
    ///
    /// A literal far beyond the program's atoms, even a valid one, makes clasp
    /// allocate space for every atom up to it, which can take gigabytes
    /// (`docs/dev/UPSTREAM-ISSUES.md` U22). Pass only literals read from this
    /// control.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, Symbol, TruthValue};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#external e. a :- e.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let e = Symbol::function("e", &[])?;
    /// let literal = ctl.symbolic_atoms()?.find(e)?.expect("e is an atom").literal();
    /// ctl.assign_external_literal(literal, TruthValue::True)?;
    /// assert!(ctl.solve(&[]).unwrap().is_sat());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn assign_external_literal(
        &mut self,
        literal: ProgramLiteral,
        value: TruthValue,
    ) -> Result<()> {
        if !literal.is_positive() {
            // Checked before clingo sees anything, so it does not poison.
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "assigning a truth value to program literal {}: the literal must be \
                     positive; pass the positive literal with the opposite truth value instead",
                    literal.get()
                ),
            ));
        }
        self.core.guarded(
            || {
                format!(
                    "assigning a truth value to program literal {}",
                    literal.get()
                )
            },
            |handle| handle.assign_external(literal.get(), value),
        )
    }

    /// Releases the external atom named by a program literal, as
    /// [`Control::release_external`], but by literal instead of by symbol.
    ///
    /// Unlike [`Control::assign_external_literal`], a negative literal is
    /// accepted: clingo documents it as releasing the same atom
    /// (`clingo.h:3124-3126`), with no truth value to double-negate.
    ///
    /// # Errors
    ///
    /// As [`Control::release_external`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, Symbol, TruthValue};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#external e.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let e = Symbol::function("e", &[])?;
    /// let literal = ctl.symbolic_atoms()?.find(e)?.expect("e is an atom").literal();
    /// ctl.assign_external_literal(literal, TruthValue::True)?;
    /// ctl.release_external_literal(literal)?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn release_external_literal(&mut self, literal: ProgramLiteral) -> Result<()> {
        self.core.guarded(
            || format!("releasing program literal {}", literal.get()),
            |handle| handle.release_external(literal.get()),
        )
    }
}

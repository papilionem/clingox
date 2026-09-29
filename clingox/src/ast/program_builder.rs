//! The program builder: adds [`Ast`] statements to a control
//! (`clingo_program_builder_*`).

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::fmt;

use super::Ast;
use crate::control::ControlCore;
use crate::error::{Error, Result};
use crate::raw;

impl ScopedControl<'_> {
    /// Adds statements to the program as syntax trees, through a
    /// [`ProgramBuilder`] that lives for the closure `f`.
    ///
    /// This is the tree counterpart of [`Control::add`]: parse or build
    /// statements with [`crate::ast`], then hand them to
    /// [`ProgramBuilder::add`]. The session begins before `f` runs and ends
    /// after it, whatever `f` returned.
    ///
    /// Behavior worth knowing (all measured against clingo 5.8.2):
    ///
    /// - statements added before any `#program` statement go to `base`, and
    ///   the current program part persists across sessions: a `#program p(k).`
    ///   statement added in one session applies to the statements of the next
    ///   until another `#program` replaces it;
    /// - statements can be added after a [`Control::ground`];
    /// - **statements added before an error or a panic are kept**. When `f`
    ///   returns an error, the session still ends and the earlier statements
    ///   are part of the program. When `f` panics, the panic unwinds through
    ///   this call, and the next call on the control ends the session first,
    ///   so a later `ground` or `solve` sees the statements too; nothing is
    ///   lost silently;
    /// - a statement with a semantic error, such as a second `#const` of the
    ///   same name, is accepted here: clingo logs the message (to the
    ///   control's logger) and reports the failure at [`Control::ground`].
    ///   Once a control has logged an error, it refuses the next
    ///   `with_program_builder` and the next [`Control::add`] with
    ///   [`ErrorKind::Parse`], "parsing failed";
    /// - [`ProgramBuilder::add`] copies the tree: editing the node afterwards
    ///   does not change the program.
    ///
    /// **Known limitation.** Grounding an `#external` statement whose type
    /// term is `|X|`, `X\2` or `X**2` over a variable crashes clingo 5.8.2,
    /// whether the statement comes from text or from a tree
    /// (`docs/dev/UPSTREAM-ISSUES.md`).
    ///
    /// # Errors
    ///
    /// - whatever `f` returns, after the session has ended, poisoning by its
    ///   kind as everywhere else (DESIGN S3); it takes priority over a
    ///   failure to end the session;
    /// - [`ErrorKind::Parse`] if the control has logged an error already, so
    ///   clingo refuses to begin ("parsing failed"); `f` is not called, and
    ///   the control is poisoned;
    /// - an error from ending the session, if `f` succeeded; it poisons;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control.
    ///
    /// A panic in `f` unwinds through this call. A panic in the control's
    /// logger while [`ProgramBuilder::add`] runs resumes when that `add`
    /// returns, unwinding through `f` like a panic of `f` itself; the session
    /// is closed by the next entry point of the control. Only a logger panic
    /// while the session is being closed resumes after the session has ended.
    ///
    /// [`ErrorKind::Parse`]: crate::ErrorKind::Parse
    /// [`ErrorKind::Poisoned`]: crate::ErrorKind::Poisoned
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{self, AstType};
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.with_program_builder(|builder| {
    ///     ast::parse_string("a. b :- a.", |statement| {
    ///         if statement.ast_type() == AstType::Rule {
    ///             builder.add(&statement)?;
    ///         }
    ///         Ok(())
    ///     })
    /// })?;
    /// ctl.ground(&[Part::base()])?;
    /// let (_, models) = ctl.solve_all()?;
    /// clingox::testing::assert_models!(models, ["a b"]);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    ///
    /// The statements added before the closure fails are kept:
    ///
    /// ```
    /// use clingox::ast::{self, AstType};
    /// use clingox::{Control, Error, ErrorKind, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// let result: Result<(), Error> = ctl.with_program_builder(|builder| {
    ///     ast::parse_string("a.", |statement| {
    ///         if statement.ast_type() == AstType::Rule {
    ///             builder.add(&statement)?;
    ///         }
    ///         Ok(())
    ///     })?;
    ///     Err(Error::new(ErrorKind::InvalidInput, "changed my mind"))
    /// });
    /// assert!(result.is_err());
    /// ctl.ground(&[Part::base()])?;
    /// assert!(ctl.solve(&[])?.is_sat());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn with_program_builder<R>(
        &mut self,
        f: impl FnOnce(&mut ProgramBuilder<'_>) -> Result<R>,
    ) -> Result<R> {
        self.core.refusal()?;
        self.core.finish_search()?;
        self.core
            .handle
            .open_program_builder()
            .map_err(|err| self.core.note(err.context("opening the program builder")))?;
        let result = f(&mut ProgramBuilder {
            control: &mut self.core,
        });
        let closed = self
            .core
            .handle
            .close_program_builder()
            .map_err(|err| err.context("closing the program builder"));
        self.core.handle.resume_logger_panic();
        raw::settle(result, closed).map_err(|err| self.core.note(err))
    }
}

/// A handle to add non-ground statements to the program, from
/// [`Control::with_program_builder`].
///
/// It borrows the control mutably (DESIGN S5) for exactly the duration of the
/// closure `with_program_builder` runs it in, so the control cannot be
/// grounded, solved or added to from inside the closure, and the builder
/// cannot outlive it.
pub struct ProgramBuilder<'c> {
    control: &'c mut ControlCore,
}

impl ProgramBuilder<'_> {
    /// Runs one call on the open program builder, naming `context` in any
    /// error.
    fn call<T>(
        &self,
        context: &str,
        f: impl FnOnce(&raw::ControlHandle) -> Result<T, Error>,
    ) -> Result<T> {
        let control = &*self.control;
        let raw_result = control.refusal().and_then(|()| f(&control.handle));
        // A `#script` statement runs its `execute` during the call, and its
        // failure replaces clingo's report of it.
        control.settle_scripts(raw_result, || context.to_owned())
    }

    /// Adds one statement to the program.
    ///
    /// The tree is copied: it stays valid, and editing it afterwards does not
    /// change what was added. Adding the same node twice adds the statement
    /// twice.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Parse`] if `statement` is not a statement, or has a
    ///   child of the wrong kind (a function term where a literal belongs,
    ///   say), with no messages. The refusal is atomic and poisons the
    ///   control, so later calls on the control, including further `add`s in
    ///   the same closure, return [`ErrorKind::Poisoned`];
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory, which poisons
    ///   the control;
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control.
    ///
    /// A statement with a semantic error is accepted; see
    /// [`Control::with_program_builder`].
    ///
    /// **Known limitation.** clingo does not check every kind combination on
    /// the way in: grounding an `#external` statement whose type term is
    /// `|X|`, `X\2` or `X**2` over a variable crashes clingo 5.8.2, from text
    /// and from trees alike (`docs/dev/UPSTREAM-ISSUES.md`).
    ///
    /// [`ErrorKind::Parse`]: crate::ErrorKind::Parse
    /// [`ErrorKind::BadAlloc`]: crate::ErrorKind::BadAlloc
    /// [`ErrorKind::Poisoned`]: crate::ErrorKind::Poisoned
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{self, AstType, Attribute};
    /// use clingox::{Control, ErrorKind};
    ///
    /// let mut ctl = Control::new()?;
    /// let err = ctl
    ///     .with_program_builder(|builder| {
    ///         let mut term = None;
    ///         ast::parse_string("p(1).", |statement| {
    ///             if statement.ast_type() == AstType::Rule {
    ///                 term = Some(statement.ast(Attribute::Head)?);
    ///             }
    ///             Ok(())
    ///         })?;
    ///         // A literal is not a statement.
    ///         builder.add(&term.unwrap())
    ///     })
    ///     .unwrap_err();
    /// assert_eq!(err.kind(), ErrorKind::Parse);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add(&mut self, statement: &Ast) -> Result<()> {
        self.call("adding a statement", |handle| {
            handle.program_builder_add(statement.as_raw())
        })
    }
}

impl fmt::Debug for ProgramBuilder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProgramBuilder").finish_non_exhaustive()
    }
}

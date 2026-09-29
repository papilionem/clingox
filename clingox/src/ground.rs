//! External functions called during grounding: [`Control::ground_with`] and
//! [`FunctionCall`] (DESIGN S8, S10).

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::borrow::Cow;
use std::ffi::CStr;
use std::fmt;

use crate::control::Part;
use crate::error::Result;
use crate::raw::GroundError;
use crate::symbol::Symbol;

/// One evaluation of an external function `@name(args)` during grounding.
///
/// [`Control::ground_with`] lends it to its closure, which reads the name and
/// the evaluated arguments and pushes the function's values. It borrows
/// clingo's data for that one call, so it cannot leave the closure.
///
/// # Examples
///
/// ```
/// use clingox::prelude::*;
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("p(@inc(1)).")?;
/// ctl.ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
///     assert_eq!(call.name(), "inc");
///     let n = call.args()[0].as_number().unwrap_or(0);
///     call.push(Symbol::number(n + 1))
/// })?;
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct FunctionCall<'a> {
    name: Cow<'a, str>,
    args: &'a [Symbol],
    values: Vec<Symbol>,
}

impl<'a> FunctionCall<'a> {
    /// A call that collects its values in `values`, which must be empty: a
    /// ground callback runs many times, and the buffer is kept between calls.
    pub(crate) fn new(name: Cow<'a, str>, args: &'a [Symbol], values: Vec<Symbol>) -> Self {
        debug_assert!(values.is_empty());
        FunctionCall { name, args, values }
    }

    /// The values pushed so far, in order.
    pub(crate) fn into_values(self) -> Vec<Symbol> {
        self.values
    }
}

impl FunctionCall<'_> {
    /// The function's name, without the `@`.
    ///
    /// Invalid UTF-8 is replaced with U+FFFD, as in [`Symbol::name`].
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The evaluated arguments.
    pub fn args(&self) -> &[Symbol] {
        self.args
    }

    /// Adds one value of the function.
    ///
    /// The term `@f(..)` stands for every value pushed: several values give
    /// several ground instances of the rule, and none drops the rule instance,
    /// as clingo does for an empty result.
    ///
    /// # Errors
    ///
    /// None today: the values are handed to clingo when the closure returns.
    /// The `Result` leaves room for checks clingo may add.
    pub fn push(&mut self, symbol: Symbol) -> Result<()> {
        self.values.push(symbol);
        Ok(())
    }
}

impl fmt::Debug for FunctionCall<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FunctionCall")
            .field("name", &self.name)
            .field("args", &self.args)
            .field("values", &self.values)
            .finish()
    }
}

impl ScopedControl<'_> {
    /// Grounds program parts as [`Control::ground`] does, and calls `f` for
    /// every evaluation of an external function `@name(args)`.
    ///
    /// clingo does not cache the calls: the same term in two rules is evaluated
    /// twice. `f` runs on the calling thread, during this call (DESIGN S10), so
    /// it may borrow locals and need not be `Send`. Grounding cannot be
    /// interrupted: an [`InterruptHandle`](crate::InterruptHandle) used
    /// meanwhile returns `false` and has no effect.
    ///
    /// Without a callback, [`Control::ground`] reports an `@f(..)` term as an
    /// [`OperationUndefined`](crate::MessageCode::OperationUndefined) message
    /// and drops the rule instance, unless a registered [script](crate::script)
    /// that has run a block says `f` is callable.
    ///
    /// **This callback wins over every script.** It is asked first for every
    /// `@` term, and no script is consulted for that call whatever the callback
    /// does, also for a name it does not know or when it pushes nothing. Use
    /// [`Control::ground`] to reach a script, or call your script from the
    /// callback yourself.
    ///
    /// # Errors
    ///
    /// - the error `f` returned, with its kind unchanged: grounding stops, and
    ///   the message names the part being grounded. This poisons the control,
    ///   whatever its kind (unlike DESIGN S3's "callback errors are
    ///   recoverable" default): clingo keeps whatever it already
    ///   ground before `f` failed and would answer from it silently, which is
    ///   never safe to hand back. Recovery means building a new [`Control`];
    /// - otherwise as [`Control::ground`], including a registered observer's
    ///   own error, which poisons the same way.
    ///
    /// A panic in `f`, or in a registered observer, stops grounding, poisons
    /// the control for the same reason, and resumes on the calling thread once
    /// this call returns.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::prelude::*;
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("p(@seq(3)).")?;
    /// ctl.ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
    ///     let n = call.args()[0].as_number().unwrap_or(0);
    ///     for i in 1..=n {
    ///         call.push(Symbol::number(i))?;
    ///     }
    ///     Ok(())
    /// })?;
    /// let (_, models) = ctl.solve_all()?;
    /// assert_eq!(models[0].symbols().len(), 3);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn ground_with<F>(&mut self, parts: &[Part], mut f: F) -> Result<()>
    where
        F: FnMut(&mut FunctionCall<'_>) -> Result<()>,
    {
        let context = || grounding(parts);
        let parts = raw_parts(parts);
        self.core
            .refusal()
            .and_then(|()| self.core.finish_search())
            .map_err(|err| self.core.note(err.context(context())))?;
        match self.core.handle.ground_with(&parts, &mut f) {
            Ok(()) => Ok(()),
            // The user's error keeps its own kind, but it is poisoned
            // unconditionally: clingo's own tests keep using a control after
            // a failed callback, but clingo also keeps the truncated program
            // it already ground and answers from it silently, which is not
            // safe to expose.
            Err(GroundError::Callback(err)) => {
                Err(self.core.note(err.context(context()).poisoning()))
            }
            Err(GroundError::Clingo(err)) => Err(self.core.note(err.context(context()))),
            Err(GroundError::Panic(payload)) => {
                self.core.poison_after_panic(&context());
                std::panic::resume_unwind(payload)
            }
        }
    }
}

/// The context of a grounding error: the parts being grounded.
pub(crate) fn grounding(parts: &[Part]) -> String {
    let names: Vec<String> = parts.iter().map(|p| format!("`{p}`")).collect();
    format!("grounding {}", names.join(", "))
}

/// The parts as clingo takes them: a name and its parameter values.
pub(crate) fn raw_parts(parts: &[Part]) -> Vec<(&CStr, &[Symbol])> {
    parts.iter().map(Part::as_raw).collect()
}

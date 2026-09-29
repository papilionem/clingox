//! Adding typed facts: [`Control::add_facts`].

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::fmt::Write as _;

use crate::control::{FACTS_PREFIX, Part};
use crate::convert::ToSymbol;
use crate::error::{Error, ErrorKind, Result};
use crate::symbol::{Symbol, SymbolKind};

impl ScopedControl<'_> {
    /// Adds facts, converted with [`ToSymbol`], and grounds them at once.
    ///
    /// Underneath, the facts become program text, one `symbol.` per fact, in a
    /// program part of their own, named `__clingox_facts_<n>` where `n` counts
    /// the calls on this control. That part alone is grounded at once.
    /// `add_facts` never adds to or grounds `base`, so no part is grounded
    /// twice in multi-shot solving.
    ///
    /// **Reserved names.** Part names that start with `__clingox_facts_` belong
    /// to `add_facts`: [`Control::add`] and [`Part::new`] refuse them with
    /// [`ErrorKind::InvalidInput`]. Do not use the prefix in a `#program`
    /// directive either.
    ///
    /// **Grounding order.** Rules grounded after the facts see them. Rules
    /// grounded before the facts were added do not: clingo never re-grounds an
    /// earlier part, so `q(X) :- p(X).` grounded first derives nothing from a
    /// later fact `p(1)`. Add the facts, then ground the parts whose rules use
    /// them.
    ///
    /// A search left open by a forgotten [`SolveHandle`](crate::SolveHandle) is
    /// closed first, as by every entry point. Then every item is converted and
    /// checked before anything is added, so a failed call adds nothing. An
    /// empty iterator adds and grounds nothing. Adding a fact that already
    /// exists is harmless.
    ///
    /// # Errors
    ///
    /// - The first error of a [`to_symbol`](ToSymbol::to_symbol), unchanged.
    ///   Nothing is added.
    /// - [`ErrorKind::Conversion`] for a symbol that clingo would not read back
    ///   as the same atom: one that is not a function with a name (a number, a
    ///   string, `#sup`, `#inf` or a tuple), or that contains a name that is
    ///   not a clingo identifier, `_*[a-z][A-Za-z0-9_']*` other than `not`
    ///   (`Foo` would read as a variable). Nothing is added, and the control is
    ///   not poisoned.
    /// - [`ErrorKind::Logic`] for a fact about an atom an earlier step defined,
    ///   such as the head of a choice rule `{p(1)}.` grounded and solved before
    ///   (clingo's "redefinition of atom"). It poisons the control.
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control.
    /// - The other errors of [`Control::add`] and [`Control::ground`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Outcome, Part, ToSymbol};
    ///
    /// #[derive(ToSymbol)]
    /// struct Edge(i32, i32);
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("reach(Y) :- edge(1, Y). reach(Z) :- reach(Y), edge(Y, Z).")?;
    /// ctl.add_facts([Edge(1, 2), Edge(2, 3)])?;
    /// ctl.ground(&[Part::base()])?;
    /// let Outcome::Sat(model, _) = ctl.solve_first()? else {
    ///     panic!("the program has a model");
    /// };
    /// assert!(model.contains("reach(3)".parse()?));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn add_facts<I>(&mut self, facts: I) -> Result<()>
    where
        I: IntoIterator,
        I::Item: ToSymbol,
    {
        let context = "adding facts";
        // Like every entry point, it closes a search a forgotten handle left
        // open (DESIGN S4) before anything else, even when it adds nothing.
        self.core
            .refusal()
            .and_then(|()| self.core.finish_search())
            .map_err(|e| e.context(context))?;
        let symbols = facts
            .into_iter()
            .map(|fact| fact.to_symbol())
            .collect::<Result<Vec<Symbol>>>()?;
        let mut program = String::new();
        for symbol in &symbols {
            check_fact(*symbol).map_err(|e| e.context(context))?;
            writeln!(program, "{symbol}.").map_err(|_| {
                Error::new(ErrorKind::BadAlloc, "clingo could not print a symbol").context(context)
            })?;
        }
        if symbols.is_empty() {
            return Ok(());
        }
        let name = format!("{FACTS_PREFIX}{}", self.core.fact_parts);
        self.core.fact_parts += 1;
        let part = Part::reserved(&name, &[])?;
        self.add_block(&name, &[], &program)
            .and_then(|()| self.ground(&[part]))
            .map_err(|e| e.context(context))
    }
}

/// Checks that clingo reads `symbol.` back as the atom `symbol`.
fn check_fact(symbol: Symbol) -> Result<()> {
    match symbol.kind() {
        SymbolKind::Function {
            name, arguments, ..
        } if !name.is_empty() => {
            check_name(name, symbol)?;
            arguments
                .iter()
                .try_for_each(|a| check_argument(*a, symbol))
        }
        _ => Err(Error::new(
            ErrorKind::Conversion,
            format!(
                "`{symbol}` is not an atom: a fact is a constant or a function, with or \
                 without classical negation"
            ),
        )),
    }
}

/// Checks the names inside an argument of `fact`. Numbers, strings, `#sup`,
/// `#inf` and tuples read back as themselves.
fn check_argument(argument: Symbol, fact: Symbol) -> Result<()> {
    match argument.kind() {
        SymbolKind::Function {
            name, arguments, ..
        } => {
            if !name.is_empty() {
                check_name(name, fact)?;
            }
            arguments.iter().try_for_each(|a| check_argument(*a, fact))
        }
        _ => Ok(()),
    }
}

fn check_name(name: &str, fact: Symbol) -> Result<()> {
    if is_identifier(name) {
        return Ok(());
    }
    Err(Error::new(
        ErrorKind::Conversion,
        format!(
            "`{fact}` cannot be added as a fact: `{name}` is not a clingo name \
             (`_*[a-z][A-Za-z0-9_']*`, other than `not`), so clingo would not read it back"
        ),
    ))
}

/// Whether clingo's lexer reads `name` as an identifier:
/// `_*[a-z][A-Za-z0-9_']*`, other than the keyword `not`.
pub(crate) fn is_identifier(name: &str) -> bool {
    let rest = name.trim_start_matches('_');
    let mut chars = rest.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '\'')
        && name != "not"
}

#[cfg(test)]
mod tests {
    use super::is_identifier;
    use crate::{Control, Part, Symbol};

    fn fact(n: i32) -> Symbol {
        Symbol::function("p", &[Symbol::number(n)]).unwrap()
    }

    /// Grounding one part twice changes nothing clingo
    /// reports, so the part names are checked here, through the counter they
    /// are made from.
    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn each_add_facts_call_uses_a_fresh_part() {
        let mut ctl = Control::new().unwrap();
        assert_eq!(ctl.core.fact_parts, 0);
        ctl.add_facts([fact(1)]).unwrap();
        assert_eq!(
            ctl.core.fact_parts, 1,
            "the first call used `__clingox_facts_0`"
        );
        ctl.add_facts([fact(2)]).unwrap();
        assert_eq!(
            ctl.core.fact_parts, 2,
            "the second call used `__clingox_facts_1`"
        );
        // Each part holds only its own facts: the first part is already
        // grounded, and grounding the second again adds nothing new.
        let second = Part::reserved("__clingox_facts_1", &[]).unwrap();
        ctl.ground(&[second]).unwrap();
        let (_, models) = ctl.solve_all().unwrap();
        assert_eq!(models.len(), 1);
        let texts: Vec<String> = models[0]
            .symbols()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(texts, ["p(1)", "p(2)"]);
    }

    #[test]
    fn identifiers_are_read_as_clingo_lexes_them() {
        for name in ["p", "_p", "__p", "x'", "pQ_9'", "default", "true", "a1"] {
            assert!(is_identifier(name), "{name}");
        }
        for name in ["", "_", "__", "Foo", "_X", "not", "1a", "é", "a b", "a-b"] {
            assert!(!is_identifier(name), "{name}");
        }
    }
}

//! Helpers for testing logic programs: [`parse_answer`] and
//! [`assert_models!`](crate::testing::assert_models).
//!
//! # Examples
//!
//! ```
//! use clingox::testing::assert_models;
//! use clingox::{Control, Part};
//!
//! let mut ctl = Control::new()?;
//! ctl.add_base("a :- not b. b :- not a.")?;
//! ctl.ground(&[Part::base()])?;
//! let (_, models) = ctl.solve_all()?;
//! assert_models!(models, ["a", "b"]);
//! # Ok::<(), clingox::Error>(())
//! ```

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::error::{Error, ErrorKind, Result};
use crate::model::OwnedModel;
use crate::symbol::Symbol;

// `#[macro_export]` puts every exported macro at the crate root. The macro
// is exported under a hidden name there, and this is its only public path
// .
#[doc(inline)]
pub use crate::__clingox_assert_models as assert_models;

/// Splits a line of clingo's answer-set output into symbols.
///
/// Terms are separated by whitespace outside string literals and
/// parentheses, so `p(1, 2) s("a b")` is two terms. Each term is parsed with
/// [`Symbol::from_str`](std::str::FromStr), which uses clingo's own parser.
/// The symbols come in the order of the line, without sorting or removing
/// duplicates. An empty or blank line gives an empty vector.
///
/// # Errors
///
/// [`ErrorKind::Parse`] for a term clingo cannot parse, and for an unbalanced
/// parenthesis or quote. [`ErrorKind::Nul`] for a NUL byte.
///
/// # Examples
///
/// ```
/// use clingox::Symbol;
/// use clingox::testing::parse_answer;
///
/// let symbols = parse_answer(r#"p(1, 2) s("a b") -q"#)?;
/// assert_eq!(symbols.len(), 3);
/// assert_eq!(symbols[1], Symbol::function("s", &[Symbol::string("a b")?])?);
/// # Ok::<(), clingox::Error>(())
/// ```
pub fn parse_answer(line: &str) -> Result<Vec<Symbol>> {
    split_terms(line)?
        .into_iter()
        .map(|term| {
            term.parse::<Symbol>().map_err(|e| match e.kind() {
                ErrorKind::Nul => e,
                _ => e.with_kind(ErrorKind::Parse),
            })
        })
        .collect()
}

/// The terms of an answer line, split at whitespace outside strings and
/// parentheses.
fn split_terms(line: &str) -> Result<Vec<&str>> {
    let unbalanced =
        |what: &str| Error::new(ErrorKind::Parse, format!("{what} in answer line {line:?}"));
    let mut terms = Vec::new();
    let mut start = None;
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, c) in line.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            c if c.is_whitespace() && depth == 0 => {
                if let Some(from) = start.take() {
                    terms.push(&line[from..index]);
                }
                continue;
            }
            '"' => in_string = true,
            '(' => depth += 1,
            ')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| unbalanced("an unmatched `)`"))?;
            }
            _ => {}
        }
        start.get_or_insert(index);
    }
    if in_string {
        return Err(unbalanced("an unterminated string"));
    }
    if depth > 0 {
        return Err(unbalanced("an unclosed `(`"));
    }
    if let Some(from) = start {
        terms.push(&line[from..]);
    }
    Ok(terms)
}

/// Asserts that `models` are exactly the models the answer lines describe.
///
/// `models` is borrowed, and must be a slice of [`OwnedModel`]s or something
/// that gives one: the `Vec` that [`Control::solve_all`] returns, an array or
/// a slice. Each line is a `&str` in clingo's answer format, parsed with
/// [`parse_answer`], and describes one model's shown symbols
/// ([`OwnedModel::symbols`]). `[]` expects no model.
///
/// The comparison ignores the order of the models and of the symbols in a
/// line, but counts models: `["a", "a"]` does not match a single model `a`.
///
/// [`Control::solve_all`]: crate::Control::solve_all
/// [`OwnedModel`]: crate::OwnedModel
/// [`OwnedModel::symbols`]: crate::OwnedModel::symbols
/// [`parse_answer`]: crate::testing::parse_answer
///
/// # Panics
///
/// On a mismatch, with a message that starts with `models differ (expected
/// <e>, found <f>)` and lists each `missing: <model>` and each
/// `unexpected: <model>`, the symbols of each sorted, `(empty)` for an empty
/// model. On a line that does not parse, with a message containing the line.
/// The panic reports the line of the macro call.
///
/// # Examples
///
/// ```
/// use clingox::testing::assert_models;
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("{a; b}. :- a, b.")?;
/// ctl.ground(&[Part::base()])?;
/// let (_, models) = ctl.solve_all()?;
/// assert_models!(models, ["", "a", "b"]);
/// # Ok::<(), clingox::Error>(())
/// ```
#[macro_export]
#[doc(hidden)]
macro_rules! __clingox_assert_models {
    ($models:expr, [$($line:expr),* $(,)?] $(,)?) => {
        $crate::testing::__assert_models(&$models, &[$($line),*])
    };
}

/// The implementation of [`assert_models!`](crate::testing::assert_models).
#[doc(hidden)]
#[track_caller]
pub fn __assert_models<M>(models: &M, lines: &[&str])
where
    M: AsRef<[OwnedModel]> + ?Sized,
{
    let found: Vec<Vec<Symbol>> = models
        .as_ref()
        .iter()
        .map(|model| sorted(model.symbols().to_vec()))
        .collect();
    // A loop, not a closure: `#[track_caller]` does not reach into closures,
    // and the panic must name the caller's line.
    let mut expected: Vec<Vec<Symbol>> = Vec::with_capacity(lines.len());
    for line in lines {
        match parse_answer(line) {
            Ok(symbols) => expected.push(sorted(symbols)),
            Err(err) => panic!("assert_models!: the expected model {line:?} does not parse: {err}"),
        }
    }
    if let Some(message) = difference(&expected, &found) {
        panic!("{message}");
    }
}

fn sorted(mut symbols: Vec<Symbol>) -> Vec<Symbol> {
    symbols.sort_unstable();
    symbols
}

/// The mismatch report, or `None` if both lists hold the same models the same
/// number of times.
fn difference(expected: &[Vec<Symbol>], found: &[Vec<Symbol>]) -> Option<String> {
    let mut balance: BTreeMap<&[Symbol], isize> = BTreeMap::new();
    for model in expected {
        *balance.entry(model.as_slice()).or_default() += 1;
    }
    for model in found {
        *balance.entry(model.as_slice()).or_default() -= 1;
    }
    if balance.values().all(|count| *count == 0) {
        return None;
    }
    let mut message = format!(
        "models differ (expected {}, found {})",
        expected.len(),
        found.len()
    );
    // The map iterates in `Symbol` order, so each group comes out sorted.
    for (label, sign) in [("missing", 1), ("unexpected", -1)] {
        for (model, count) in &balance {
            for _ in 0..(count * sign).max(0) {
                // Writing to a `String` cannot fail.
                let _ = write!(message, "\n{label}: {}", show(model));
            }
        }
    }
    Some(message)
}

/// A model's symbols joined by spaces, or `(empty)`.
fn show(model: &[Symbol]) -> String {
    if model.is_empty() {
        return "(empty)".to_owned();
    }
    model
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::split_terms;

    #[test]
    fn terms_split_at_whitespace_outside_strings_and_parentheses() {
        assert_eq!(
            split_terms(r#" a p(1, 2)  s("a b\" c)") t((1, 2),f( g ))"#).unwrap(),
            ["a", "p(1, 2)", r#"s("a b\" c)")"#, "t((1, 2),f( g ))"]
        );
        assert_eq!(split_terms("").unwrap(), Vec::<&str>::new());
        assert_eq!(split_terms(" \t\n").unwrap(), Vec::<&str>::new());
    }

    #[test]
    fn unbalanced_lines_are_errors() {
        for line in ["p(", "p)", "a \"open", "p(1))", "s(\"x\\\")"] {
            assert!(split_terms(line).is_err(), "{line}");
        }
    }
}

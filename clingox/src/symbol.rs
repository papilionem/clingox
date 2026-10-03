//! Symbols: clingo's ground terms.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use crate::error::{Error, Result};
use crate::raw::{self, RawSymbol, SymbolType};

/// A ground term: a number, a string, a function (including constants and
/// tuples), `#inf` or `#sup`.
///
/// A symbol is a small `Copy` handle into clingo's global symbol table. Symbols
/// are never freed, so they, and the names, strings and arguments borrowed from
/// them, live for the rest of the process. The table grows with every distinct
/// symbol created and never shrinks.
///
/// Symbols are ordered as clingo orders them: `#inf`, then numbers, then
/// functions without arguments, then strings, then functions with arguments,
/// then `#sup`. [`Display`](fmt::Display) prints clingo's text form, and
/// [`FromStr`] parses it.
///
/// # Examples
///
/// ```
/// use clingox::Symbol;
///
/// let p = Symbol::function("p", &[Symbol::number(1), Symbol::string("x")?])?;
/// assert_eq!(p.to_string(), r#"p(1,"x")"#);
/// assert_eq!(p.name(), Some("p"));
///
/// let parsed: Symbol = r#"p(1,"x")"#.parse()?;
/// assert_eq!(parsed, p);
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Symbol(RawSymbol);

/// What a [`Symbol`] is, with its parts.
///
/// The parts of a function are borrowed from clingo for the rest of the
/// process, so `arguments` is a `&'static [Symbol]`. To test a symbol against
/// an expected one, compare the [`Symbol`]s themselves; to look inside, compare
/// `arguments` with an array or match it with a slice pattern. A whole
/// `SymbolKind::Function { .. }` built by hand for the comparison needs a
/// `'static` slice, which is why the first way is simpler.
///
/// # Examples
///
/// ```
/// use clingox::{Symbol, SymbolKind};
///
/// let p = Symbol::function("p", &[Symbol::number(1), Symbol::number(2)])?;
///
/// // Compare symbols, not their kinds:
/// assert_eq!(p, "p(1,2)".parse::<Symbol>()?);
///
/// // Look inside:
/// let SymbolKind::Function { name, arguments, .. } = p.kind() else {
///     panic!("p(1,2) is a function");
/// };
/// assert_eq!(name, "p");
/// assert_eq!(arguments, [Symbol::number(1), Symbol::number(2)]);
/// if let [first, _] = arguments {
///     assert_eq!(first.kind(), SymbolKind::Number(1));
/// }
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SymbolKind {
    /// `#inf`, the smallest symbol.
    Infimum,
    /// A number.
    Number(i32),
    /// A string, without quotes or escapes.
    String(&'static str),
    /// A function. A constant has no arguments, and a tuple has an empty name.
    Function {
        /// The name, empty for a tuple.
        name: &'static str,
        /// The arguments.
        arguments: &'static [Symbol],
        /// Whether the function carries classical negation.
        sign: Sign,
    },
    /// `#sup`, the largest symbol.
    Supremum,
    /// A symbol of a type clingo uses only internally, such as its "special"
    /// symbol (type 6 in clingo 5.8), which the public API never creates.
    ///
    /// It is not expected in practice: every symbol clingox can build or read
    /// from a program has one of the kinds above. The variant exists so that
    /// [`Symbol::kind`] never mistakes such a symbol for another kind.
    Other,
}

/// The sign of a function symbol: `-p` is negative, `p` positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sign {
    /// No classical negation.
    Positive,
    /// Classical negation, written `-p`.
    Negative,
}

impl Sign {
    fn from_positive(positive: bool) -> Sign {
        if positive {
            Sign::Positive
        } else {
            Sign::Negative
        }
    }
}

impl Symbol {
    pub(crate) fn raw(self) -> RawSymbol {
        self.0
    }

    /// Wraps a symbol value that clingo returned. Values from anywhere else
    /// could be passed back to clingo as symbols that do not exist.
    pub(crate) fn from_clingo(raw: RawSymbol) -> Symbol {
        Symbol(raw)
    }

    /// The number `n`.
    #[must_use]
    #[inline]
    pub fn number(n: i32) -> Symbol {
        Symbol(raw::create_number(n))
    }

    /// `#sup`, the largest symbol.
    #[must_use]
    #[inline]
    pub fn supremum() -> Symbol {
        Symbol(raw::create_supremum())
    }

    /// `#inf`, the smallest symbol.
    #[must_use]
    #[inline]
    pub fn infimum() -> Symbol {
        Symbol(raw::create_infimum())
    }

    /// A string symbol. `s` is the content, without quotes or escapes.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Nul`](crate::ErrorKind::Nul) if `s` contains a NUL byte;
    /// - [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) if clingo runs out of
    ///   memory;
    /// - [`ErrorKind::Version`](crate::ErrorKind::Version) if the linked clingo is
    ///   not 5.8.1 or newer within 5.8.
    ///
    /// # Examples
    ///
    /// ```
    /// let s = clingox::Symbol::string("hi")?;
    /// assert_eq!(s.as_string(), Some("hi"));
    /// assert_eq!(s.to_string(), "\"hi\"");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn string(s: &str) -> Result<Symbol> {
        raw::create_string(s)
            .map(Symbol)
            .map_err(|e| e.context("creating a string symbol"))
    }

    /// A positive function symbol. With no arguments it is a constant.
    ///
    /// # Errors
    ///
    /// As [`Symbol::string`], for a NUL byte in `name`.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::Symbol;
    ///
    /// let f = Symbol::function("f", &[Symbol::number(1)])?;
    /// assert_eq!(f.to_string(), "f(1)");
    /// let c = Symbol::function("c", &[])?;
    /// assert_eq!(c.to_string(), "c");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn function(name: &str, arguments: &[Symbol]) -> Result<Symbol> {
        Symbol::function_with_sign(name, arguments, Sign::Positive)
    }

    /// A function symbol with the given sign; [`Sign::Negative`] gives `-name(…)`.
    ///
    /// # Errors
    ///
    /// As [`Symbol::string`], for a NUL byte in `name`.
    pub fn function_with_sign(name: &str, arguments: &[Symbol], sign: Sign) -> Result<Symbol> {
        raw::create_function(name, arguments, sign == Sign::Positive)
            .map(Symbol)
            .map_err(|e| e.context(format!("creating function symbol {name:?}")))
    }

    /// A tuple, which clingo represents as a function with an empty name.
    ///
    /// # Errors
    ///
    /// As [`Symbol::string`], except that a tuple has no name to contain NUL.
    pub fn tuple(arguments: &[Symbol]) -> Result<Symbol> {
        raw::create_function("", arguments, true)
            .map(Symbol)
            .map_err(|e| e.context("creating a tuple symbol"))
    }

    /// What kind of symbol this is, with its parts: [`SymbolKind::Other`] for
    /// a type clingo uses only internally.
    pub fn kind(&self) -> SymbolKind {
        match raw::symbol_type(self.0) {
            SymbolType::Infimum => SymbolKind::Infimum,
            SymbolType::Supremum => SymbolKind::Supremum,
            SymbolType::Number => self
                .as_number()
                .map_or(SymbolKind::Infimum, SymbolKind::Number),
            SymbolType::String => self
                .as_string()
                .map_or(SymbolKind::Infimum, SymbolKind::String),
            // The type is known, so the three reads skip the type checks the
            // public accessors repeat.
            SymbolType::Function => SymbolKind::Function {
                name: raw::symbol_name(self.0).unwrap_or_default(),
                arguments: raw::symbol_arguments(self.0).unwrap_or_default(),
                sign: raw::symbol_is_positive(self.0).map_or(Sign::Positive, Sign::from_positive),
            },
            // clingo's internal "special" symbol (libgringo/src/symbol.cc:78),
            // or a type a later clingo adds.
            SymbolType::Other(_) => SymbolKind::Other,
        }
    }

    /// The number, if this is a number symbol.
    ///
    /// The accessor is not called `number`, because that is the constructor.
    #[inline]
    pub fn as_number(&self) -> Option<i32> {
        (raw::symbol_type(self.0) == SymbolType::Number)
            .then(|| raw::symbol_number(self.0).ok())
            .flatten()
    }

    /// The content of a string symbol, without quotes or escapes.
    ///
    /// A string clingo read from a file that is not valid UTF-8 is returned with
    /// the invalid bytes replaced by U+FFFD.
    #[inline]
    pub fn as_string(&self) -> Option<&'static str> {
        (raw::symbol_type(self.0) == SymbolType::String)
            .then(|| raw::symbol_string(self.0).ok())
            .flatten()
    }

    /// The name of a function symbol; empty for a tuple.
    #[inline]
    pub fn name(&self) -> Option<&'static str> {
        (raw::symbol_type(self.0) == SymbolType::Function)
            .then(|| raw::symbol_name(self.0).ok())
            .flatten()
    }

    /// The arguments of a function symbol; empty for a constant.
    #[inline]
    pub fn arguments(&self) -> Option<&'static [Symbol]> {
        (raw::symbol_type(self.0) == SymbolType::Function)
            .then(|| raw::symbol_arguments(self.0).ok())
            .flatten()
    }

    /// The sign of a function symbol.
    #[inline]
    pub fn sign(&self) -> Option<Sign> {
        (raw::symbol_type(self.0) == SymbolType::Function)
            .then(|| raw::symbol_is_positive(self.0).ok())
            .flatten()
            .map(Sign::from_positive)
    }

    /// The name, arguments and sign of a function symbol, with one check of
    /// the type instead of the three the accessors make.
    pub(crate) fn function_parts(self) -> Option<(&'static str, &'static [Symbol], Sign)> {
        if raw::symbol_type(self.0) != SymbolType::Function {
            return None;
        }
        Some((
            raw::symbol_name(self.0).ok()?,
            raw::symbol_arguments(self.0).ok()?,
            Sign::from_positive(raw::symbol_is_positive(self.0).ok()?),
        ))
    }
}

impl From<i32> for Symbol {
    fn from(n: i32) -> Symbol {
        Symbol::number(n)
    }
}

impl PartialEq for Symbol {
    #[inline]
    fn eq(&self, other: &Symbol) -> bool {
        raw::symbol_is_equal_to(self.0, other.0)
    }
}

impl Eq for Symbol {}

impl Hash for Symbol {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        raw::symbol_hash(self.0).hash(state);
    }
}

impl PartialOrd for Symbol {
    #[inline]
    fn partial_cmp(&self, other: &Symbol) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Symbol {
    /// clingo's own order, from `clingo_symbol_is_less_than`, which differs
    /// from any order on the raw 64-bit value.
    #[inline]
    fn cmp(&self, other: &Symbol) -> Ordering {
        if raw::symbol_is_less_than(self.0, other.0) {
            Ordering::Less
        } else if raw::symbol_is_less_than(other.0, self.0) {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

/// A string read from a file that is not valid UTF-8 is written with U+FFFD in
/// place of the invalid bytes, as [`Symbol::as_string`] returns it.
impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Lossy, since a `Display` that fails makes `to_string` and `{}` panic.
        // What is left to fail is clingo running out of memory for the text.
        let text = raw::symbol_to_string_lossy(self.0).map_err(|_| fmt::Error)?;
        f.write_str(&text)
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Symbol({self})")
    }
}

impl FromStr for Symbol {
    type Err = Error;

    /// Parses a term in clingo's syntax with `clingo_parse_term`. Arithmetic in
    /// the term is evaluated, so `"1+2"` gives `3`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Parse`](crate::ErrorKind::Parse) for a syntax error, with
    ///   clingo's messages in [`Error::messages`];
    /// - [`ErrorKind::Nul`](crate::ErrorKind::Nul) if `s` contains a NUL byte.
    fn from_str(s: &str) -> Result<Symbol> {
        raw::parse_term(s)
            .map(Symbol)
            .map_err(|e| e.context(format!("parsing term {s:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;

    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn accessors_of_other_kinds_are_none() {
        let s = Symbol::string("s").unwrap();
        assert_eq!(s.name(), None);
        assert_eq!(s.arguments(), None);
        assert_eq!(s.sign(), None);
        assert_eq!(Symbol::supremum().as_string(), None);
        assert_eq!(Symbol::infimum().as_number(), None);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn parsing_evaluates_arithmetic() {
        assert_eq!("1+2".parse::<Symbol>().unwrap(), Symbol::number(3));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn a_parse_error_names_the_term_and_carries_messages() {
        let err = "p(".parse::<Symbol>().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Parse);
        assert!(err.to_string().contains("\"p(\""), "{err}");
        assert!(!err.messages().is_empty(), "{err:?}");
        assert_eq!("a\0".parse::<Symbol>().unwrap_err().kind(), ErrorKind::Nul);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn tuples_have_no_sign_prefix_and_negative_constants_have_a_name() {
        let t = Symbol::tuple(&[]).unwrap();
        assert_eq!(t.to_string(), "()");
        let q = Symbol::function_with_sign("q", &[Symbol::number(1)], Sign::Negative).unwrap();
        assert_eq!(q.to_string(), "-q(1)");
        assert_eq!(q.name(), Some("q"));
    }
}

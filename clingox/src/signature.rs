//! Predicate signatures: a name, an arity and a sign.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use crate::error::{Error, ErrorKind, Result};
use crate::raw::{self, RawSignature};
use crate::symbol::Sign;

/// A predicate signature: a name, an arity and a sign, printed as `p/2` or
/// `-q/0`.
///
/// Like [`Symbol`](crate::Symbol), it is a small value in clingo's global
/// table, `Copy + Send + Sync + 'static` (DESIGN S12). Its order is clingo's:
/// positive signatures before negative ones, then by arity, then by name.
///
/// # Examples
///
/// ```
/// use clingox::{Sign, Signature};
///
/// let p = Signature::new("p", 2)?;
/// assert_eq!(p.to_string(), "p/2");
/// let q: Signature = "-q/0".parse()?;
/// assert_eq!(q.sign(), Sign::Negative);
/// assert!(p < q);
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Clone, Copy)]
pub struct Signature(RawSignature);

impl Signature {
    /// A positive signature.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Nul`] if `name` contains a NUL byte;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// let p = clingox::Signature::new("p", 1)?;
    /// assert_eq!((p.name(), p.arity()), ("p", 1));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn new(name: &str, arity: u32) -> Result<Signature> {
        Signature::with_sign(name, arity, Sign::Positive)
    }

    /// A signature with the given sign; a negative one is that of classically
    /// negated atoms such as `-q(1)`.
    ///
    /// # Errors
    ///
    /// As [`Signature::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Sign, Signature};
    ///
    /// let q = Signature::with_sign("q", 1, Sign::Negative)?;
    /// assert_eq!(q.to_string(), "-q/1");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn with_sign(name: &str, arity: u32, sign: Sign) -> Result<Signature> {
        raw::create_signature(name, arity, sign == Sign::Positive)
            .map(Signature)
            .map_err(|e| e.context(format!("creating signature `{name}/{arity}`")))
    }

    /// Wraps a signature clingo returned.
    pub(crate) fn from_clingo(raw: RawSignature) -> Signature {
        Signature(raw)
    }

    /// The raw value, for passing back to clingo.
    pub(crate) fn raw(self) -> RawSignature {
        self.0
    }

    /// The name. Invalid UTF-8 is replaced with U+FFFD, as in
    /// [`Symbol::name`](crate::Symbol::name).
    pub fn name(&self) -> &'static str {
        raw::signature_name(self.0)
    }

    /// The number of arguments.
    pub fn arity(&self) -> u32 {
        raw::signature_arity(self.0)
    }

    /// The sign.
    pub fn sign(&self) -> Sign {
        if raw::signature_is_positive(self.0) {
            Sign::Positive
        } else {
            Sign::Negative
        }
    }
}

impl PartialEq for Signature {
    fn eq(&self, other: &Signature) -> bool {
        raw::signature_is_equal_to(self.0, other.0)
    }
}

impl Eq for Signature {}

impl Hash for Signature {
    fn hash<H: Hasher>(&self, state: &mut H) {
        raw::signature_hash(self.0).hash(state);
    }
}

impl PartialOrd for Signature {
    fn partial_cmp(&self, other: &Signature) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Signature {
    /// clingo's order (`clingo_signature_is_less_than`): positive before
    /// negative, then by arity, then by name.
    fn cmp(&self, other: &Signature) -> Ordering {
        if raw::signature_is_less_than(self.0, other.0) {
            Ordering::Less
        } else if raw::signature_is_less_than(other.0, self.0) {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

impl fmt::Display for Signature {
    /// `name/arity`, with a leading `-` for a negative signature, as clingo's
    /// C++ API prints it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.sign() == Sign::Negative {
            f.write_str("-")?;
        }
        write!(f, "{}/{}", self.name(), self.arity())
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({self})")
    }
}

impl FromStr for Signature {
    type Err = Error;

    /// Reads the form [`Display`](fmt::Display) prints: an optional `-`, the
    /// name, `/`, and the arity as a decimal `u32`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Parse`] for anything else, including an empty name and a
    ///   missing or out-of-range arity;
    /// - [`ErrorKind::Nul`] if the name contains a NUL byte.
    fn from_str(s: &str) -> Result<Signature> {
        let malformed = || {
            Error::new(
                ErrorKind::Parse,
                format!("{s:?} is not a signature of the form name/arity"),
            )
        };
        let (sign, rest) = match s.strip_prefix('-') {
            Some(rest) => (Sign::Negative, rest),
            None => (Sign::Positive, s),
        };
        let (name, arity) = rest.rsplit_once('/').ok_or_else(malformed)?;
        // `u32::from_str` also takes a leading `+`, which is not decimal form.
        if name.is_empty() || arity.is_empty() || !arity.bytes().all(|b| b.is_ascii_digit()) {
            return Err(malformed());
        }
        let arity = arity.parse().map_err(|_| malformed())?;
        Signature::with_sign(name, arity, sign)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn a_plus_sign_is_not_a_decimal_arity() {
        let err = "p/+1".parse::<Signature>().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Parse);
        assert_eq!("-p/1".parse::<Signature>().unwrap().name(), "p");
        assert_eq!(
            "-/1".parse::<Signature>().unwrap_err().kind(),
            ErrorKind::Parse
        );
    }
}

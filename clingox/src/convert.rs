//! Converting Rust values to and from symbols: [`ToSymbol`], [`FromSymbol`]
//! and [`Predicate`].

use crate::error::{Error, ErrorKind, Result};
use crate::symbol::{Sign, Symbol};

/// A Rust value with a clingo form: it converts to a [`Symbol`].
///
/// [`Control::add_facts`](crate::Control::add_facts) takes values of this
/// trait, and `sym!` splices them. Derive it for your own structs and enums
/// with `#[derive(ToSymbol)]` (feature `derive`, on by default); the
/// [`FromSymbol`] documentation describes the mapping, which is the same in
/// both directions.
///
/// It is implemented for:
///
/// | Type | Symbol |
/// |---|---|
/// | `i32` | the number |
/// | the other integer types | the number; [`ErrorKind::Conversion`] outside `i32` |
/// | [`Symbol`] | itself |
/// | `()` and tuples up to 12 elements | the clingo tuple `()`, `(1,)`, `(1,2)` |
/// | `Vec<T>` and `[T]` | the clingo tuple of the elements |
/// | `&T`, `Box<T>`, `Rc<T>` and `Arc<T>` | as `T`, errors included |
///
/// `String` and `&str` do not implement it, because a Rust string is either a
/// clingo string (`"comp13"`) or a constant (`comp13`), and the two never
/// match each other in rules. Build the symbol with [`Symbol::string`] or
/// [`Symbol::function`], or mark a derived field `#[clingo(string)]` or
/// `#[clingo(constant)]`. `bool`, `Option`, `char` and floats have no clingo
/// value either.
///
/// # Errors
///
/// [`to_symbol`](ToSymbol::to_symbol) keeps the kind of the step that failed:
/// [`ErrorKind::Nul`] for a NUL byte in a string or name, and
/// [`ErrorKind::Conversion`] for a value that has no symbol, such as an
/// integer outside `i32`. A hand-written implementation reports a value
/// without a symbol with [`Error::conversion`].
///
/// # Examples
///
/// ```
/// use clingox::{Symbol, ToSymbol};
///
/// #[derive(ToSymbol)]
/// struct Edge(i32, i32);
///
/// assert_eq!(Edge(1, 2).to_symbol()?.to_string(), "edge(1,2)");
/// assert_eq!((1, vec![2, 3]).to_symbol()?.to_string(), "(1,(2,3))");
/// assert!(u32::MAX.to_symbol().is_err());
/// # Ok::<(), clingox::Error>(())
/// ```
///
/// A hand-written implementation for a type the derive does not cover, which
/// reports a value it cannot convert with [`Error::conversion`]:
///
/// ```
/// use clingox::{Error, ErrorKind, FromSymbol, Symbol, ToSymbol};
///
/// /// A difficulty from 0 to 9.
/// #[derive(Debug, PartialEq)]
/// struct Level(u8);
///
/// impl ToSymbol for Level {
///     fn to_symbol(&self) -> clingox::Result<Symbol> {
///         if self.0 > 9 {
///             return Err(Error::conversion(format!("level {} is above 9", self.0)));
///         }
///         Ok(Symbol::number(i32::from(self.0)))
///     }
/// }
///
/// impl FromSymbol for Level {
///     fn from_symbol(symbol: Symbol) -> clingox::Result<Level> {
///         match symbol.as_number().and_then(|n| u8::try_from(n).ok()) {
///             Some(n) if n <= 9 => Ok(Level(n)),
///             _ => Err(Error::conversion(format!("`{symbol}` is not a level"))),
///         }
///     }
/// }
///
/// assert_eq!(Level(3).to_symbol()?.to_string(), "3");
/// assert_eq!(Level(12).to_symbol().unwrap_err().kind(), ErrorKind::Conversion);
/// assert_eq!(Level::from_symbol(Symbol::number(4))?, Level(4));
/// assert!(Level::from_symbol(Symbol::number(10)).is_err());
/// # Ok::<(), clingox::Error>(())
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no clingo form: it does not implement `ToSymbol`",
    label = "`{Self}` cannot be converted to a clingo symbol",
    note = "derive `ToSymbol` for your own types; for text, mark a `String` field \
            `#[clingo(string)]` or `#[clingo(constant)]`, or build the symbol with \
            `Symbol::string` or `Symbol::function`"
)]
pub trait ToSymbol {
    /// Converts the value to a symbol.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Conversion`] for a value that has no symbol, and
    /// [`ErrorKind::Nul`] for a NUL byte in a string or name.
    fn to_symbol(&self) -> Result<Symbol>;
}

/// A Rust value read from a [`Symbol`].
///
/// Derive it with `#[derive(FromSymbol)]` (feature `derive`, on by default).
/// It is implemented for the integer types (a number that fits the type),
/// [`Symbol`] (any symbol), `()` and tuples up to 12 elements (a tuple of
/// exactly that arity), `Vec<T>` (a tuple of any arity), and `Box<T>` (as
/// `T`, which lets a derived enum hold itself).
///
/// # The derived mapping
///
/// A struct is the function `name(field, …)`, its fields in declaration
/// order, with positive sign. A struct without fields is the constant `name`.
/// The name is the value of `#[clingo(name = "..")]` on the type, or else the
/// type's name in snake case. Snake case puts `_` before each uppercase letter
/// that starts a word, lowercases everything, and keeps leading underscores:
///
/// | Rust name | clingo name |
/// |---|---|
/// | `TaskAssignment` | `task_assignment` |
/// | `HTTPCheck` | `http_check` |
/// | `Vec2` | `vec2` |
/// | `Point3D` | `point3_d` |
/// | `_Hidden` | `_hidden` |
///
/// A letter after a digit starts a new word, so `Point3D` is `point3_d`; write
/// `#[clingo(name = "point3d")]` for another name. A name must be a clingo
/// name, `_*[a-z][A-Za-z0-9_']*` other than `not`; anything else is a compile
/// error. Enum variants are named the same way.
///
/// A `String` field must say what it is: `#[clingo(string)]` for a clingo
/// string (`"comp13"`), or `#[clingo(constant)]` for a constant (`comp13`). A
/// `String` field without either is a compile error. Every other field
/// converts with its own type's trait, so derived types nest:
/// `edge(node(1),node(2))`.
///
/// Each variant of an enum is its own term, named after the variant in snake
/// case or by `#[clingo(name = "..")]` on the variant. `from_symbol` picks the
/// variant whose name and arity match.
///
/// Deriving `FromSymbol` on a struct also implements [`Predicate`], which
/// [`Model::atoms`](crate::Model::atoms) reads by. An enum spans several names
/// and arities, so it does not implement `Predicate`.
///
/// # Errors
///
/// [`from_symbol`](FromSymbol::from_symbol) returns
/// [`ErrorKind::Conversion`] for a symbol of the wrong shape, and its message
/// contains the symbol's text. A derived struct accepts only a positive
/// function with its name and arity; an error in a field names the type, the
/// field and the symbol. A hand-written implementation reports a mismatch
/// with [`Error::conversion`], as the example under [`ToSymbol`] does.
///
/// # Examples
///
/// ```
/// use clingox::{ErrorKind, FromSymbol, Predicate, Symbol};
///
/// #[derive(FromSymbol, Debug, PartialEq)]
/// struct Asserted {
///     #[clingo(constant)]
///     object: String,
///     #[clingo(string)]
///     note: String,
///     value: u8,
/// }
///
/// let symbol: Symbol = r#"asserted(comp13,"checked",3)"#.parse()?;
/// let asserted = Asserted::from_symbol(symbol)?;
/// assert_eq!(asserted.object, "comp13");
/// assert_eq!(asserted.note, "checked");
/// assert_eq!(<Asserted as Predicate>::ARITY, 3);
///
/// let too_big: Symbol = r#"asserted(comp13,"checked",300)"#.parse()?;
/// let err = Asserted::from_symbol(too_big).unwrap_err();
/// assert_eq!(err.kind(), ErrorKind::Conversion);
/// # Ok::<(), clingox::Error>(())
/// ```
///
/// An enum:
///
/// ```
/// use clingox::{FromSymbol, Symbol, ToSymbol};
///
/// #[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
/// enum Shape {
///     Point,
///     Circle(i32),
///     Rect { w: i32, h: i32 },
///     #[clingo(name = "sq")]
///     Square(i32),
/// }
///
/// assert_eq!(Shape::Rect { w: 2, h: 3 }.to_symbol()?.to_string(), "rect(2,3)");
/// assert_eq!(Shape::from_symbol("sq(4)".parse()?)?, Shape::Square(4));
/// assert!(Shape::from_symbol("circle(1,2)".parse()?).is_err());
/// # Ok::<(), clingox::Error>(())
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no clingo form: it does not implement `FromSymbol`",
    label = "`{Self}` cannot be read from a clingo symbol",
    note = "derive `FromSymbol` for your own types; for text, mark a `String` field \
            `#[clingo(string)]` or `#[clingo(constant)]`, or read the `Symbol` and use \
            `Symbol::as_string` or `Symbol::name`"
)]
pub trait FromSymbol: Sized {
    /// Reads a value from a symbol.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Conversion`] if the symbol does not have the shape of the
    /// type; the message contains the symbol's text.
    fn from_symbol(symbol: Symbol) -> Result<Self>;
}

/// A type read from the atoms of one predicate, `NAME/ARITY` with positive
/// sign.
///
/// [`Model::atoms`](crate::Model::atoms) and
/// [`Model::shown`](crate::Model::shown) select the symbols of this predicate
/// and convert each with [`FromSymbol`]. `#[derive(FromSymbol)]` implements it
/// for a struct; implement it by hand for a type with its own
/// [`FromSymbol`].
///
/// # Examples
///
/// ```
/// use clingox::{FromSymbol, Predicate};
///
/// #[derive(FromSymbol)]
/// #[clingo(name = "edge")]
/// struct Link(i32, i32);
///
/// assert_eq!(Link::NAME, "edge");
/// assert_eq!(Link::ARITY, 2);
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a predicate: it does not implement `Predicate`",
    label = "`{Self}` does not name one predicate",
    note = "`#[derive(FromSymbol)]` on a struct implements `Predicate`; an enum spans several \
            names and arities, so read each predicate into a struct of its own"
)]
pub trait Predicate: FromSymbol {
    /// The predicate's name.
    const NAME: &'static str;
    /// The predicate's arity: the number of arguments.
    const ARITY: u32;
}

/// The error for a symbol that does not have the shape `expected` describes.
pub(crate) fn mismatch(symbol: Symbol, expected: &str) -> Error {
    Error::conversion(format!("`{symbol}` is not {expected}"))
}

/// The elements of a tuple symbol: a positive function with an empty name.
fn tuple_elements(symbol: Symbol) -> Option<&'static [Symbol]> {
    let (name, arguments, sign) = symbol.function_parts()?;
    (name.is_empty() && sign == Sign::Positive).then_some(arguments)
}

impl<T: ToSymbol + ?Sized> ToSymbol for &T {
    fn to_symbol(&self) -> Result<Symbol> {
        (**self).to_symbol()
    }
}

impl<T: ToSymbol + ?Sized> ToSymbol for Box<T> {
    fn to_symbol(&self) -> Result<Symbol> {
        (**self).to_symbol()
    }
}

impl<T: FromSymbol> FromSymbol for Box<T> {
    fn from_symbol(symbol: Symbol) -> Result<Self> {
        T::from_symbol(symbol).map(Box::new)
    }
}

impl<T: ToSymbol + ?Sized> ToSymbol for std::rc::Rc<T> {
    fn to_symbol(&self) -> Result<Symbol> {
        (**self).to_symbol()
    }
}

impl<T: ToSymbol + ?Sized> ToSymbol for std::sync::Arc<T> {
    fn to_symbol(&self) -> Result<Symbol> {
        (**self).to_symbol()
    }
}

impl ToSymbol for Symbol {
    fn to_symbol(&self) -> Result<Symbol> {
        Ok(*self)
    }
}

impl FromSymbol for Symbol {
    fn from_symbol(symbol: Symbol) -> Result<Self> {
        Ok(symbol)
    }
}

impl ToSymbol for i32 {
    fn to_symbol(&self) -> Result<Symbol> {
        Ok(Symbol::number(*self))
    }
}

impl FromSymbol for i32 {
    fn from_symbol(symbol: Symbol) -> Result<Self> {
        symbol
            .as_number()
            .ok_or_else(|| mismatch(symbol, "a number"))
    }
}

/// The integer types other than `i32`, which convert with a range check in
/// each direction, never by truncation.
macro_rules! checked_integers {
    ($($t:ty),*) => {$(
        impl ToSymbol for $t {
            fn to_symbol(&self) -> Result<Symbol> {
                i32::try_from(*self).map(Symbol::number).map_err(|_| {
                    Error::new(
                        ErrorKind::Conversion,
                        format!(
                            "the {} {self} is outside the range of clingo numbers ({} to {})",
                            stringify!($t),
                            i32::MIN,
                            i32::MAX
                        ),
                    )
                })
            }
        }

        impl FromSymbol for $t {
            fn from_symbol(symbol: Symbol) -> Result<Self> {
                let number = i32::from_symbol(symbol)?;
                <$t>::try_from(number).map_err(|_| {
                    mismatch(symbol, concat!("a number within the range of `", stringify!($t), "`"))
                })
            }
        }
    )*};
}

checked_integers!(i8, i16, i64, i128, isize, u8, u16, u32, u64, u128, usize);

impl<T: ToSymbol> ToSymbol for [T] {
    fn to_symbol(&self) -> Result<Symbol> {
        let elements = self
            .iter()
            .map(ToSymbol::to_symbol)
            .collect::<Result<Vec<_>>>()?;
        Symbol::tuple(&elements)
    }
}

impl<T: ToSymbol> ToSymbol for Vec<T> {
    fn to_symbol(&self) -> Result<Symbol> {
        self.as_slice().to_symbol()
    }
}

impl<T: FromSymbol> FromSymbol for Vec<T> {
    fn from_symbol(symbol: Symbol) -> Result<Self> {
        let elements = tuple_elements(symbol).ok_or_else(|| mismatch(symbol, "a tuple"))?;
        elements
            .iter()
            .map(|element| {
                T::from_symbol(*element).map_err(|e| e.context(format!("reading tuple `{symbol}`")))
            })
            .collect()
    }
}

/// Tuples convert to clingo tuples of the same arity.
macro_rules! tuples {
    ($(($($name:ident $index:tt),*) = $arity:literal;)*) => {$(
        impl<$($name: ToSymbol),*> ToSymbol for ($($name,)*) {
            fn to_symbol(&self) -> Result<Symbol> {
                Symbol::tuple(&[$(self.$index.to_symbol()?),*])
            }
        }

        impl<$($name: FromSymbol),*> FromSymbol for ($($name,)*) {
            fn from_symbol(symbol: Symbol) -> Result<Self> {
                match tuple_elements(symbol) {
                    Some([$($name),*]) => Ok(($(
                        $name::from_symbol(*$name)
                            .map_err(|e| e.context(format!("reading tuple `{symbol}`")))?,
                    )*)),
                    _ => Err(mismatch(symbol, concat!("a tuple of ", $arity, " elements"))),
                }
            }
        }
    )*};
}

#[allow(non_snake_case)]
mod tuple_impls {
    use super::{FromSymbol, Result, Symbol, ToSymbol, mismatch, tuple_elements};

    tuples! {
        () = 0;
        (A 0) = 1;
        (A 0, B 1) = 2;
        (A 0, B 1, C 2) = 3;
        (A 0, B 1, C 2, D 3) = 4;
        (A 0, B 1, C 2, D 3, E 4) = 5;
        (A 0, B 1, C 2, D 3, E 4, F 5) = 6;
        (A 0, B 1, C 2, D 3, E 4, F 5, G 6) = 7;
        (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7) = 8;
        (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8) = 9;
        (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9) = 10;
        (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10) = 11;
        (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11) = 12;
    }
}

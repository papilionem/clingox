//! Safe, idiomatic Rust bindings to clingo, the answer set programming system.
//!
//! A [`Control`] holds a logic program. Add program text with
//! [`Control::add`], ground it with [`Control::ground`], and solve it with
//! [`Control::solve`]. Ground terms are [`Symbol`]s.
//!
//! Models are read while the search runs: [`Control::for_each_model`] calls a
//! closure on each one, and [`Control::solve_yield`] returns a [`SolveHandle`]
//! that lends them one at a time. [`Control::solve_first`],
//! [`Control::solve_optimal`] and [`Control::solve_all`] return owned copies.
//!
//! ```
//! use clingox::prelude::*;
//!
//! let mut ctl = Control::new()?;
//! ctl.add_base("a :- not b. b :- not a.")?;
//! ctl.ground(&[Part::base()])?;
//! assert!(ctl.solve(&[])?.is_sat());
//! # Ok::<(), clingox::Error>(())
//! ```
//!
//! Errors are [`Error`]s; match on [`Error::kind`]. The result alias is
//! [`clingox::Result`](Result), which the prelude does not export.
//!
//! clingox builds and links clingo 5.8.2. A clingo installed on the system is
//! accepted from 5.8.1 on within 5.8, at build time and again at run time,
//! where any other version is [`ErrorKind::Version`].

#![deny(unsafe_code)]

#[allow(unsafe_code)]
mod raw;

pub mod application;
pub mod ast;
mod async_solve;
mod atoms;
pub mod backend;
mod builder;
mod config;
mod control;
mod convert;
mod error;
mod externals;
mod facts;
mod ground;
mod interrupt;
mod model;
pub mod observer;
pub mod propagate;
pub mod script;
mod signature;
mod solve;
mod solve_events;
mod stats;
mod symbol;
pub mod testing;
mod theory;

#[cfg(doctest)]
mod doctests;

#[doc(hidden)]
#[path = "private.rs"]
pub mod __private;

pub use async_solve::AsyncSolveHandle;
pub use atoms::{ProgramLiteral, SymbolicAtom, SymbolicAtomIter, SymbolicAtoms};
pub use builder::ControlBuilder;
pub use config::{ConfigKind, Configuration};
pub use control::{Assumption, Control, Part, ScopedControl, SolveResult};
pub use convert::{FromSymbol, Predicate, ToSymbol};
pub use error::{Error, ErrorKind, Location, Message, MessageCode, Result};
pub use externals::TruthValue;
pub use ground::FunctionCall;
pub use interrupt::{InterruptHandle, SolveOptions};
pub use model::{
    Consequence, ExtendableModel, Model, ModelKind, OwnedModel, ShowType, SolveControl,
};
pub use script::Script;
pub use signature::Signature;
pub use solve::{Outcome, SolveHandle};
pub use solve_events::SolveEventHandler;
pub use stats::{MutableStatistics, StatKind, Statistics, StatsTree};
pub use symbol::{Sign, Symbol, SymbolKind};
pub use theory::{
    Id, TheoryAtom, TheoryAtomIter, TheoryAtoms, TheoryElement, TheoryTerm, TheoryTermKind,
};

/// The version of the clingo library this program is linked with, as
/// `(major, minor, revision)`, asked of the library at run time.
///
/// The crate is built for clingo 5.8.2 ([`clingox_sys::CLINGO_VERSION`] is that
/// compile-time value). A clingo installed on the system is accepted from 5.8.1
/// within 5.8, so a program that needs a fix of one revision can compare this
/// with the number it wants.
///
/// # Examples
///
/// ```
/// let (major, minor, _revision) = clingox::version();
/// assert_eq!((major, minor), (5, 8));
/// ```
#[must_use]
pub fn version() -> (u32, u32, u32) {
    raw::library_version()
}

/// Derives [`ToSymbol`](trait@ToSymbol) for a struct or an enum.
///
/// The mapping and the `#[clingo(..)]` attributes are described under
/// [`FromSymbol`](trait@FromSymbol), and are the same in both directions.
/// Needs the feature `derive`, on by default.
///
/// # Examples
///
/// ```
/// use clingox::ToSymbol;
///
/// #[derive(ToSymbol)]
/// struct Asserted {
///     #[clingo(constant)]
///     object: String,
///     value: i32,
/// }
///
/// let fact = Asserted { object: "comp13".into(), value: 0 };
/// assert_eq!(fact.to_symbol()?.to_string(), "asserted(comp13,0)");
/// # Ok::<(), clingox::Error>(())
/// ```
#[cfg(feature = "derive")]
pub use clingox_derive::ToSymbol;

/// Derives [`FromSymbol`](trait@FromSymbol) for a struct or an enum, and
/// [`Predicate`] for a struct.
///
/// The mapping and the `#[clingo(..)]` attributes are described under
/// [`FromSymbol`](trait@FromSymbol). Needs the feature `derive`, on by
/// default.
///
/// # Examples
///
/// ```
/// use clingox::{FromSymbol, Predicate};
///
/// #[derive(FromSymbol, Debug, PartialEq)]
/// #[clingo(name = "edge")]
/// struct Link(i32, i32);
///
/// assert_eq!(Link::from_symbol("edge(1,2)".parse()?)?, Link(1, 2));
/// assert_eq!((Link::NAME, Link::ARITY), ("edge", 2));
/// # Ok::<(), clingox::Error>(())
/// ```
#[cfg(feature = "derive")]
pub use clingox_derive::FromSymbol;

/// Builds a [`Symbol`] from clingo's term syntax.
///
/// `sym!` takes one ground term and expands to an expression of type
/// [`clingox::Result<Symbol>`](Result). It accepts:
///
/// - an integer within `i32`, with an optional `-`: `sym!(-5)`;
/// - a string literal, plain or raw, whose value is the string's content:
///   `sym!("a b")`;
/// - `#sup` and `#inf`;
/// - a name, optionally with arguments and optionally with `-` for classical
///   negation: `sym!(-p(1, q))`. Rust keywords are ordinary names
///   (`sym!(type(1))`), and `p()` is the constant `p`;
/// - a tuple: `()`, `(1,)`, `(1, 2)`; `(t)` is `t`, as in clingo;
/// - a splice `{expr}` of any value that implements [`ToSymbol`](trait@ToSymbol).
///   The expression is borrowed, evaluated once, in the caller's context
///   (so `?` inside it returns from the caller's function), and splices are
///   evaluated from left to right.
///
/// Variables, a missing or extra comma, an integer outside `i32` or with a
/// suffix, and any other token are compile errors that point at the token.
/// Arithmetic, intervals and pools are not accepted; build such symbols by
/// parsing text with [`str::parse`] instead.
///
/// # Errors
///
/// At run time, the error of a splice's `to_symbol`, unchanged, and
/// [`ErrorKind::Nul`] for a string with a NUL byte.
///
/// # Examples
///
/// ```
/// use clingox::{Symbol, sym};
///
/// let n = 42;
/// let s = sym!(p(1, "x", c, -q, (1, 2), #sup, {n}))?;
/// assert_eq!(s.to_string(), r#"p(1,"x",c,-q,(1,2),#sup,42)"#);
/// assert_eq!(s, r#"p(1,"x",c,-q,(1,2),#sup,42)"#.parse::<Symbol>()?);
/// # Ok::<(), clingox::Error>(())
/// ```
///
/// A variable is a compile error:
///
/// ```compile_fail
/// let s = clingox::sym!(p(X));
/// ```
#[cfg(feature = "derive")]
pub use clingox_derive::sym;

/// The common types, for `use clingox::prelude::*`.
///
/// It leaves out [`Error`], [`ErrorKind`], [`Message`], [`Location`] and the
/// [`Result`] alias, so a glob import cannot shadow a two-parameter `Result`.
/// It adds [`std::ops::ControlFlow`], which model closures return.
pub mod prelude {
    pub use std::ops::ControlFlow;

    pub use crate::builder::ControlBuilder;
    pub use crate::control::{Assumption, Control, Part, ScopedControl, SolveResult};
    pub use crate::convert::{FromSymbol, Predicate, ToSymbol};
    pub use crate::error::MessageCode;
    pub use crate::externals::TruthValue;
    pub use crate::ground::FunctionCall;
    pub use crate::interrupt::SolveOptions;
    pub use crate::model::{ExtendableModel, Model, ModelKind, OwnedModel, ShowType, SolveControl};
    pub use crate::script::Script;
    pub use crate::signature::Signature;
    pub use crate::solve::{Outcome, SolveHandle};
    pub use crate::solve_events::SolveEventHandler;
    pub use crate::stats::{StatKind, StatsTree};
    pub use crate::symbol::{Sign, Symbol, SymbolKind};
    #[cfg(feature = "derive")]
    pub use clingox_derive::{FromSymbol, ToSymbol, sym};
}

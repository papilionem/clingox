//! Safe Rust bindings to [clingo](https://potassco.org/clingo/), the answer set
//! programming system from the Potassco project.
//!
//! Answer set programming (ASP) states a problem as a logic program: facts,
//! rules and constraints. clingo grounds the program and searches for its
//! answer sets, each of which is a solution. clingox drives clingo from Rust: it
//! adds program text and facts, grounds and solves, and reads the answer sets
//! back, with no `unsafe` in your code. It builds clingo 5.8.2 from vendored
//! source, with patches for known clingo defects, or links a clingo installed
//! on the system.
//!
//! # Example
//!
//! ```
//! use clingox::prelude::*;
//!
//! let mut ctl = Control::new()?;
//! // Pick exactly one colour, but not red.
//! ctl.add_base(
//!     "colour(red; green; blue).
//!      { pick(C) : colour(C) } = 1.
//!      :- pick(red).
//!      #show pick/1.",
//! )?;
//! ctl.ground(&[Part::base()])?;
//!
//! let (result, models) = ctl.solve_all()?;
//! assert!(result.is_sat());
//! let picks: Vec<String> = models
//!     .iter()
//!     .flat_map(|model| model.symbols())
//!     .map(ToString::to_string)
//!     .collect();
//! assert_eq!(picks, ["pick(blue)", "pick(green)"]);
//! # Ok::<(), clingox::Error>(())
//! ```
//!
//! # Where to start
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
//! The typed layer converts between Rust values and clingo terms:
//! [`ToSymbol`](trait@ToSymbol) and [`FromSymbol`](trait@FromSymbol), their
//! derives, [`Control::add_facts`] and [`Model::atoms`].
//!
//! Errors are [`Error`]s; match on [`Error::kind`]. The result alias is
//! [`clingox::Result`](Result), which the prelude does not export.
//!
//! # Modules
//!
//! | Module | Contents |
//! |---|---|
//! | [`prelude`] | the common types, for `use clingox::prelude::*` |
//! | [`propagate`] | propagators: custom theories inside the search |
//! | [`observer`] | watching the ground program as it is built |
//! | [`backend`] | adding ground rules directly, bypassing the grounder |
//! | [`ast`] | clingo's syntax trees: parsing and rewriting programs |
//! | [`application`] | running clingo's own command line with your options |
//! | [`script`] | custom scripting languages for `#script` blocks |
//! | [`testing`] | helpers for testing logic programs |
//!
//! # Feature flags
//!
//! All four are on by default.
//!
//! | Feature | What it does |
//! |---|---|
//! | `vendored` | Builds clingo 5.8.2 from the source in `clingox-sys`, with its patches. Without it, a system clingo 5.8.1 or newer within 5.8 is linked. |
//! | `threads` | Builds clingo with threads: parallel solving, timeouts and async solving. Without it, those return [`ErrorKind::Unsupported`]. |
//! | `derive` | `#[derive(ToSymbol)]`, `#[derive(FromSymbol)]` and [`sym!`](macro@sym). |
//! | `log` | Forwards clingo's messages to the `log` crate, target `clingox`. |
//!
//! # Versions and platforms
//!
//! The version number names the clingo release inside: `508.2.x` contains clingo
//! 5.8.2. A clingo installed on the system is accepted from 5.8.1 on within 5.8,
//! at build time and again at run time, where any other version is
//! [`ErrorKind::Version`]. The [versions chapter] explains the scheme and the
//! MSRV policy, and the [platform page] lists the targets the test suite runs on.
//!
//! # Further reading
//!
//! [The clingox guide](https://papilionem.github.io/clingox/) teaches the crate
//! from the first program on, and has pages for readers coming from
//! [pyclingo](https://papilionem.github.io/clingox/reference/coming-from-pyclingo.html)
//! or the [`clingo` crate](https://papilionem.github.io/clingox/reference/coming-from-clingo-crate.html).
//!
//! [versions chapter]: https://papilionem.github.io/clingox/concepts/versions.html
//! [platform page]: https://papilionem.github.io/clingox/reference/platforms.html

#![cfg_attr(docsrs, feature(doc_cfg))]
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
mod segment;
mod signature;
mod solve;
mod solve_events;
mod stats;
mod symbol;
pub mod testing;
mod theory;
mod walk;

#[cfg(doctest)]
mod doctests;

#[doc(hidden)]
#[path = "private.rs"]
pub mod __private;

pub use async_solve::AsyncSolveHandle;
pub use atoms::{ProgramLiteral, SymbolicAtom, SymbolicAtomIter, SymbolicAtoms};
pub use builder::ControlBuilder;
pub use config::{ConfigChildren, ConfigEntry, ConfigKind, Configuration};
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
pub use segment::PathSegment;
pub use signature::Signature;
pub use solve::{Outcome, SolveHandle};
pub use solve_events::SolveEventHandler;
pub use stats::{MutableStatistics, StatKind, Statistics, StatsChildren, StatsEntry, StatsTree};
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
    pub use crate::segment::PathSegment;
    pub use crate::signature::Signature;
    pub use crate::solve::{Outcome, SolveHandle};
    pub use crate::solve_events::SolveEventHandler;
    pub use crate::stats::{StatKind, StatsTree};
    pub use crate::symbol::{Sign, Symbol, SymbolKind};
    #[cfg(feature = "derive")]
    pub use clingox_derive::{FromSymbol, ToSymbol, sym};
}

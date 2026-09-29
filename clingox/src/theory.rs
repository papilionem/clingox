//! The theory atoms of a grounding: inspecting theory terms, elements
//! and atoms that occur in the ground program.
//!
//! `#theory` blocks let a program declare its own syntax for statements
//! clingo does not interpret itself, such as `&sum { 1; 2 } <= 3.`. clingox
//! does not interpret them either; it only lets a program read back what
//! clingo parsed, the same way [`crate::SymbolicAtoms`] reads back ordinary
//! atoms. Authoring theory atoms from the backend and watching them
//! through the ground program observer build on the [`Id`] newtype
//! this module introduces.

#[cfg(doc)]
use crate::control::Control;
use crate::control::ScopedControl;
use std::fmt;

use crate::atoms::ProgramLiteral;
use crate::control::ErrorSink;
use crate::error::{Error, ErrorKind, Result};
use crate::raw;
use crate::symbol::Symbol;

/// The id of a theory term, element or atom (`clingo_id_t`).
///
/// It is a separate id space from [`ProgramLiteral`] and clingox's other
/// newtypes (DESIGN S17); the backend and the ground program observer reuse it
/// for the same purpose.
///
/// **Ids are reused.** clingo resets all structural information about theory
/// atoms, elements and terms after each [`Control::solve`]: if a later
/// [`Control::ground`] call grounds fresh theory atoms, they get the same ids
/// an earlier grounding used, starting at zero again (`clingo.h`'s
/// `TheoryAtoms` group). An `Id` from one grounding read after a later one is
/// not detected as stale by clingox: if it is out of range for the new
/// grounding, clingo reports a clean [`ErrorKind::Logic`]; if it happens to
/// still be in range, it silently names whatever term, element or atom now has
/// that number. This is the same "wrong but never unsafe" shape
/// [`crate::ProgramLiteral`] and [`crate::Symbol`] already document for a value
/// read from another grounding.
///
/// `Id` has no public constructor: every value a caller can hold either came
/// from [`TheoryAtoms::iter`] (which only ever generates ids in the current
/// `0..len()` range) or from a term, element or atom clingo itself returned
/// while resolving one of those. An out-of-range *atom* id specifically
/// segfaults clingo (checked directly against clingo 5.8.2); the implementation
/// never passes clingo an atom id that did not come from the current `0..len()`
/// range of the same, still-borrowed [`TheoryAtoms`], and each `unsafe` block
/// that calls an atom-id-taking function says so.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("#theory t { term { }; &a/0 : term, head }. &a { 1 }.")?;
/// ctl.ground(&[Part::base()])?;
/// let id = ctl.theory_atoms()?.iter().next().unwrap()?.id();
/// assert_eq!(format!("{id:?}"), "Id(0)");
/// # Ok::<(), clingox::Error>(())
/// ```
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Id(u32);

impl Id {
    pub(crate) fn from_raw(raw: u32) -> Id {
        Id(raw)
    }

    pub(crate) fn raw(self) -> u32 {
        self.0
    }
}

/// The kind of a theory term (`clingo_theory_term_type_e`).
///
/// [`TheoryAtoms::term_kind`] reads it; [`TheoryAtoms::term`] dispatches on it
/// internally to build the right [`TheoryTerm`] variant, so a caller can never
/// call a type-specific accessor on the wrong kind.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part, TheoryTermKind};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("#theory t { term { }; &a/0 : term, head }. &a { 1 }.")?;
/// ctl.ground(&[Part::base()])?;
/// let atoms = ctl.theory_atoms()?;
/// let atom = atoms.iter().next().unwrap()?;
/// let term = atom.elements()?[0].tuple()?[0];
/// assert_eq!(atoms.term_kind(term)?, TheoryTermKind::Number);
/// # Ok::<(), clingox::Error>(())
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TheoryTermKind {
    /// A tuple term, `(1,2,3)`.
    Tuple,
    /// A list term, `[1,2,3]`.
    List,
    /// A set term, `{1,2,3}`.
    Set,
    /// A function term, `f(1,2,3)`.
    Function,
    /// A number term, `42`.
    Number,
    /// A symbolic constant term, `c`.
    Symbol,
}

fn kind_from_raw(raw: raw::TermType) -> Result<TheoryTermKind> {
    Ok(match raw {
        raw::TermType::Tuple => TheoryTermKind::Tuple,
        raw::TermType::List => TheoryTermKind::List,
        raw::TermType::Set => TheoryTermKind::Set,
        raw::TermType::Function => TheoryTermKind::Function,
        raw::TermType::Number => TheoryTermKind::Number,
        raw::TermType::Symbol => TheoryTermKind::Symbol,
        raw::TermType::Other(value) => {
            return Err(Error::new(
                ErrorKind::Unknown,
                format!("clingo reported an unrecognized theory term kind ({value})"),
            ));
        }
    })
}

/// A theory term, resolved from a [`TheoryAtoms`] view.
///
/// A compound term's arguments are themselves resolved `TheoryTerm` values, not
/// ids a caller would have to look up again: [`TheoryAtoms::term`] builds the
/// whole tree in one call, the same way it settles the top-level kind before
/// calling clingo's type-specific accessor, so there is no wrong-kind call to
/// make from safe code at any depth.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part, TheoryTerm, TheoryTermKind};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("#theory t { term { }; &a/0 : term, head }. &a { f(1) }.")?;
/// ctl.ground(&[Part::base()])?;
/// let atoms = ctl.theory_atoms()?;
/// let atom = atoms.iter().next().unwrap()?;
/// let term_id = atom.elements()?[0].tuple()?[0];
/// let term = atoms.term(term_id)?;
/// assert_eq!(term.to_string(), "f(1)");
/// let TheoryTerm::Compound { kind: TheoryTermKind::Function, name: Some("f"), ref arguments } = term
/// else {
///     panic!("expected the function f");
/// };
/// assert_eq!(*arguments, vec![TheoryTerm::Number(1)]);
/// # Ok::<(), clingox::Error>(())
/// ```
#[derive(Debug)]
pub enum TheoryTerm<'c> {
    /// A number term, `42`.
    Number(i32),
    /// A symbolic constant term, `c`, as the 0-arity function symbol clingo's
    /// own parser would build from the same name, or a string term, `"text"`,
    /// as the [`Symbol::string`] of the text without its quotes (the escapes
    /// `\n`, `\\` and `\"` resolved), the value the same string has in the
    /// program.
    Symbol(Symbol),
    /// A tuple, list, set or function term.
    Compound {
        /// [`TheoryTermKind::Tuple`], [`TheoryTermKind::List`],
        /// [`TheoryTermKind::Set`] or [`TheoryTermKind::Function`]. Every
        /// value this crate builds is one of those four;
        /// [`TheoryTermKind::Number`] or [`TheoryTermKind::Symbol`] here is a
        /// value only a caller can construct directly, since this field is
        /// public. [`Display`](fmt::Display) does not panic on it, but
        /// prints no particular text either: clingo itself never produces
        /// the combination, so there is no oracle for what it should look
        /// like.
        kind: TheoryTermKind,
        /// The function's name; `None` for a tuple, list or set, or, again
        /// only from a caller-constructed value, for a `Function` with no
        /// name. [`Display`](fmt::Display) treats a missing name as empty
        /// rather than panicking.
        name: Option<&'c str>,
        /// The term's arguments, resolved.
        arguments: Vec<TheoryTerm<'c>>,
    },
}

impl<'c> Clone for TheoryTerm<'c> {
    /// Iterative: the derived `Clone` would recurse into
    /// `Vec<TheoryTerm>::clone`, one native stack frame per level, once for
    /// every level of a compound term's own arguments. This clones a
    /// leaf directly and defers a compound's clone until its own arguments
    /// have all been cloned, tracked on an explicit stack instead of the
    /// native call stack, so cloning a term of any depth costs O(1) native
    /// stack frames.
    fn clone(&self) -> Self {
        enum Frame<'a, 'c> {
            Visit(&'a TheoryTerm<'c>),
            Build {
                kind: TheoryTermKind,
                name: Option<&'c str>,
                arity: usize,
            },
        }
        let mut stack = vec![Frame::Visit(self)];
        let mut built: Vec<TheoryTerm<'c>> = Vec::new();
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Visit(TheoryTerm::Number(n)) => built.push(TheoryTerm::Number(*n)),
                Frame::Visit(TheoryTerm::Symbol(s)) => built.push(TheoryTerm::Symbol(*s)),
                Frame::Visit(TheoryTerm::Compound {
                    kind,
                    name,
                    arguments,
                }) => {
                    stack.push(Frame::Build {
                        kind: *kind,
                        name: *name,
                        arity: arguments.len(),
                    });
                    for argument in arguments.iter().rev() {
                        stack.push(Frame::Visit(argument));
                    }
                }
                Frame::Build { kind, name, arity } => {
                    // The last `arity` items built are this compound's own
                    // arguments, in order (each was pushed onto `built` only
                    // once fully resolved, in left-to-right visitation
                    // order, since the loop above pushes them onto `stack`
                    // in reverse so they come off, and so finish, in the
                    // original order).
                    let split_at = built.len() - arity;
                    let arguments = built.split_off(split_at);
                    built.push(TheoryTerm::Compound {
                        kind,
                        name,
                        arguments,
                    });
                }
            }
        }
        built
            .pop()
            .expect("the worklist always finishes with exactly the one cloned term")
    }
}

impl PartialEq for TheoryTerm<'_> {
    /// Iterative: the derived `PartialEq` would recurse into
    /// `Vec<TheoryTerm>::eq`, one native stack frame per level. This walks
    /// both trees in lockstep on an explicit stack of node pairs instead,
    /// stopping at the first mismatch, so comparing two terms of any depth
    /// costs O(1) native stack frames.
    fn eq(&self, other: &Self) -> bool {
        let mut stack = vec![(self, other)];
        while let Some((a, b)) = stack.pop() {
            match (a, b) {
                (TheoryTerm::Number(a), TheoryTerm::Number(b)) => {
                    if a != b {
                        return false;
                    }
                }
                (TheoryTerm::Symbol(a), TheoryTerm::Symbol(b)) => {
                    if a != b {
                        return false;
                    }
                }
                (
                    TheoryTerm::Compound {
                        kind: k1,
                        name: n1,
                        arguments: a1,
                    },
                    TheoryTerm::Compound {
                        kind: k2,
                        name: n2,
                        arguments: a2,
                    },
                ) => {
                    if k1 != k2 || n1 != n2 || a1.len() != a2.len() {
                        return false;
                    }
                    stack.extend(a1.iter().zip(a2.iter()));
                }
                (
                    TheoryTerm::Number(_) | TheoryTerm::Symbol(_) | TheoryTerm::Compound { .. },
                    _,
                ) => {
                    return false;
                }
            }
        }
        true
    }
}

impl Drop for TheoryTerm<'_> {
    /// Iterative: the compiler's own field-by-field drop glue
    /// would recurse into `Vec<TheoryTerm>::drop`, one native stack frame
    /// per level, exactly as the derived `Clone` and `PartialEq` above did.
    /// This takes this node's own arguments out first (leaving it with
    /// nothing left to recurse into once this function returns and the
    /// compiler's own glue runs on what remains), and flattens the rest of
    /// the tree onto an explicit worklist instead: each popped term has its
    /// own arguments taken out the same way before it is allowed to drop,
    /// so no `TheoryTerm` value's own drop ever has non-empty arguments left
    /// to recurse into, whatever the tree's depth.
    fn drop(&mut self) {
        let mut worklist: Vec<TheoryTerm<'_>> = Vec::new();
        if let TheoryTerm::Compound { arguments, .. } = self {
            worklist.append(arguments);
        }
        while let Some(mut term) = worklist.pop() {
            if let TheoryTerm::Compound { arguments, .. } = &mut term {
                worklist.append(arguments);
            }
            // `term` drops here with empty arguments, so this recurses no
            // further (its own `Drop::drop` runs, finds nothing left in
            // `arguments`, and returns at once).
        }
    }
}

/// The leading characters gringo treats as an operator token
/// (`libgringo/src/output/theory.cc`, `TheoryData::printTerm`).
const THEORY_OPERATOR_CHARS: &str = "/!<=>+-*\\?&@|:;~^.";

/// The operator text to print a function term as an operator expression
/// (`(a+b)`, `(-a)`) instead of ordinary prefix notation (`f(a,b)`), or `None`
/// to print it the ordinary way.
///
/// Mirrors gringo's own `TheoryData::printTerm` exactly: a function of arity
/// 0 to 2 is printed as an operator when its name starts with one of
/// [`THEORY_OPERATOR_CHARS`], or is exactly `"not"`; a function of arity 3 or
/// more is always printed as `name(args)`, whatever its name looks like.
fn operator_form(name: &str, arity: usize) -> Option<&str> {
    if arity > 2 {
        return None;
    }
    if name.starts_with(|c: char| THEORY_OPERATOR_CHARS.contains(c)) {
        return Some(name);
    }
    if name == "not" {
        return Some(if arity == 1 { "not " } else { " not " });
    }
    None
}

impl fmt::Display for TheoryTerm<'_> {
    /// Prints the term exactly as `clingo_theory_atoms_term_to_string` would
    /// (`libgringo/src/output/theory.cc`, `TheoryData::printTerm`, which
    /// backs the C function): a negative number in parentheses, a symbol as
    /// itself, a list as `[args]`, a set as `{args}`, and a tuple as `(args)`
    /// except a one-element tuple, which keeps its trailing comma (`(1,)`).
    /// A function is `name(args)`, unless its name makes it an operator, in
    /// which case it prints infix (`(a+b)`) or prefix unary (`(-a)`,
    /// `(not a)`) instead. Every leaf recurses into a [`Symbol`]'s own
    /// `Display`, which already produces clingo's exact text.
    ///
    /// This is computed here rather than by calling
    /// `clingo_theory_atoms_term_to_string` again, because a resolved
    /// [`TheoryTerm`] no longer holds the atoms handle that call needs: its
    /// `Compound` variant is exactly the three fields (`kind`, `name`,
    /// `arguments`) the contract and the acceptance tests fix, with no room
    /// for a cached handle or a captured string alongside them. Correctness
    /// is pinned by tests, not by a panicking runtime check: the acceptance
    /// tests, the conformance ports and `api_theory_display.rs` check it for
    /// every term kind, including nested compounds and theory operators
    /// (`clingox/tests/api_theory_atoms.rs`,
    /// `clingox/tests/api_theory_display.rs`,
    /// `clingox/tests/conformance_pyclingo.rs`). In debug builds,
    /// `warn_if_display_disagrees_with_clingo` also cross-checks every
    /// resolved compound term against clingo's own `term_to_string`, once
    /// per resolution, and logs a warning rather than panicking if they ever
    /// disagree: a mismatch is a clingox bug, not something a caller did
    /// wrong, so it must never crash a user's own program.
    ///
    /// `Display` is total: `Compound`'s fields are public (RULES §4 lets a
    /// convenience add validation, but never at the cost of a public field a
    /// documented contract fixes; see the type's own doc comment), so a
    /// caller can build a value clingo itself never produces, such as a
    /// `Function` with `name: None`, or a `Number`/`Symbol` `kind` on a
    /// `Compound`. Library code must not panic on caller-supplied input
    /// (RULES 11.2), so both cases print through the same fallback as an
    /// ordinary or operator function term, with an empty name standing in
    /// for the missing one. Nothing states what that fallback text must be:
    /// clingo never produces the value that reaches it, so no oracle exists
    /// to match.
    ///
    /// **Iterative:**
    /// recursing into an argument's own `Display` (`write!(f, "{argument}")`)
    /// once per level overflows a real stack at a few hundred to a few
    /// thousand levels. Instead, this expands one node at a time onto an
    /// explicit, heap-allocated stack of literal text and not-yet-printed
    /// subterms, in the exact order the recursive version above would have
    /// written them (each node's own text and its children's `Item::Term`
    /// placeholders are queued in left-to-right order, then pushed onto the
    /// stack reversed, so popping the stack reproduces that same order), so
    /// the native call stack never grows with the term's depth.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// One unit of work for the iterative renderer: a subterm still to
        /// print, or literal text to write as-is.
        enum Item<'a, 'c> {
            Term(&'a TheoryTerm<'c>),
            Text(&'c str),
        }

        let mut stack = vec![Item::Term(self)];
        while let Some(item) = stack.pop() {
            match item {
                Item::Text(text) => f.write_str(text)?,
                Item::Term(TheoryTerm::Number(n)) if *n < 0 => write!(f, "({n})")?,
                Item::Term(TheoryTerm::Number(n)) => write!(f, "{n}")?,
                Item::Term(TheoryTerm::Symbol(s)) => write!(f, "{s}")?,
                Item::Term(TheoryTerm::Compound {
                    kind:
                        kind @ (TheoryTermKind::Tuple | TheoryTermKind::List | TheoryTermKind::Set),
                    arguments,
                    ..
                }) => {
                    let (open, close) = match kind {
                        TheoryTermKind::Tuple => ("(", ")"),
                        TheoryTermKind::List => ("[", "]"),
                        TheoryTermKind::Set => ("{", "}"),
                        TheoryTermKind::Function
                        | TheoryTermKind::Number
                        | TheoryTermKind::Symbol => {
                            unreachable!("matched above")
                        }
                    };
                    let mut emit = vec![Item::Text(open)];
                    for (index, argument) in arguments.iter().enumerate() {
                        if index > 0 {
                            emit.push(Item::Text(","));
                        }
                        emit.push(Item::Term(argument));
                    }
                    if *kind == TheoryTermKind::Tuple && arguments.len() == 1 {
                        emit.push(Item::Text(","));
                    }
                    emit.push(Item::Text(close));
                    stack.extend(emit.into_iter().rev());
                }
                // `Function`, the only kind meant to reach here, and, as a
                // non-panicking fallback, a `Number` or `Symbol` `kind` a
                // caller built directly (see the doc comment above): both
                // print as an ordinary or operator function term. `name` is
                // `None` only for such a caller-built value; clingo never
                // omits a function's name, so the empty string stands in for
                // it rather than panicking.
                Item::Term(TheoryTerm::Compound {
                    name, arguments, ..
                }) => {
                    let name = name.unwrap_or("");
                    let mut emit = Vec::new();
                    if let Some(op) = operator_form(name, arguments.len()) {
                        emit.push(Item::Text("("));
                        if arguments.len() <= 1 {
                            emit.push(Item::Text(op));
                        }
                        for (index, argument) in arguments.iter().enumerate() {
                            if index > 0 {
                                emit.push(Item::Text(op));
                            }
                            emit.push(Item::Term(argument));
                        }
                        emit.push(Item::Text(")"));
                    } else {
                        emit.push(Item::Text(name));
                        emit.push(Item::Text("("));
                        for (index, argument) in arguments.iter().enumerate() {
                            if index > 0 {
                                emit.push(Item::Text(","));
                            }
                            emit.push(Item::Term(argument));
                        }
                        emit.push(Item::Text(")"));
                    }
                    stack.extend(emit.into_iter().rev());
                }
            }
        }
        Ok(())
    }
}

/// A view of the theory atoms of a control's current grounding, in clingo's
/// order.
///
/// It borrows the control (DESIGN S5), so the control cannot ground or solve
/// while one is alive. [`Control::theory_atoms`] makes one.
///
/// # Examples
///
/// ```
/// use clingox::{Control, Part};
///
/// let mut ctl = Control::new()?;
/// ctl.add_base("#theory t { term { }; &a/0 : term, head }. &a { 1 }.")?;
/// ctl.ground(&[Part::base()])?;
/// let atoms = ctl.theory_atoms()?;
/// assert_eq!(atoms.len()?, 1);
/// assert!(!atoms.is_empty()?);
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct TheoryAtoms<'c> {
    control: ErrorSink<'c>,
    atoms: raw::Theory<'c>,
}

impl<'c> TheoryAtoms<'c> {
    /// Builds one from an already-obtained raw view, for a source other than
    /// [`Control::theory_atoms`]: [`PropagateInit::theory_atoms`] has no
    /// `Control` to poison through (`ErrorSink::None`), only the raw atoms view
    /// from `clingo_propagate_init_theory_atoms`.
    ///
    /// [`PropagateInit::theory_atoms`]: crate::propagate::PropagateInit::theory_atoms
    pub(crate) fn from_parts(control: ErrorSink<'c>, atoms: raw::Theory<'c>) -> TheoryAtoms<'c> {
        TheoryAtoms { control, atoms }
    }
}

impl ScopedControl<'_> {
    /// A view of the theory atoms of the current grounding.
    ///
    /// A search left open by a forgotten handle is closed first (DESIGN S4).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("a. b :- a.")?;
    /// ctl.ground(&[Part::base()])?;
    /// assert!(ctl.theory_atoms()?.is_empty()?);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn theory_atoms(&self) -> Result<TheoryAtoms<'_>> {
        let atoms = self.core.observed(
            || "reading the theory atoms".to_owned(),
            raw::ControlHandle::theory_atoms,
        )?;
        Ok(TheoryAtoms {
            control: ErrorSink::Control(&self.core),
            atoms,
        })
    }
}

impl<'c> TheoryAtoms<'c> {
    /// The number of theory atoms.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    pub fn len(&self) -> Result<usize> {
        self.read("counting the theory atoms", raw::Theory::size)
    }

    /// Whether there are no theory atoms.
    ///
    /// # Errors
    ///
    /// As [`TheoryAtoms::len`].
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Iterates over every theory atom, in clingo's order (consecutive ids
    /// from zero). Each item is read as the iterator advances; an error is
    /// yielded as an `Err` item, after which the iterator ends.
    #[expect(
        clippy::iter_without_into_iter,
        reason = "an `IntoIterator for &TheoryAtoms` would make `for x in atoms.iter()` (used \
                  throughout the acceptance and conformance tests) trigger \
                  clippy::explicit_iter_loop instead, in test files this must not \
                  edit; SymbolicAtoms can offer both because none of its own tests iterate this \
                  way"
    )]
    pub fn iter(&self) -> TheoryAtomIter<'c> {
        TheoryAtomIter {
            control: self.control,
            atoms: self.atoms,
            state: TheoryAtomIterState::Start,
        }
    }

    /// The kind of a theory term.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Logic`] if `term` is out of range for the current
    ///   grounding, which also poisons the control (`clingo.h`'s `TheoryAtoms`
    ///   group resets term ids after each solve; see [`Id`]);
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::Unknown`] if clingo reports a term kind this version of
    ///   clingox does not know, or if clingo runs out of memory.
    pub fn term_kind(&self, term: Id) -> Result<TheoryTermKind> {
        self.read("reading a theory term's kind", |atoms| {
            kind_from_raw(atoms.term_type(term)?)
        })
    }

    /// The theory term for `term`, fully resolved: a compound term's
    /// arguments are themselves resolved [`TheoryTerm`] values.
    ///
    /// # Errors
    ///
    /// As [`TheoryAtoms::term_kind`]; additionally [`ErrorKind::Nul`] if a
    /// symbolic constant term's name cannot be turned back into a [`Symbol`]
    /// (clingo's own parser accepted it, so this should not happen for a term
    /// read from a real grounding).
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, TheoryTerm};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#theory t { term { }; &a/0 : term, head }. &a { 1 }.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let atoms = ctl.theory_atoms()?;
    /// let atom = atoms.iter().next().unwrap()?;
    /// let term_id = atom.elements()?[0].tuple()?[0];
    /// assert_eq!(atoms.term(term_id)?, TheoryTerm::Number(1));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn term(&self, term: Id) -> Result<TheoryTerm<'c>> {
        self.read("reading a theory term", |atoms| resolve_term(atoms, term))
    }

    fn read<T>(&self, context: &str, f: impl FnOnce(raw::Theory<'c>) -> Result<T>) -> Result<T> {
        self.control
            .observed(|| context.to_owned(), || f(self.atoms))
    }
}

/// The symbol a theory term of kind `Symbol` stands for.
///
/// clingo stores a string of the program, `&a { "str" }`, as a symbol term
/// whose name is the string in quotes, escaped the way clingo prints strings:
/// `\n`, `\\` and `\"`. A name that is not an identifier cannot start with a
/// quote, so a quoted name is a string and anything else is a constant.
fn symbol_term(name: &str) -> Result<Symbol> {
    let Some(inner) = name
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return Symbol::function(name, &[]);
    };
    let mut text = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => text.push('\n'),
            Some(other) => text.push(other),
            None => text.push('\\'),
        }
    }
    Symbol::string(&text)
}

/// Resolves a term id into a [`TheoryTerm`], walking into a compound term's
/// arguments. Free-standing (not a method) because each step walks the same
/// `atoms` handle for a different id, not through a fresh borrow of
/// `&TheoryAtoms`.
///
/// **Iterative:** a
/// version that called itself once per level of a compound term's own
/// arguments overflowed a real stack at a few hundred to a few thousand
/// levels (each recursive call to a fallible clingo-calling function is much
/// heavier on the stack than, say, a plain tree walk). This instead expands
/// one id at a time onto an explicit, heap-allocated stack: a leaf resolves
/// and is pushed onto `built` at once; a compound pushes a `Build` step
/// naming its own kind, name and arity, then its own children's ids (in
/// reverse, so they come off, and so resolve, in their original left-to-
/// right order), and only turns into a `TheoryTerm::Compound` once every one
/// of those children has finished and is waiting on `built`.
///
/// **The debug cross-check runs once, at the root, not at every level**:
/// the old, recursive version
/// called `warn_if_display_disagrees_with_clingo` once per compound node,
/// each call itself printing that whole subtree, for O(n^2) total work on a
/// term with n nodes; checking the fully resolved term once, here, is O(n).
fn resolve_term(atoms: raw::Theory<'_>, term: Id) -> Result<TheoryTerm<'_>> {
    enum Step<'c> {
        Visit(Id),
        Build {
            kind: TheoryTermKind,
            name: Option<&'c str>,
            arity: usize,
        },
    }

    let mut stack = vec![Step::Visit(term)];
    let mut built: Vec<TheoryTerm<'_>> = Vec::new();
    while let Some(step) = stack.pop() {
        match step {
            Step::Visit(id) => {
                let kind = kind_from_raw(atoms.term_type(id)?)?;
                match kind {
                    TheoryTermKind::Number => {
                        built.push(TheoryTerm::Number(atoms.term_number(id)?));
                    }
                    TheoryTermKind::Symbol => {
                        let name = atoms.term_name(id)?;
                        built.push(TheoryTerm::Symbol(symbol_term(name)?));
                    }
                    TheoryTermKind::Function
                    | TheoryTermKind::Tuple
                    | TheoryTermKind::List
                    | TheoryTermKind::Set => {
                        let name = if kind == TheoryTermKind::Function {
                            Some(atoms.term_name(id)?)
                        } else {
                            None
                        };
                        let arguments = atoms.term_arguments(id)?;
                        stack.push(Step::Build {
                            kind,
                            name,
                            arity: arguments.len(),
                        });
                        for &argument in arguments.iter().rev() {
                            stack.push(Step::Visit(argument));
                        }
                    }
                }
            }
            Step::Build { kind, name, arity } => {
                // As `Clone`'s own worklist above: the last `arity` items on
                // `built` are this compound's own arguments, already in
                // left-to-right order.
                let split_at = built.len() - arity;
                let arguments = built.split_off(split_at);
                built.push(TheoryTerm::Compound {
                    kind,
                    name,
                    arguments,
                });
            }
        }
    }
    let value = built
        .pop()
        .expect("the worklist always finishes with exactly the one resolved term");
    // Skipped for a term deep enough to risk clingo's own stack in the
    // cross-check itself (see `MAX_CROSS_CHECK_DEPTH`'s own doc comment);
    // checking only the root (above) already keeps this from ever running
    // more than once per `resolve_term` call, whatever the term's shape.
    if matches!(value, TheoryTerm::Compound { .. }) && term_depth(&value) <= MAX_CROSS_CHECK_DEPTH {
        warn_if_display_disagrees_with_clingo(atoms, term, &value);
    }
    Ok(value)
}

/// A term's own nesting depth (a leaf is depth 1), walked iteratively for
/// the same reason every other traversal in this module is:
/// only [`resolve_term`]'s own cross-check bound needs this, so it is not
/// computed for every node while resolving, only once, on the finished
/// tree, right before deciding whether to run that check at all.
fn term_depth(value: &TheoryTerm<'_>) -> u32 {
    let mut deepest = 0;
    let mut stack = vec![(value, 1)];
    while let Some((term, depth)) = stack.pop() {
        deepest = deepest.max(depth);
        if deepest > MAX_CROSS_CHECK_DEPTH {
            // No need to keep walking a term already known too deep to
            // check; the exact depth past this point is never read.
            return deepest;
        }
        if let TheoryTerm::Compound { arguments, .. } = term {
            stack.extend(arguments.iter().map(|argument| (argument, depth + 1)));
        }
    }
    deepest
}

/// The deepest term [`warn_if_display_disagrees_with_clingo`] still cross-
/// checks against clingo's own `term_to_string`.
///
/// Checked directly, in this worktree, on a 2 MB thread in a debug build
/// (the shape `deep_theory_terms.rs`'s own acceptance test uses):
/// `clingo_theory_atoms_term_to_string` itself -- gringo's own
/// `TheoryData::printTerm`, not any of clingox's own code -- resolves a
/// left-nested chain up to depth 20000 without trouble and overflows
/// somewhere before depth 50000. clingox's own resolving, `Display`,
/// `Clone`, `PartialEq` and `Drop` are iterative and bounded by none of
/// this: verified separately, on the same thread shape, with
/// this cross-check disabled entirely, that they resolve, print, clone,
/// compare and drop a depth-100000 term without trouble. This bound keeps
/// the cross-check's value for every realistic term (`docs/dev/UPSTREAM-
/// ISSUES.md` U22's own huge-literal fixtures and every test in this
/// crate stay orders of magnitude below it) while never handing clingo's
/// own recursive printer a term deep enough to risk the same crash this
/// decision fixes on clingox's side of the line -- a hostile aspif file's
/// deeply nested theory term is exactly the
/// input this bound is for.
const MAX_CROSS_CHECK_DEPTH: u32 = 1000;

/// In debug builds, checks that clingox's own structural
/// [`Display`](fmt::Display) for `value` agrees with clingo's own
/// `clingo_theory_atoms_term_to_string` for the same term, and logs a warning
/// (target `clingox`) instead of panicking if they disagree.
///
/// [`TheoryTerm::Compound`]'s three fields (`kind`, `name`, `arguments`) are
/// exactly what the contract and the acceptance tests fix, with no room to
/// cache clingo's text alongside them (a private field would still force every
/// external match on `Compound` to add `..`, which the acceptance and
/// conformance tests do not all do, and which must not be added to them).
/// Logging rather than asserting means a clingox bug, or a change in gringo's
/// undocumented printer (`libgringo/src/output/theory.cc`,
/// `TheoryData::printTerm`) in a future clingo release, is surfaced instead of
/// silently wrong, but never crashes a user's own program the way a panicking
/// `debug_assert!` would. It costs nothing in a release build, as
/// `debug_assert!` itself would (RULES 11.3).
#[cfg(debug_assertions)]
fn warn_if_display_disagrees_with_clingo(atoms: raw::Theory<'_>, term: Id, value: &TheoryTerm<'_>) {
    let Ok(expected) = atoms.term_to_string(term) else {
        return;
    };
    let displayed = value.to_string();
    if displayed != expected {
        log_display_disagreement(term, &displayed, &expected);
    }
}

#[cfg(not(debug_assertions))]
fn warn_if_display_disagrees_with_clingo(
    _atoms: raw::Theory<'_>,
    _term: Id,
    _value: &TheoryTerm<'_>,
) {
}

#[cfg(all(debug_assertions, feature = "log"))]
fn log_display_disagreement(term: Id, displayed: &str, expected: &str) {
    log::warn!(
        target: "clingox",
        "TheoryTerm's Display ({displayed:?}) disagrees with clingo's own \
         term_to_string ({expected:?}) for {term:?}; please report this as a clingox bug"
    );
}

#[cfg(all(debug_assertions, not(feature = "log")))]
fn log_display_disagreement(_term: Id, _displayed: &str, _expected: &str) {}

impl fmt::Debug for TheoryAtoms<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TheoryAtoms").finish_non_exhaustive()
    }
}

/// An iterator over theory atoms, from [`TheoryAtoms::iter`]. It borrows the
/// control.
pub struct TheoryAtomIter<'c> {
    control: ErrorSink<'c>,
    atoms: raw::Theory<'c>,
    state: TheoryAtomIterState,
}

#[derive(Clone, Copy)]
enum TheoryAtomIterState {
    /// Not started: the first call to `next` asks clingo for the count.
    Start,
    Running {
        next: u32,
        len: usize,
    },
    Done,
}

impl<'c> TheoryAtomIter<'c> {
    fn step(&mut self) -> Result<Option<TheoryAtom<'c>>> {
        let (next, len) = match self.state {
            TheoryAtomIterState::Done => return Ok(None),
            TheoryAtomIterState::Start => (0, self.atoms.size()?),
            TheoryAtomIterState::Running { next, len } => (next, len),
        };
        if usize::try_from(next).unwrap_or(usize::MAX) >= len {
            self.state = TheoryAtomIterState::Done;
            return Ok(None);
        }
        self.state = TheoryAtomIterState::Running {
            next: next + 1,
            len,
        };
        Ok(Some(TheoryAtom {
            control: self.control,
            atoms: self.atoms,
            id: Id::from_raw(next),
        }))
    }
}

impl<'c> Iterator for TheoryAtomIter<'c> {
    type Item = Result<TheoryAtom<'c>>;

    fn next(&mut self) -> Option<Result<TheoryAtom<'c>>> {
        let control = self.control;
        let item = control
            .refusal()
            .and_then(|()| self.step())
            .map_err(|err| control.note(err.context("reading the theory atoms")));
        if item.is_err() {
            self.state = TheoryAtomIterState::Done;
        }
        item.transpose()
    }
}

impl std::iter::FusedIterator for TheoryAtomIter<'_> {}

impl fmt::Debug for TheoryAtomIter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TheoryAtomIter").finish_non_exhaustive()
    }
}

/// One theory atom of the grounding.
///
/// It borrows the control (DESIGN S5), through the [`TheoryAtoms`] view it
/// came from.
pub struct TheoryAtom<'c> {
    control: ErrorSink<'c>,
    atoms: raw::Theory<'c>,
    id: Id,
}

impl<'c> TheoryAtom<'c> {
    /// The atom's own id.
    ///
    /// Infallible: the id is already known once the atom is constructed.
    /// Mainly useful to confirm that ids are reused after a solve and a fresh
    /// grounding (`clingo.h`'s `TheoryAtoms` group; see [`Id`]).
    pub fn id(&self) -> Id {
        self.id
    }

    /// The atom's theory term: for `&a { ... }`, the term `a`.
    ///
    /// # Errors
    ///
    /// As [`TheoryAtoms::term`], but never [`ErrorKind::Logic`]: this atom's
    /// own term id is always in range for the grounding it came from.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, TheoryTerm};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#theory t { term { }; &a/0 : term, head }. &a { 1 }.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let atoms = ctl.theory_atoms()?;
    /// let atom = atoms.iter().next().unwrap()?;
    /// let TheoryTerm::Symbol(name) = atom.term()? else {
    ///     panic!("expected the symbolic term `a`");
    /// };
    /// assert_eq!(name.name(), Some("a"));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn term(&self) -> Result<TheoryTerm<'c>> {
        self.read("reading a theory atom's term", |atoms| {
            let term = atoms.atom_term(self.id)?;
            resolve_term(atoms, term)
        })
    }

    /// The atom's elements: for `&a { 1; 2,3: a,b }.`, two elements.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#theory t { term { }; &a/0 : term, head }. &a { 1; 2 }.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let atoms = ctl.theory_atoms()?;
    /// let atom = atoms.iter().next().unwrap()?;
    /// assert_eq!(atom.elements()?.len(), 2);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn elements(&self) -> Result<Vec<TheoryElement<'c>>> {
        self.read("reading a theory atom's elements", |atoms| {
            Ok(atoms
                .atom_elements(self.id)?
                .iter()
                .map(|&id| TheoryElement {
                    control: self.control,
                    atoms,
                    id,
                })
                .collect())
        })
    }

    /// The atom's guard, a connective and a term: for `&a { } = 1.`,
    /// `Some(("=", TheoryTerm::Number(1)))`. `None` if the atom has no guard.
    ///
    /// The connective's string has the same solve-step-scoped lifetime
    /// `clingo.h` documents for it (unlike [`Symbol::name`]'s `'static` one):
    /// [`TheoryAtoms<'c>`] borrows `&'c Control`, and grounding or solving
    /// again needs `&mut Control` (S5), so `'c` cannot outlive the step this
    /// string is valid for.
    ///
    /// # Errors
    ///
    /// As [`TheoryAtoms::term`], but never [`ErrorKind::Logic`], as
    /// [`TheoryAtom::term`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, TheoryTerm};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base("#theory t { term { }; &a/0 : term, {=}, term, head }. &a { } = 1.")?;
    /// ctl.ground(&[Part::base()])?;
    /// let atoms = ctl.theory_atoms()?;
    /// let atom = atoms.iter().next().unwrap()?;
    /// let (connective, term) = atom.guard()?.expect("the atom has a guard");
    /// assert_eq!(connective, "=");
    /// assert_eq!(term, TheoryTerm::Number(1));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn guard(&self) -> Result<Option<(&'c str, TheoryTerm<'c>)>> {
        self.read("reading a theory atom's guard", |atoms| {
            if !atoms.atom_has_guard(self.id)? {
                return Ok(None);
            }
            let (connective, term) = atoms.atom_guard(self.id)?;
            Ok(Some((connective, resolve_term(atoms, term)?)))
        })
    }

    /// The atom's program literal, or `None` for a `directive`-role atom, which
    /// is never part of a rule body or head and so gets no literal at all
    /// (clingo's own `0` sentinel, checked directly against clingo 5.8.2).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, Part, TheoryTerm};
    ///
    /// let mut ctl = Control::new()?;
    /// ctl.add_base(
    ///     "#theory t { term { }; &a/0 : term, head; \
    ///      &b/0 : term, {=}, term, directive }. \
    ///      &a { 1 }. &b { } = 1.",
    /// )?;
    /// ctl.ground(&[Part::base()])?;
    /// let atoms = ctl.theory_atoms()?;
    /// for item in atoms.iter() {
    ///     let atom = item?;
    ///     let has_literal = atom.literal()?.is_some();
    ///     match atom.term()? {
    ///         TheoryTerm::Symbol(s) if s.name() == Some("a") => assert!(has_literal),
    ///         TheoryTerm::Symbol(s) if s.name() == Some("b") => assert!(!has_literal),
    ///         other => panic!("unexpected top-level term {other:?}"),
    ///     }
    /// }
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn literal(&self) -> Result<Option<ProgramLiteral>> {
        self.read("reading a theory atom's literal", |atoms| {
            Ok(ProgramLiteral::from_raw(atoms.atom_literal(self.id)?))
        })
    }

    fn read<T>(&self, context: &str, f: impl FnOnce(raw::Theory<'c>) -> Result<T>) -> Result<T> {
        self.control
            .observed(|| context.to_owned(), || f(self.atoms))
    }

    /// As [`Display`](fmt::Display), but fallible: `Display` itself never
    /// fails (see below), so this is how a caller who wants to know
    /// *why* printing failed, rather than just seeing a placeholder, still
    /// can.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    pub fn try_to_string(&self) -> Result<String> {
        self.read("printing a theory atom", |atoms| {
            atoms.atom_to_string(self.id)
        })
    }
}

impl fmt::Display for TheoryAtom<'_> {
    /// Never returns [`fmt::Error`] for a clingo failure:
    /// `ToString::to_string`'s blanket implementation `expect`s a `Display`
    /// implementation never to fail, so mapping a clingo failure straight to
    /// `fmt::Error` made `to_string()` panic instead of ever giving the caller
    /// an `Error` (for instance printing a theory atom read from a control that
    /// failed and poisoned meanwhile). A failure instead writes a placeholder,
    /// `<error: ...>`, the same shape [`fmt::Debug`] below already uses for an
    /// unreadable field; [`TheoryAtom::try_to_string`] is the fallible way to
    /// print, for a caller that wants the `Error` itself rather than this
    /// placeholder.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_to_string() {
            Ok(text) => f.write_str(&text),
            Err(err) => write!(f, "<error: {err}>"),
        }
    }
}

impl fmt::Debug for TheoryAtom<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// A value, or the error that kept clingo from reporting it.
        struct Read<T>(Result<T>);
        impl<T: fmt::Debug> fmt::Debug for Read<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match &self.0 {
                    Ok(value) => value.fmt(f),
                    Err(err) => write!(f, "<{err}>"),
                }
            }
        }
        f.debug_struct("TheoryAtom")
            .field("id", &self.id)
            .field("term", &Read(self.term()))
            .field("guard", &Read(self.guard()))
            .field("literal", &Read(self.literal()))
            .finish_non_exhaustive()
    }
}

/// One element of a theory atom, such as `2,3: a,b` in `&a { 2,3: a,b }.`.
///
/// It borrows the control (DESIGN S5), through the [`TheoryAtoms`] view it
/// came from.
pub struct TheoryElement<'c> {
    control: ErrorSink<'c>,
    atoms: raw::Theory<'c>,
    id: Id,
}

impl<'c> TheoryElement<'c> {
    /// The element's tuple: for `2,3: a,b`, the term ids of `2` and `3`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Poisoned`] if an earlier error poisoned the control;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    pub fn tuple(&self) -> Result<&'c [Id]> {
        self.read("reading a theory element's tuple", |atoms| {
            atoms.element_tuple(self.id)
        })
    }

    /// The element's condition: for `2,3: a,b`, the program literals of `a`
    /// and `b`.
    ///
    /// Unlike [`TheoryElement::tuple`], this is an owned `Vec`, not a borrow
    /// of the control: clingo fills every element's condition into one
    /// scratch buffer that it reuses and reallocates on every call, for any
    /// element (UPSTREAM-ISSUES U24), so clingox copies the literals out
    /// before returning them.
    ///
    /// # Errors
    ///
    /// As [`TheoryElement::tuple`].
    pub fn condition(&self) -> Result<Vec<ProgramLiteral>> {
        self.read("reading a theory element's condition", |atoms| {
            atoms.element_condition(self.id)
        })
    }

    /// The id of the element's condition, or `None` if the element has no
    /// condition, such as the bare `1` in `&a { 1 }.` (clingo's own `0`
    /// sentinel for "no condition"). Unlike [`TheoryElement::condition`], a
    /// `Some` value is not necessarily an aspif literal: it can be mapped to a
    /// solver literal only from inside a propagator
    /// (`clingo_propagate_init_solver_literal`), and, checked directly against
    /// clingo 5.8.2, it is ordinarily a real id from clasp's own body-id range
    /// (`2^28` and above) -- well outside [`ProgramLiteral::MAX_MAGNITUDE`],
    /// which is exactly why this does not reject it the way
    /// [`ProgramLiteral::from_raw`] would (`MAX_MAGNITUDE` excludes that range
    /// for an ordinary literal, which this is not one of).
    ///
    /// # Errors
    ///
    /// As [`TheoryElement::tuple`].
    pub fn condition_id(&self) -> Result<Option<ProgramLiteral>> {
        self.read("reading a theory element's condition id", |atoms| {
            let raw = atoms.element_condition_id(self.id)?;
            // `from_nonzero` returns `None` for exactly clingo's own `0`
            // sentinel ("no condition"); nothing else about `raw` can be
            // invalid once the magnitude check is out of the picture.
            Ok(ProgramLiteral::from_nonzero(raw))
        })
    }

    fn read<T>(&self, context: &str, f: impl FnOnce(raw::Theory<'c>) -> Result<T>) -> Result<T> {
        self.control
            .observed(|| context.to_owned(), || f(self.atoms))
    }

    /// As [`TheoryAtom::try_to_string`].
    ///
    /// # Errors
    ///
    /// As [`TheoryAtom::try_to_string`].
    pub fn try_to_string(&self) -> Result<String> {
        self.read("printing a theory element", |atoms| {
            atoms.element_to_string(self.id)
        })
    }
}

impl fmt::Display for TheoryElement<'_> {
    /// As [`TheoryAtom`]'s `Display`: never returns [`fmt::Error`] for a
    /// clingo failure; [`TheoryElement::try_to_string`] is the
    /// fallible way to print.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_to_string() {
            Ok(text) => f.write_str(&text),
            Err(err) => write!(f, "<error: {err}>"),
        }
    }
}

impl fmt::Debug for TheoryElement<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// As `TheoryAtom`'s `Debug`.
        struct Read<T>(Result<T>);
        impl<T: fmt::Debug> fmt::Debug for Read<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match &self.0 {
                    Ok(value) => value.fmt(f),
                    Err(err) => write!(f, "<{err}>"),
                }
            }
        }
        f.debug_struct("TheoryElement")
            .field("id", &self.id)
            .field("tuple", &Read(self.tuple()))
            .field("condition", &Read(self.condition()))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use crate::Control;
    use crate::control::Part;

    /// `clingo_theory_atoms_term_to_string`(`_size`) has no call site in the
    /// public API: [`TheoryTerm`]'s `Display` is computed structurally
    /// instead (see its rustdoc for why a resolved term cannot hold the
    /// atoms handle that call needs). This test is what keeps the pair
    /// itself wrapped and tested (RULES 3): it reads `TheoryAtoms`'s own
    /// private field directly, in the same module, to call the raw wrapper
    /// on a representative term of every kind (a nested binary operator
    /// inside a function, a list, a singleton tuple, a set and a unary
    /// operator) and checks clingox's `Display` against it.
    ///
    /// Oracle: Python module `clingo` 5.8.2, 2026-09-27 (checked directly):
    /// `#theory t { term { + : 1, binary, left; - : 4, unary }; \
    /// &a/0 : term, head }. &a { f(1+2), [1,a], (1,), {1}, -1 }.` has one
    /// element whose five terms print as `f((1+2))`, `[1,a]`, `(1,)`, `{1}`
    /// and `(-1)`.
    #[test]
    #[cfg_attr(miri, ignore = "calls into clingo")]
    fn displayed_compounds_match_clingos_own_term_to_string() {
        let mut ctl = Control::new().unwrap();
        ctl.add_base(
            "#theory t { term { + : 1, binary, left; - : 4, unary }; \
             &a/0 : term, head }. \
             &a { f(1+2), [1,a], (1,), {1}, -1 }.",
        )
        .unwrap();
        ctl.ground(&[Part::base()]).unwrap();
        let atoms = ctl.theory_atoms().unwrap();
        let atom = atoms.iter().next().unwrap().unwrap();
        let tuple = atom.elements().unwrap()[0].tuple().unwrap().to_vec();
        assert_eq!(tuple.len(), 5);
        for term_id in tuple {
            let displayed = atoms.term(term_id).unwrap().to_string();
            // Read `TheoryAtoms`'s own private field: this test lives in the
            // same module, so it can reach the raw handle without a public
            // accessor clingox would otherwise never need.
            let from_clingo = atoms.atoms.term_to_string(term_id).unwrap();
            assert_eq!(displayed, from_clingo, "term {term_id:?}");
        }
    }
}

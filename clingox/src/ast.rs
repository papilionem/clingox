//! Abstract syntax trees: clingo's own representation of a program.
//!
//! [`parse_string`] and [`parse_files`] turn program text into a stream of
//! [`Ast`] nodes, one per top-level statement. An `Ast`'s shape is read
//! through [`Ast::ast_type`] and the typed accessors ([`Ast::number`],
//! [`Ast::symbol`], [`Ast::span`], [`Ast::string`], [`Ast::ast`],
//! [`Ast::optional_ast`], and the array accessors), each keyed by an
//! [`Attribute`].
//!
//! Nodes are built with one generated constructor per node type
//! ([`rule`], [`function`], [`variable`], ...) and edited in place with the
//! setters ([`Ast::set_ast`], [`Ast::set_string`], [`Ast::insert_ast_at`],
//! ...). Clones of an `Ast` share one node, so an edit through one is seen
//! through every other; [`Ast::copy`] and [`Ast::deep_copy`] give independent
//! top nodes. A [`Visitor`] rewrites a whole tree without touching the
//! original.
//!
//! ```
//! use clingox::ast::{self, AstType, Attribute};
//!
//! let mut rules = Vec::new();
//! ast::parse_string("a. b :- a.", |node| {
//!     if node.ast_type() == AstType::Rule {
//!         rules.push(node);
//!     }
//!     Ok(())
//! })?;
//! assert_eq!(rules.len(), 2);
//! assert_eq!(rules[0].to_string(), "a.");
//! assert_eq!(rules[1].ast(Attribute::Head)?.to_string(), "b");
//! # Ok::<(), clingox::Error>(())
//! ```
//!
//! Building a rule and editing it:
//!
//! ```
//! use clingox::Symbol;
//! use clingox::ast::{self, Attribute, LiteralSign, Span};
//!
//! let span = Span::new("<example>", 1, 1, "<example>", 1, 8)?;
//! let literal = |name: &str| -> clingox::Result<ast::Ast> {
//!     let term = ast::symbolic_term(&span, Symbol::function(name, &[])?)?;
//!     let atom = ast::symbolic_atom(&term)?;
//!     ast::literal(&span, LiteralSign::NoSign, &atom)
//! };
//! let rule = ast::rule(&span, &literal("h")?, &[literal("a")?])?;
//! assert_eq!(rule.to_string(), "h :- a.");
//!
//! // The setters take `&self`: a clone sees the change too.
//! let alias = rule.clone();
//! alias.push_ast(Attribute::Body, &literal("b")?)?;
//! assert_eq!(rule.to_string(), "h :- a; b.");
//! # Ok::<(), clingox::Error>(())
//! ```

use std::cmp::Ordering;
use std::collections::HashSet;
use std::ffi::CString;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::OnceLock;

use crate::Symbol;
use crate::error::{Error, ErrorKind, Result};
use crate::raw::{self, RawAst};

#[rustfmt::skip]
mod generated;
mod program_builder;
mod unpool;

pub use program_builder::ProgramBuilder;
pub use unpool::Unpool;

// Every constructor, the nine enum types, `Visitor` and `walk` are generated
// from clingo's constructor table (`cargo xtask ast-codegen`). No constructor
// checks for a cycle: the node it returns is new, so nothing can reach it yet.
pub use generated::*;
use generated::{NumberDomain, child_attributes, number_domain};

/// A node in a clingo abstract syntax tree.
///
/// `Ast` is reference counted (DESIGN S12): [`Clone`] acquires a new
/// reference to the same underlying node, and [`Drop`] releases one; the
/// node itself is freed only when its last `Ast` is dropped. Every clone
/// shares one node, exactly as two Python references to the same pyclingo
/// `AST` object do: mutating through one clone is visible through
/// every other. [`Ast::deep_copy`] gives an independent node instead.
///
/// The setters ([`Ast::set_ast`] and the rest) take `&self`, as pyclingo's
/// attribute assignment does. Every getter returns an owned value (a
/// `String`, a `Vec`, a new `Ast` handle), never a borrow into the node, so a
/// value read before an edit stays valid after it.
///
/// `Ast` is **not** [`Send`] or [`Sync`]: clingo's own reference count is a
/// plain `unsigned`, incremented and decremented with no lock or atomic
/// instruction (`astv2.hh:125`, `astv2.cc:238-243`). Two `Ast` clones of the
/// same node cloned or dropped concurrently from different threads would
/// race on that field.
///
/// [`PartialEq`]/[`Eq`]/[`Ord`]/[`Hash`] compare and order nodes
/// structurally, **ignoring their `location` attribute**: two nodes parsed
/// from the same text at different positions compare equal. [`Ast::ptr_eq`]
/// compares identity instead, which is what the node-rewriting [`Visitor`]
/// uses to decide whether a child actually changed.
///
/// Because a node can change through any clone, an `Ast` used as a key of a
/// `HashMap` or `HashSet` (or in a `BTreeMap`) must not be edited while it is
/// in the collection: its hash and order change with its content, which
/// breaks the collection's invariants. That is a logic error in the
/// collection, never undefined behavior, as with any key that has interior
/// mutability.
///
/// # The reference count
///
/// The counter also limits cloning. Every clone, and every
/// getter that returns a node ([`Ast::ast`], [`Ast::optional_ast`],
/// [`Ast::ast_at`]), adds one to a 32-bit count inside the node, and
/// `std::mem::forget` on `u32::MAX` such handles is safe Rust. Against the
/// vendored clingo, which clingox patches (UPSTREAM-ISSUES U47), the next
/// increment aborts the process, as `Rc` does on overflow. A build against a
/// system clingo has no such patch: the count wraps to zero and the next
/// release frees a node that handles still point to. Reaching that takes
/// billions of leaked handles, but it is a use after free, so do not leak
/// handles in bulk with a system library.
///
/// # Deeply nested trees
///
/// Releasing a node, printing it ([`Display`](fmt::Display)),
/// [`Ast::deep_copy`], `==`, `cmp` and `hash` all recurse once per level of
/// nesting inside clingo (its own C++ code; clingox adds no recursion of
/// its own), so a very deep tree can overflow the thread's stack and abort
/// the process, `Drop` included. Parsing itself does not recurse: a
/// million-level term parses. Measured on a 2 MiB stack, dropping a node
/// aborts at roughly 9 000 levels of `f(f(...))` and 20 000 of `-(-(...))`.
/// Run such input on a thread with a large stack
/// ([`std::thread::Builder::stack_size`]). Ordinary programs are nowhere
/// near these depths.
pub struct Ast(RawAst);

// Every accessor's success and failure paths are exercised against real,
// parsed nodes in `clingox/tests/api_ast_values.rs` and
// `clingox/tests/api_ast_parse.rs`; `Ast` itself carries no unit tests here,
// matching `Signature`/`Symbol`'s own precedent of leaving live-clingo
// coverage entirely to the integration suite.

impl Ast {
    /// Wraps a node clingo already produced. The caller already owns the one
    /// reference this handle represents (clingo's own getters `incRef` the
    /// value before returning it, and the parse/unpool callbacks are
    /// acquired by the trampoline before this is called).
    pub(crate) fn from_raw(raw: RawAst) -> Ast {
        Ast(raw)
    }

    /// The underlying node, still owned by `self`.
    pub(crate) fn as_raw(&self) -> RawAst {
        self.0
    }

    /// Whether `self` and `other` are the same underlying node: identity,
    /// not the structural, location-ignoring comparison [`Ast::eq`] does.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast;
    ///
    /// let mut nodes = Vec::new();
    /// ast::parse_string("a.", |node| { nodes.push(node); Ok(()) })?;
    /// let clone = nodes[0].clone();
    /// assert!(nodes[0].ptr_eq(&clone));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    #[must_use]
    pub fn ptr_eq(&self, other: &Ast) -> bool {
        self.0 == other.0
    }

    /// A shallow copy: a new top node whose children are shared with `self`
    /// (`clingo_ast_copy`). A child read from the copy and from the
    /// original are [`Ast::ptr_eq`].
    ///
    /// A shallow copy shares its child nodes with the original: setting an
    /// attribute on a child reached through the copy changes the same child
    /// the original sees. Use [`Ast::deep_copy`] for an independent tree.
    /// The copy's own arrays are its own, though: replacing, inserting or
    /// deleting an element of the copy's `body` leaves the original's `body`
    /// alone.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    pub fn copy(&self) -> Result<Ast> {
        raw::ast_copy(self.0)
            .map(Ast::from_raw)
            .map_err(|e| e.context("copying an ast node"))
    }

    /// A deep copy: a new top node whose children are new nodes too
    /// (`clingo_ast_deep_copy`). A child read from the copy is never
    /// [`Ast::ptr_eq`] to the original's.
    ///
    /// # Errors
    ///
    /// As [`Ast::copy`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::Attribute;
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string("a.", |node| { nodes.push(node); Ok(()) })?;
    /// let rule = &nodes[1];
    /// let deep = rule.deep_copy()?;
    /// assert!(!rule.ptr_eq(&deep));
    /// assert!(!rule.ast(Attribute::Head)?.ptr_eq(&deep.ast(Attribute::Head)?));
    /// assert_eq!(*rule, deep, "still structurally equal");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn deep_copy(&self) -> Result<Ast> {
        raw::ast_deep_copy(self.0)
            .map(Ast::from_raw)
            .map_err(|e| e.context("deep-copying an ast node"))
    }

    /// The node's type.
    ///
    /// Cannot fail: `clingo_ast_get_type` only copies a field
    /// (`Model::number`'s own established "cannot fail in practice" pattern).
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::AstType;
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string("a.", |node| { nodes.push(node); Ok(()) })?;
    /// assert_eq!(nodes[0].ast_type(), AstType::Program);
    /// assert_eq!(nodes[1].ast_type(), AstType::Rule);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    #[must_use]
    pub fn ast_type(&self) -> AstType {
        raw::ast_type(self.0)
    }

    /// Whether this node carries `attribute`.
    ///
    /// Cannot fail, for the same reason [`Ast::ast_type`] cannot.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::Attribute;
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string("a.", |node| { nodes.push(node); Ok(()) })?;
    /// let rule = &nodes[1];
    /// assert!(rule.has_attribute(Attribute::Head));
    /// assert!(!rule.has_attribute(Attribute::Symbol));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    #[must_use]
    pub fn has_attribute(&self, attribute: Attribute) -> bool {
        raw::ast_has_attribute(self.0, attribute)
    }

    /// The kind of `attribute` on this node, or `None` if the node does not
    /// carry it.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{Attribute, AttributeType};
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string("a.", |node| { nodes.push(node); Ok(()) })?;
    /// let rule = &nodes[1];
    /// assert_eq!(rule.attribute_type(Attribute::Body), Some(AttributeType::AstArray));
    /// assert_eq!(rule.attribute_type(Attribute::Symbol), None);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    #[must_use]
    pub fn attribute_type(&self, attribute: Attribute) -> Option<AttributeType> {
        self.has_attribute(attribute).then(|| {
            raw::ast_attribute_type(self.0, attribute).unwrap_or_else(|err| {
                unreachable!("has_attribute already confirmed the attribute is present: {err}")
            })
        })
    }

    /// Checks that `attribute` is present and of kind `expected`, so every
    /// typed accessor below fails uniformly with a clingox-raised
    /// [`ErrorKind::InvalidInput`], never clingo's own less specific
    /// `Runtime`/`Logic` error for the same mistake, so the error kind is
    /// uniform.
    fn require_type(&self, attribute: Attribute, expected: AttributeType) -> Result<()> {
        if self.attribute_type(attribute) == Some(expected) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "attribute `{attribute:?}` is not a `{expected:?}` on this `{:?}` node",
                    self.ast_type()
                ),
            ))
        }
    }

    /// Checks `index` against `len`, the same way as `require_type` for a
    /// wrong `AttributeType`, before an array accessor ever reaches clingo:
    /// clingo's own bounds check on the six index-taking write functions
    /// is missing or UB, and the checked getters here stay uniform
    /// with them rather than relying on `std::vector::at`'s own `Logic`
    /// error for the read side.
    fn require_index(attribute: Attribute, index: usize, len: usize) -> Result<()> {
        if index < len {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "index {index} is out of range for attribute `{attribute:?}`, which has {len} elements"
                ),
            ))
        }
    }

    /// The value of a `number` attribute.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent, or present with
    /// a different [`AttributeType`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::Attribute;
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string("not a.", |node| { nodes.push(node); Ok(()) })?;
    /// let head = nodes[1].ast(Attribute::Head)?;
    /// assert_eq!(head.number(Attribute::Sign)?, 1);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn number(&self, attribute: Attribute) -> Result<i32> {
        self.require_type(attribute, AttributeType::Number)?;
        raw::ast_attribute_get_number(self.0, attribute)
    }

    /// The value of a `symbol` attribute.
    ///
    /// # Errors
    ///
    /// As [`Ast::number`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{Attribute, AstType};
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string("a(1).", |node| { nodes.push(node); Ok(()) })?;
    /// let term = nodes[1]
    ///     .ast(Attribute::Head)?
    ///     .ast(Attribute::Atom)?
    ///     .ast(Attribute::Symbol)?
    ///     .ast_at(Attribute::Arguments, 0)?;
    /// assert_eq!(term.ast_type(), AstType::SymbolicTerm);
    /// assert_eq!(term.symbol(Attribute::Symbol)?, clingox::Symbol::number(1));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn symbol(&self, attribute: Attribute) -> Result<Symbol> {
        self.require_type(attribute, AttributeType::Symbol)?;
        raw::ast_attribute_get_symbol(self.0, attribute)
    }

    /// The value of a `location` attribute.
    ///
    /// # Errors
    ///
    /// As [`Ast::number`], and [`ErrorKind::Utf8`] if a file name of the span
    /// is not valid UTF-8, which only a file that clingo reads itself can cause
    /// (an `#include` of a file whose name is not UTF-8). The name is never
    /// converted lossily, because the converted name would match no file.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::Attribute;
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string("a.\n", |node| { nodes.push(node); Ok(()) })?;
    /// let span = nodes[1].span(Attribute::Location)?;
    /// assert_eq!((span.begin_line(), span.begin_column()), (1, 1));
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn span(&self, attribute: Attribute) -> Result<Span> {
        self.require_type(attribute, AttributeType::Location)?;
        raw::ast_attribute_get_location(self.0, attribute)
    }

    /// The value of a `string` attribute, copied into an owned [`String`].
    ///
    /// Text that is not valid UTF-8, which only unrestricted text such as a
    /// `#script` block's body can carry through the parser, is
    /// [`ErrorKind::Utf8`], never a lossy conversion (the same rule
    /// [`Symbol::as_string`] follows).
    ///
    /// # Errors
    ///
    /// As [`Ast::number`], and [`ErrorKind::Utf8`] above.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::Attribute;
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string("p(x).", |node| { nodes.push(node); Ok(()) })?;
    /// let symbol = nodes[1].ast(Attribute::Head)?.ast(Attribute::Atom)?.ast(Attribute::Symbol)?;
    /// assert_eq!(symbol.string(Attribute::Name)?, "p");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn string(&self, attribute: Attribute) -> Result<String> {
        self.require_type(attribute, AttributeType::String)?;
        raw::ast_attribute_get_string(self.0, attribute)
    }

    /// The value of an `ast` attribute: a new, independently owned child
    /// that outlives `self` (clingo `incRef`s it before returning it, so it
    /// needs no lifetime tied to `&self`).
    ///
    /// # Errors
    ///
    /// As [`Ast::number`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{AstType, Attribute};
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string("a.", |node| { nodes.push(node); Ok(()) })?;
    /// let rule = nodes.pop().unwrap();
    /// let head = rule.ast(Attribute::Head)?;
    /// drop(rule);
    /// assert_eq!(head.ast_type(), AstType::Literal);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn ast(&self, attribute: Attribute) -> Result<Ast> {
        self.require_type(attribute, AttributeType::Ast)?;
        raw::ast_attribute_get_ast(self.0, attribute).map(Ast::from_raw)
    }

    /// The value of an `optional_ast` attribute: `None` for clingo's own absent
    /// value, otherwise an owned child as [`Ast::ast`].
    ///
    /// # Errors
    ///
    /// As [`Ast::number`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::Attribute;
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string(":- #count{X : p(X)} > 3.", |node| { nodes.push(node); Ok(()) })?;
    /// let atom = nodes[1].ast_at(Attribute::Body, 0)?.ast(Attribute::Atom)?;
    /// assert!(atom.optional_ast(Attribute::LeftGuard)?.is_some());
    /// assert!(atom.optional_ast(Attribute::RightGuard)?.is_none());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn optional_ast(&self, attribute: Attribute) -> Result<Option<Ast>> {
        self.require_type(attribute, AttributeType::OptionalAst)?;
        raw::ast_attribute_get_optional_ast(self.0, attribute).map(|opt| opt.map(Ast::from_raw))
    }

    /// The element at `index` of a `string_array` attribute, copied into an
    /// owned [`String`].
    ///
    /// # Errors
    ///
    /// As [`Ast::string`], and [`ErrorKind::InvalidInput`] if `index` is out
    /// of range.
    pub fn string_at(&self, attribute: Attribute, index: usize) -> Result<String> {
        self.require_type(attribute, AttributeType::StringArray)?;
        let len = raw::ast_attribute_size_string_array(self.0, attribute)?;
        Self::require_index(attribute, index, len)?;
        raw::ast_attribute_get_string_at(self.0, attribute, index)
    }

    /// The element at `index` of an `ast_array` attribute, an owned child as
    /// [`Ast::ast`].
    ///
    /// # Errors
    ///
    /// As [`Ast::ast`], and [`ErrorKind::InvalidInput`] if `index` is out of
    /// range.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::Attribute;
    ///
    /// let mut nodes = Vec::new();
    /// clingox::ast::parse_string("q(X) :- p(X), X > 1.", |node| { nodes.push(node); Ok(()) })?;
    /// let rule = &nodes[1];
    /// assert_eq!(rule.ast_array_len(Attribute::Body)?, 2);
    /// let first = rule.ast_at(Attribute::Body, 0)?;
    /// assert_eq!(first.to_string(), "p(X)");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn ast_at(&self, attribute: Attribute, index: usize) -> Result<Ast> {
        self.require_type(attribute, AttributeType::AstArray)?;
        let len = raw::ast_attribute_size_ast_array(self.0, attribute)?;
        Self::require_index(attribute, index, len)?;
        raw::ast_attribute_get_ast_at(self.0, attribute, index).map(Ast::from_raw)
    }

    /// The number of elements of a `string_array` attribute.
    ///
    /// # Errors
    ///
    /// As [`Ast::number`].
    pub fn string_array_len(&self, attribute: Attribute) -> Result<usize> {
        self.require_type(attribute, AttributeType::StringArray)?;
        raw::ast_attribute_size_string_array(self.0, attribute)
    }

    /// The number of elements of an `ast_array` attribute.
    ///
    /// # Errors
    ///
    /// As [`Ast::number`].
    pub fn ast_array_len(&self, attribute: Attribute) -> Result<usize> {
        self.require_type(attribute, AttributeType::AstArray)?;
        raw::ast_attribute_size_ast_array(self.0, attribute)
    }
}

/// Setters and array editors.
///
/// All take `&self`: clones of an `Ast` share one node, so an edit through one
/// handle is seen through every clone and every parent that holds the node,
/// as with pyclingo's attribute assignment. Values are borrowed, and after the
/// call the caller's handles stay valid (clingo keeps its own reference).
///
/// Every method checks, in this order, and reports the first problem as an
/// error before it changes anything:
///
/// 1. the attribute is present on this node and of the method's
///    [`AttributeType`]: [`ErrorKind::InvalidInput`], never clingo's own
///    `Runtime` error for the same mistake;
/// 2. the value: [`ErrorKind::Nul`] for a NUL byte in a string, and for
///    [`Ast::set_number`] a number outside the attribute's domain
///    ([`ErrorKind::InvalidInput`]);
/// 3. for a node value, that it is not `self` and does not reach `self`
///    through its children: [`ErrorKind::InvalidInput`]. clingo accepts a
///    cycle and then overflows the stack in `Display`, `Hash`, `Eq` and
///    `deep_copy`, so clingox refuses it;
/// 4. for the `*_at` methods, the index: [`ErrorKind::InvalidInput`], where
///    clingo would read or write out of bounds.
///
/// Because a node's hash and order depend on its content, do not edit a node
/// that is a key of a `HashMap`, `HashSet` or `BTreeMap` (see [`Ast`]).
impl Ast {
    /// Checks `attribute` (check 1) and returns the length of the array it
    /// names, for the `*_at` methods.
    fn array_len(&self, attribute: Attribute, expected: AttributeType) -> Result<usize> {
        self.require_type(attribute, expected)?;
        match expected {
            AttributeType::StringArray => raw::ast_attribute_size_string_array(self.0, attribute),
            _ => raw::ast_attribute_size_ast_array(self.0, attribute),
        }
    }

    /// Check 4 for an insertion: `index == len` appends.
    fn require_insert_index(attribute: Attribute, index: usize, len: usize) -> Result<()> {
        if index <= len {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "insertion index {index} is beyond the end of attribute `{attribute:?}`, which has {len} elements"
                ),
            ))
        }
    }

    /// Check 3: `value` must not be `self`, nor reach it through child
    /// attributes.
    ///
    /// Iterative (an explicit stack, never recursion) with a set of the node
    /// pointers seen, so it is linear in the number of distinct nodes, a
    /// shared child reached twice is visited once, and a tree as deep as
    /// clingo can build cannot overflow the stack here. A node of an unknown
    /// type ([`AstType::Other`], only reachable through a newer clingo) has no
    /// known children and is not descended into.
    fn require_no_cycle(&self, attribute: Attribute, value: &Ast) -> Result<()> {
        let mut seen: HashSet<RawAst> = HashSet::new();
        let mut stack = vec![value.clone()];
        while let Some(node) = stack.pop() {
            if node.ptr_eq(self) {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "setting `{attribute:?}` on this `{:?}` node would make the node its own descendant",
                        self.ast_type()
                    ),
                ));
            }
            if !seen.insert(node.0) {
                continue;
            }
            for &(child, kind) in child_attributes(node.ast_type()) {
                match kind {
                    AttributeType::Ast => stack.push(node.ast(child)?),
                    AttributeType::OptionalAst => stack.extend(node.optional_ast(child)?),
                    AttributeType::AstArray => {
                        for index in 0..node.ast_array_len(child)? {
                            stack.push(node.ast_at(child, index)?);
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Writes a `number` attribute.
    ///
    /// A number that stands for an enum or a boolean must be in its domain:
    /// `0..=max` for the attributes the enum types of this module cover
    /// ([`LiteralSign`], [`BinaryOperator`], ...), `0` or `1` for a boolean
    /// such as `function.external`. clingo itself accepts any integer and then
    /// prints the node wrongly. Other numbers (`priority`, `arity`) take any
    /// `i32`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent, not a
    /// `number`, or `value` is outside its domain.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ErrorKind;
    /// use clingox::ast::{self, Attribute};
    ///
    /// let mut nodes = Vec::new();
    /// ast::parse_string("a.", |node| { nodes.push(node); Ok(()) })?;
    /// let literal = nodes[1].ast(Attribute::Head)?;
    /// literal.set_number(Attribute::Sign, 1)?;
    /// assert_eq!(nodes[1].to_string(), "not a.");
    /// let err = literal.set_number(Attribute::Sign, 7).unwrap_err();
    /// assert_eq!(err.kind(), ErrorKind::InvalidInput);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn set_number(&self, attribute: Attribute, value: i32) -> Result<()> {
        self.require_type(attribute, AttributeType::Number)?;
        let (allowed, domain) = match number_domain(self.ast_type(), attribute) {
            NumberDomain::Any => (true, String::new()),
            NumberDomain::Bool => (matches!(value, 0 | 1), "0 or 1".to_owned()),
            NumberDomain::Range(max) => ((0..=max).contains(&value), format!("0..={max}")),
        };
        if !allowed {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "{value} is outside {domain}, the values of `{attribute:?}` on a `{:?}` node",
                    self.ast_type()
                ),
            ));
        }
        raw::ast_attribute_set_number(self.0, attribute, value)
    }

    /// Writes a `symbol` attribute.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent or not a
    /// `symbol`.
    pub fn set_symbol(&self, attribute: Attribute, value: Symbol) -> Result<()> {
        self.require_type(attribute, AttributeType::Symbol)?;
        raw::ast_attribute_set_symbol(self.0, attribute, value)
    }

    /// Writes a `location` attribute.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent or not a
    /// `location`. A [`Span`] is valid by construction, so its contents
    /// cannot fail.
    pub fn set_span(&self, attribute: Attribute, value: &Span) -> Result<()> {
        self.require_type(attribute, AttributeType::Location)?;
        raw::ast_attribute_set_location(self.0, attribute, value)
    }

    /// Writes a `string` attribute.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent or not a
    /// `string`; [`ErrorKind::Nul`] if `value` contains a NUL byte.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{self, Attribute};
    ///
    /// let mut nodes = Vec::new();
    /// ast::parse_string("p(X).", |node| { nodes.push(node); Ok(()) })?;
    /// let symbol = nodes[1].ast(Attribute::Head)?.ast(Attribute::Atom)?.ast(Attribute::Symbol)?;
    /// let before = symbol.string(Attribute::Name)?;
    /// symbol.set_string(Attribute::Name, "q")?;
    /// // The owned `String` read earlier is unchanged; the node is not.
    /// assert_eq!(before, "p");
    /// assert_eq!(nodes[1].to_string(), "q(X).");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn set_string(&self, attribute: Attribute, value: &str) -> Result<()> {
        self.require_type(attribute, AttributeType::String)?;
        let value = raw::c_str(value)?;
        raw::ast_attribute_set_string(self.0, attribute, &value)
    }

    fn set_ast_with(&self, attribute: Attribute, value: &Ast, check_cycle: bool) -> Result<()> {
        self.require_type(attribute, AttributeType::Ast)?;
        if check_cycle {
            self.require_no_cycle(attribute, value)?;
        }
        raw::ast_attribute_set_ast(self.0, attribute, value.0)
    }

    /// Writes an `ast` attribute. `value` is borrowed: it stays usable, and
    /// the node's `attribute` is [`Ast::ptr_eq`] to it afterwards.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent or not an
    /// `ast`, or if `value` is `self` or reaches `self` through its children.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ErrorKind;
    /// use clingox::ast::{self, Attribute};
    ///
    /// let mut nodes = Vec::new();
    /// ast::parse_string("a. b.", |node| { nodes.push(node); Ok(()) })?;
    /// let (first, second) = (nodes[1].clone(), nodes[2].clone());
    /// first.set_ast(Attribute::Head, &second.ast(Attribute::Head)?)?;
    /// assert_eq!(first.to_string(), "b.");
    ///
    /// // A node cannot become its own descendant.
    /// let err = first.set_ast(Attribute::Head, &first).unwrap_err();
    /// assert_eq!(err.kind(), ErrorKind::InvalidInput);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn set_ast(&self, attribute: Attribute, value: &Ast) -> Result<()> {
        self.set_ast_with(attribute, value, true)
    }

    /// As [`Ast::set_ast`], without the cycle check.
    ///
    /// Only for a `self` that no one but the caller holds a handle to yet, so
    /// that no `value` can reach it: `walk` edits the copy it made, which
    /// nothing but `walk` references until it returns.
    pub(crate) fn set_ast_fresh(&self, attribute: Attribute, value: &Ast) -> Result<()> {
        self.set_ast_with(attribute, value, false)
    }

    fn set_optional_ast_with(
        &self,
        attribute: Attribute,
        value: Option<&Ast>,
        check_cycle: bool,
    ) -> Result<()> {
        self.require_type(attribute, AttributeType::OptionalAst)?;
        if let (true, Some(value)) = (check_cycle, value) {
            self.require_no_cycle(attribute, value)?;
        }
        raw::ast_attribute_set_optional_ast(self.0, attribute, value.map(Ast::as_raw))
    }

    /// Writes an `optional_ast` attribute; `None` clears it.
    ///
    /// # Errors
    ///
    /// As [`Ast::set_ast`], except that an `ast` attribute is the wrong kind
    /// here, and `None` never forms a cycle.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{self, Attribute};
    ///
    /// let mut nodes = Vec::new();
    /// ast::parse_string(":- #count{X : p(X)} > 3.", |node| { nodes.push(node); Ok(()) })?;
    /// let atom = nodes[1].ast_at(Attribute::Body, 0)?.ast(Attribute::Atom)?;
    /// assert!(atom.optional_ast(Attribute::LeftGuard)?.is_some());
    /// atom.set_optional_ast(Attribute::LeftGuard, None)?;
    /// assert!(atom.optional_ast(Attribute::LeftGuard)?.is_none());
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn set_optional_ast(&self, attribute: Attribute, value: Option<&Ast>) -> Result<()> {
        self.set_optional_ast_with(attribute, value, true)
    }

    /// As [`Ast::set_optional_ast`], without the cycle check; see
    /// [`Ast::set_ast_fresh`] for when that is sound.
    pub(crate) fn set_optional_ast_fresh(&self, attribute: Attribute, value: &Ast) -> Result<()> {
        self.set_optional_ast_with(attribute, Some(value), false)
    }

    /// Replaces the element at `index` of a `string_array` attribute.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent, not a
    /// `string_array`, or `index >= len`; [`ErrorKind::Nul`] for a NUL byte.
    pub fn set_string_at(&self, attribute: Attribute, index: usize, value: &str) -> Result<()> {
        let len = self.array_len(attribute, AttributeType::StringArray)?;
        let value = raw::c_str(value)?;
        Self::require_index(attribute, index, len)?;
        raw::ast_attribute_set_string_at(self.0, attribute, index, &value)
    }

    /// Inserts an element at `index` of a `string_array` attribute; `index ==
    /// len` appends.
    ///
    /// # Errors
    ///
    /// As [`Ast::set_string_at`], with `index > len` out of range.
    pub fn insert_string_at(&self, attribute: Attribute, index: usize, value: &str) -> Result<()> {
        let len = self.array_len(attribute, AttributeType::StringArray)?;
        let value = raw::c_str(value)?;
        Self::require_insert_index(attribute, index, len)?;
        raw::ast_attribute_insert_string_at(self.0, attribute, index, &value)
    }

    /// Deletes the element at `index` of a `string_array` attribute.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent, not a
    /// `string_array`, or `index >= len` (so on an empty array always).
    pub fn delete_string_at(&self, attribute: Attribute, index: usize) -> Result<()> {
        let len = self.array_len(attribute, AttributeType::StringArray)?;
        Self::require_index(attribute, index, len)?;
        raw::ast_attribute_delete_string_at(self.0, attribute, index)
    }

    fn set_ast_at_with(
        &self,
        attribute: Attribute,
        index: usize,
        value: &Ast,
        check_cycle: bool,
    ) -> Result<()> {
        let len = self.array_len(attribute, AttributeType::AstArray)?;
        if check_cycle {
            self.require_no_cycle(attribute, value)?;
        }
        Self::require_index(attribute, index, len)?;
        raw::ast_attribute_set_ast_at(self.0, attribute, index, value.0)
    }

    /// Replaces the element at `index` of an `ast_array` attribute.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent, not an
    /// `ast_array`, `index >= len`, or `value` is `self` or reaches `self`.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{self, Attribute};
    ///
    /// let mut nodes = Vec::new();
    /// ast::parse_string("h :- a, b.", |node| { nodes.push(node); Ok(()) })?;
    /// let rule = &nodes[1];
    /// // Aliasing is fine: the element is held by its handle for the call.
    /// rule.set_ast_at(Attribute::Body, 1, &rule.ast_at(Attribute::Body, 0)?)?;
    /// assert_eq!(rule.to_string(), "h :- a; a.");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn set_ast_at(&self, attribute: Attribute, index: usize, value: &Ast) -> Result<()> {
        self.set_ast_at_with(attribute, index, value, true)
    }

    /// As [`Ast::set_ast_at`], without the cycle check; see
    /// [`Ast::set_ast_fresh`] for when that is sound.
    pub(crate) fn set_ast_at_fresh(
        &self,
        attribute: Attribute,
        index: usize,
        value: &Ast,
    ) -> Result<()> {
        self.set_ast_at_with(attribute, index, value, false)
    }

    /// Inserts an element at `index` of an `ast_array` attribute; `index ==
    /// len` appends.
    ///
    /// # Errors
    ///
    /// As [`Ast::set_ast_at`], with `index > len` out of range.
    pub fn insert_ast_at(&self, attribute: Attribute, index: usize, value: &Ast) -> Result<()> {
        let len = self.array_len(attribute, AttributeType::AstArray)?;
        self.require_no_cycle(attribute, value)?;
        Self::require_insert_index(attribute, index, len)?;
        raw::ast_attribute_insert_ast_at(self.0, attribute, index, value.0)
    }

    /// Deletes the element at `index` of an `ast_array` attribute.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent, not an
    /// `ast_array`, or `index >= len` (so on an empty array always).
    pub fn delete_ast_at(&self, attribute: Attribute, index: usize) -> Result<()> {
        let len = self.array_len(attribute, AttributeType::AstArray)?;
        Self::require_index(attribute, index, len)?;
        raw::ast_attribute_delete_ast_at(self.0, attribute, index)
    }

    /// Appends an element to a `string_array` attribute
    /// (`insert_string_at(len)`).
    ///
    /// # Errors
    ///
    /// As [`Ast::insert_string_at`], without the index errors.
    pub fn push_string(&self, attribute: Attribute, value: &str) -> Result<()> {
        let len = self.array_len(attribute, AttributeType::StringArray)?;
        let value = raw::c_str(value)?;
        raw::ast_attribute_insert_string_at(self.0, attribute, len, &value)
    }

    /// Appends an element to an `ast_array` attribute (`insert_ast_at(len)`).
    ///
    /// # Errors
    ///
    /// As [`Ast::insert_ast_at`], without the index errors.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{self, Attribute};
    ///
    /// let mut nodes = Vec::new();
    /// ast::parse_string("h :- a. b.", |node| { nodes.push(node); Ok(()) })?;
    /// let rule = nodes[1].copy()?; // edit a copy, keep the parsed rule as it was
    /// rule.push_ast(Attribute::Body, &nodes[2].ast(Attribute::Head)?)?;
    /// assert_eq!(rule.to_string(), "h :- a; b.");
    /// assert_eq!(nodes[1].to_string(), "h :- a.");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn push_ast(&self, attribute: Attribute, value: &Ast) -> Result<()> {
        let len = self.array_len(attribute, AttributeType::AstArray)?;
        self.require_no_cycle(attribute, value)?;
        raw::ast_attribute_insert_ast_at(self.0, attribute, len, value.0)
    }

    /// Replaces a whole `string_array` attribute: every element is deleted,
    /// then `values` are inserted in order (pyclingo's assignment of a list).
    ///
    /// Every failure that can be reported beforehand (the attribute, a NUL
    /// byte in any element) is reported before anything changes. If clingo
    /// then runs out of memory ([`ErrorKind::BadAlloc`]) part way through,
    /// the array may be left partially updated.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent or not a
    /// `string_array`; [`ErrorKind::Nul`]; [`ErrorKind::BadAlloc`] as above.
    pub fn set_string_array(&self, attribute: Attribute, values: &[impl AsRef<str>]) -> Result<()> {
        let len = self.array_len(attribute, AttributeType::StringArray)?;
        let values = values
            .iter()
            .map(|value| raw::c_str(value.as_ref()))
            .collect::<Result<Vec<_>>>()?;
        for index in (0..len).rev() {
            raw::ast_attribute_delete_string_at(self.0, attribute, index)?;
        }
        for (index, value) in values.iter().enumerate() {
            raw::ast_attribute_insert_string_at(self.0, attribute, index, value)?;
        }
        Ok(())
    }

    /// Replaces a whole `ast_array` attribute: every element is deleted, then
    /// `values` are inserted in order. `values` may be collected from the same
    /// array: the handles keep the elements alive.
    ///
    /// Every failure that can be reported beforehand (the attribute, a cycle
    /// through any element) is reported before anything changes. If clingo then
    /// runs out of memory ([`ErrorKind::BadAlloc`]) part way through, the array
    /// may be left partially updated.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] if the attribute is absent or not an
    /// `ast_array`, or an element is `self` or reaches `self`;
    /// [`ErrorKind::BadAlloc`] as above.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{self, Attribute};
    ///
    /// let mut nodes = Vec::new();
    /// ast::parse_string("h :- a, b, c.", |node| { nodes.push(node); Ok(()) })?;
    /// let rule = &nodes[1];
    /// let mut body: Vec<_> = (0..3)
    ///     .map(|i| rule.ast_at(Attribute::Body, i))
    ///     .collect::<Result<_, _>>()?;
    /// body.reverse();
    /// rule.set_ast_array(Attribute::Body, &body)?;
    /// assert_eq!(rule.to_string(), "h :- c; b; a.");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn set_ast_array(&self, attribute: Attribute, values: &[Ast]) -> Result<()> {
        let len = self.array_len(attribute, AttributeType::AstArray)?;
        for value in values {
            self.require_no_cycle(attribute, value)?;
        }
        for index in (0..len).rev() {
            raw::ast_attribute_delete_ast_at(self.0, attribute, index)?;
        }
        for (index, value) in values.iter().enumerate() {
            raw::ast_attribute_insert_ast_at(self.0, attribute, index, value.0)?;
        }
        Ok(())
    }
}

impl Clone for Ast {
    /// Acquires a new reference to the same node (`clingo_ast_acquire`);
    /// infallible.
    fn clone(&self) -> Ast {
        raw::ast_acquire(self.0);
        Ast(self.0)
    }
}

impl Drop for Ast {
    /// Releases this reference (`clingo_ast_release`), freeing the node once
    /// its last reference is gone; infallible, and never panics.
    fn drop(&mut self) {
        raw::ast_release(self.0);
    }
}

impl PartialEq for Ast {
    /// Structural equality, ignoring the `location` attribute
    /// (`clingo_ast_equal`; pyclingo's own documented claim, `ast.py:979-982`).
    fn eq(&self, other: &Ast) -> bool {
        raw::ast_equal(self.0, other.0)
    }
}

impl Eq for Ast {}

impl PartialOrd for Ast {
    fn partial_cmp(&self, other: &Ast) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ast {
    /// Structural order, ignoring `location` as [`Ast::eq`] does
    /// (`clingo_ast_less_than`, called twice for a three-way order, the same
    /// shape [`Symbol::cmp`](crate::Symbol) already uses).
    fn cmp(&self, other: &Ast) -> Ordering {
        if raw::ast_less_than(self.0, other.0) {
            Ordering::Less
        } else if raw::ast_less_than(other.0, self.0) {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

impl Hash for Ast {
    /// Agrees with [`Ast::eq`]: also ignores `location`
    /// (`clingo_ast_hash`, `astv2.cc:47-50`, `:164-174`).
    ///
    /// The hash follows the node's content, which a setter called through any
    /// clone can change. A node used as a key of a `HashMap` or `HashSet` must
    /// not be edited while it is in the collection; doing so is a logic error
    /// in the collection, never undefined behavior.
    fn hash<H: Hasher>(&self, state: &mut H) {
        raw::ast_hash(self.0).hash(state);
    }
}

impl Ast {
    /// As [`Display`](fmt::Display), but fallible: `Display` itself never
    /// fails (it writes a placeholder instead), so this is how a caller who
    /// wants to know *why* printing failed can find out.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Utf8`] if the node holds text that is not valid UTF-8
    ///   (for example a `#script` block read from a file with a stray
    ///   `\xff` byte);
    /// - otherwise the error clingo reported, such as [`ErrorKind::BadAlloc`].
    pub fn try_to_string(&self) -> Result<String> {
        raw::ast_to_string(self.0)
    }
}

impl fmt::Display for Ast {
    /// gringo's own textual form (`clingo_ast_to_string`). For a node that
    /// came from a real parse, the printed text re-parses to a node equal
    /// (location-ignoring) to this one; a hand-built node whose
    /// attributes were never checked for well-formedness is not promised
    /// to (pyclingo's own doc, `ast.py:983-984`).
    ///
    /// Never returns [`fmt::Error`] for a clingo or UTF-8 failure: the
    /// blanket `ToString` implementation `expect`s `Display` never to
    /// fail, so that would make `to_string()` panic. A failure instead
    /// writes a placeholder, `<error: ...>`, as [`TheoryAtom`'s
    /// `Display`](crate::TheoryAtom) does; [`Ast::try_to_string`] is the
    /// fallible way to print.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_to_string() {
            Ok(text) => f.write_str(&text),
            Err(err) => write!(f, "<error: {err}>"),
        }
    }
}

/// Every [`Attribute`] variant, in declaration order, for [`Ast`]'s own
/// fallback [`fmt::Debug`] to count.
const ATTRIBUTES: [Attribute; 45] = [
    Attribute::Argument,
    Attribute::Arguments,
    Attribute::Arity,
    Attribute::Atom,
    Attribute::Atoms,
    Attribute::AtomType,
    Attribute::Bias,
    Attribute::Body,
    Attribute::Code,
    Attribute::Coefficient,
    Attribute::Comparison,
    Attribute::Condition,
    Attribute::Elements,
    Attribute::External,
    Attribute::ExternalType,
    Attribute::Function,
    Attribute::Guard,
    Attribute::Guards,
    Attribute::Head,
    Attribute::IsDefault,
    Attribute::Left,
    Attribute::LeftGuard,
    Attribute::Literal,
    Attribute::Location,
    Attribute::Modifier,
    Attribute::Name,
    Attribute::NodeU,
    Attribute::NodeV,
    Attribute::OperatorName,
    Attribute::OperatorType,
    Attribute::Operators,
    Attribute::Parameters,
    Attribute::Positive,
    Attribute::Priority,
    Attribute::Right,
    Attribute::RightGuard,
    Attribute::SequenceType,
    Attribute::Sign,
    Attribute::Symbol,
    Attribute::Term,
    Attribute::Terms,
    Attribute::Value,
    Attribute::Variable,
    Attribute::Weight,
    Attribute::CommentType,
];

impl fmt::Debug for Ast {
    /// `"Ast(<type>, <n> attributes)"`, never the raw pointer (RULES 11.3).
    /// Use `Display` to see the node's text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = ATTRIBUTES
            .iter()
            .filter(|&&a| self.has_attribute(a))
            .count();
        write!(f, "Ast({:?}, {count} attributes)", self.ast_type())
    }
}

/// The type of a node (`clingo_ast_type_e`, 46 named kinds).
///
/// [`AstType::Other`] is a node type a newer clingo defines that this build
/// does not know; only [`Ast::ast_type`] ever produces one, by reading an
/// already-existing node, and nothing in the crate constructs one.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AstType {
    // terms
    /// An identifier, such as a `#program` parameter name.
    Id,
    /// A variable, such as `X`.
    Variable,
    /// A symbolic term: a plain symbol embedded in the syntax tree.
    SymbolicTerm,
    /// A unary operation, such as `-a`.
    UnaryOperation,
    /// A binary operation, such as `a+b`.
    BinaryOperation,
    /// An interval, such as `1..3`.
    Interval,
    /// A function term, including a constant or a tuple.
    Function,
    /// A pool of alternatives, such as `(a;b)`.
    Pool,
    // simple atoms
    /// `#true` or `#false`.
    BooleanConstant,
    /// A symbolic atom, such as `p(X)`.
    SymbolicAtom,
    /// A comparison, such as `X > 1`.
    Comparison,
    // aggregates
    /// One side of a comparison or an aggregate's bound.
    Guard,
    /// A literal with a condition, inside an aggregate or a disjunction.
    ConditionalLiteral,
    /// An aggregate over conditional literals.
    Aggregate,
    /// One element of a body aggregate.
    BodyAggregateElement,
    /// A `#count`/`#sum`/... aggregate in a rule's body.
    BodyAggregate,
    /// One element of a head aggregate.
    HeadAggregateElement,
    /// A `#count`/`#sum`/... aggregate in a rule's head.
    HeadAggregate,
    /// A disjunction of literals.
    Disjunction,
    // theory atoms
    /// A theory sequence, such as a theory tuple or list.
    TheorySequence,
    /// A theory function term.
    TheoryFunction,
    /// One element of an unparsed theory term.
    TheoryUnparsedTermElement,
    /// An unparsed theory term, before operator precedence is resolved.
    TheoryUnparsedTerm,
    /// A theory atom's guard.
    TheoryGuard,
    /// One element of a theory atom.
    TheoryAtomElement,
    /// A theory atom.
    TheoryAtom,
    // literals
    /// A literal, with a sign and an atom.
    Literal,
    // theory definition
    /// One operator of a `#theory` term definition.
    TheoryOperatorDefinition,
    /// One term definition inside a `#theory` statement.
    TheoryTermDefinition,
    /// A theory atom definition's guard.
    TheoryGuardDefinition,
    /// One atom definition inside a `#theory` statement.
    TheoryAtomDefinition,
    // statements
    /// A rule, including an integrity constraint or a fact.
    Rule,
    /// A `#const` definition.
    Definition,
    /// A `#show p/n.` or `#show -p/n.` directive.
    ShowSignature,
    /// A `#show term : body.` directive.
    ShowTerm,
    /// A `#minimize`/`#maximize` statement.
    Minimize,
    /// A `#script` block.
    Script,
    /// A `#program name(parameters).` directive.
    Program,
    /// An `#external` statement.
    External,
    /// An `#edge` statement.
    Edge,
    /// A `#heuristic` statement.
    Heuristic,
    /// One atom of a `#project` statement.
    ProjectAtom,
    /// A `#project p/n.` directive.
    ProjectSignature,
    /// A `#defined` directive.
    Defined,
    /// A `#theory` statement.
    TheoryDefinition,
    /// A comment.
    Comment,
    /// A node type this build of clingox does not know.
    Other(i32),
}

/// The name of an [`Ast`] attribute (`clingo_ast_attribute_e`, 45 named
/// entries).
///
/// [`Attribute::Coefficient`] and [`Attribute::Variable`] are declared by the
/// header and appear in clingo's own attribute-name table, but no node kind
/// this clingo produces ever carries them (checked directly against the
/// constructor table, `control.cc:1428-1487`): every typed accessor called
/// with either fails [`ErrorKind::InvalidInput`] on every node.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Attribute {
    /// The operand of a unary operation.
    Argument,
    /// The arguments of a function, a pool or a theory function.
    Arguments,
    /// The arity of a `#show`, `#project` or `#defined` signature, or of a
    /// theory atom definition. Never negative in a node built by clingox.
    Arity,
    /// The atom of a literal, or of an `#external`, `#heuristic` or `#project`
    /// statement.
    Atom,
    /// A theory definition's atom definitions.
    Atoms,
    /// A theory atom definition's head/body/any/directive kind.
    AtomType,
    /// A `#heuristic` statement's bias term.
    Bias,
    /// The body, a sequence of literals, of a rule or of an `#external`,
    /// `#edge`, `#heuristic`, `#project` atom, `#show` term or `#minimize`
    /// statement.
    Body,
    /// A `#script` block's code.
    Code,
    /// A coefficient. No constructor this clingo defines ever uses it.
    Coefficient,
    /// A comparison operator.
    Comparison,
    /// The condition of a conditional literal, or of an aggregate or theory
    /// atom element.
    Condition,
    /// The elements of an aggregate, a disjunction, a theory atom or an
    /// unparsed theory term.
    Elements,
    /// Whether a function term is an external function, written `@f(..)`, a
    /// call into a script (0 or 1).
    External,
    /// An `#external` statement's type term (`true`, `false`, `free` or
    /// `defined`).
    ExternalType,
    /// A body or head aggregate's function (`#count`, `#sum`, ...).
    Function,
    /// A theory atom's or a theory atom definition's guard.
    Guard,
    /// A sequence of guards.
    Guards,
    /// A rule's head.
    Head,
    /// Whether a `#const` definition is a default, which a command-line
    /// definition may override (0 or 1); `[override]` makes it 0.
    IsDefault,
    /// The left operand of a binary operation, or the lower end of an interval.
    Left,
    /// An aggregate's left-hand bound.
    LeftGuard,
    /// The literal of a conditional literal.
    Literal,
    /// A node's source location.
    Location,
    /// A `#heuristic` statement's modifier.
    Modifier,
    /// A name: a function, an identifier, a theory operator, or similar.
    Name,
    /// The first endpoint of an `#edge` statement.
    NodeU,
    /// The second endpoint of an `#edge` statement.
    NodeV,
    /// A theory guard's operator name.
    OperatorName,
    /// The operator of a unary or binary operation, or the kind of a theory
    /// operator definition.
    OperatorType,
    /// The operators of an unparsed theory term element or of a theory guard
    /// definition (names), or of a theory term definition (operator
    /// definitions).
    Operators,
    /// A `#program` directive's parameters.
    Parameters,
    /// Whether a `#show`, `#project` or `#defined` signature is positive, that
    /// is not classically negated (0 or 1).
    Positive,
    /// The priority of a `#minimize` or `#heuristic` statement (a term), or of
    /// a theory operator definition (a number).
    Priority,
    /// The right operand of a binary operation, or the upper end of an
    /// interval.
    Right,
    /// An aggregate's right-hand bound.
    RightGuard,
    /// A theory sequence's tuple/list/set kind.
    SequenceType,
    /// A literal's sign.
    Sign,
    /// A symbolic term's symbol, or the term of a symbolic atom.
    Symbol,
    /// The term of a comparison, a guard, a theory atom or a `#show` statement,
    /// or of an unparsed theory term element or theory guard; the name of the
    /// term definition in a theory guard or atom definition.
    Term,
    /// A sequence of terms (an aggregate or theory atom element's tuple, a
    /// theory sequence's terms, a `#minimize` statement's tuple, or a theory
    /// definition's term definitions).
    Terms,
    /// A boolean constant's or a `#const` definition's value, or a comment's
    /// text.
    Value,
    /// Declared by the header, but no node kind this clingo produces carries it
    /// (a variable's name is [`Attribute::Name`]).
    Variable,
    /// A `#minimize` element's weight.
    Weight,
    /// A comment's kind (block or line).
    CommentType,
}

/// The kind of value an [`Attribute`] holds on a particular node
/// (`clingo_ast_attribute_type_e`, 8 kinds).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttributeType {
    /// Read with [`Ast::number`].
    Number,
    /// Read with [`Ast::symbol`].
    Symbol,
    /// Read with [`Ast::span`]. The attribute's own value is a [`Span`], not
    /// a type named `Location` (which [`crate::Location`] already names).
    Location,
    /// Read with [`Ast::string`].
    String,
    /// Read with [`Ast::ast`].
    Ast,
    /// Read with [`Ast::optional_ast`].
    OptionalAst,
    /// Read with [`Ast::string_at`]/[`Ast::string_array_len`].
    StringArray,
    /// Read with [`Ast::ast_at`]/[`Ast::ast_array_len`].
    AstArray,
}

/// A span in a program's source text: where a node begins and ends.
///
/// Unlike [`crate::Location`] (a single point, parsed from the text of a
/// clingo log message), a `Span` is what `clingo_location_t` itself is: a
/// beginning and an end, each a file, a line and a column, which can
/// genuinely differ across files for one node.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    begin_file: &'static str,
    end_file: &'static str,
    begin_line: usize,
    end_line: usize,
    begin_column: usize,
    end_column: usize,
}

impl Span {
    /// A span from clingo's own `clingo_location_t` values, read from a node.
    pub(crate) fn from_raw_parts(
        begin_file: &'static str,
        end_file: &'static str,
        begin_line: usize,
        end_line: usize,
        begin_column: usize,
        end_column: usize,
    ) -> Span {
        Span {
            begin_file,
            end_file,
            begin_line,
            end_line,
            begin_column,
            end_column,
        }
    }

    /// A span from a file, line and column for the beginning and again for
    /// the end. The argument order follows clingo's `Location` constructor.
    ///
    /// The file names are interned with clingo, so [`Span::begin_file`] and
    /// [`Span::end_file`] return `&'static str` and equal names give the same
    /// string. Nothing else is checked: line 0, column 0, an end before the
    /// beginning and two different files are all accepted by clingo.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Nul`] for a NUL byte in either file name;
    /// - [`ErrorKind::InvalidInput`] if a line or column is above
    ///   `u32::MAX`, which clingo would silently truncate to 32 bits;
    /// - [`ErrorKind::BadAlloc`] if clingo runs out of memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::Span;
    ///
    /// let span = Span::new("encoding.lp", 3, 1, "encoding.lp", 3, 9)?;
    /// assert_eq!(span.to_string(), "encoding.lp:3:1-9");
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn new(
        begin_file: &str,
        begin_line: usize,
        begin_column: usize,
        end_file: &str,
        end_line: usize,
        end_column: usize,
    ) -> Result<Span> {
        let numbers = [begin_line, begin_column, end_line, end_column];
        let begin_file = raw::intern(begin_file)?;
        let end_file = raw::intern(end_file)?;
        if numbers.iter().any(|&n| u32::try_from(n).is_err()) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "a line or column above {} does not fit clingo's 32-bit locations: \
                     {begin_line}:{begin_column}-{end_line}:{end_column}",
                    u32::MAX
                ),
            ));
        }
        Ok(Span::from_raw_parts(
            begin_file,
            end_file,
            begin_line,
            end_line,
            begin_column,
            end_column,
        ))
    }

    /// The span for a node a program builds without source text: the file
    /// `<generated>`, line 1, column 1, for the beginning and the end alike.
    /// It is what the clingo crate's `Location::default()` is for. clingo
    /// accepts any location, so the span goes into a node and comes back
    /// unchanged.
    ///
    /// # Panics
    ///
    /// If clingo runs out of memory while it stores the file name, once per
    /// process.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{self, Attribute, Span};
    ///
    /// let span = Span::synthetic();
    /// assert_eq!(span.to_string(), "<generated>:1:1");
    /// let node = ast::id(&span, "x")?;
    /// assert_eq!(node.span(Attribute::Location)?, span);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    #[must_use]
    pub fn synthetic() -> Span {
        static SYNTHETIC: OnceLock<Span> = OnceLock::new();
        SYNTHETIC
            .get_or_init(|| {
                Span::new("<generated>", 1, 1, "<generated>", 1, 1)
                    .unwrap_or_else(|e| panic!("cannot store the synthetic file name: {e}"))
            })
            .clone()
    }

    /// The file where the span begins. `"<string>"` for [`parse_string`]'s
    /// own input, or a similar bracketed name for other synthetic input.
    ///
    /// Internalized by clingo for the duration of the process
    /// (`clingo.h:205-209`), the same flyweight [`crate::Signature::name`]
    /// already reads from; never panics.
    #[must_use]
    pub fn begin_file(&self) -> &'static str {
        self.begin_file
    }

    /// The file where the span ends; usually the same as [`Span::begin_file`].
    #[must_use]
    pub fn end_file(&self) -> &'static str {
        self.end_file
    }

    /// The line where the span begins, starting at 1.
    #[must_use]
    pub fn begin_line(&self) -> usize {
        self.begin_line
    }

    /// The line where the span ends.
    #[must_use]
    pub fn end_line(&self) -> usize {
        self.end_line
    }

    /// The column where the span begins, starting at 1.
    #[must_use]
    pub fn begin_column(&self) -> usize {
        self.begin_column
    }

    /// The column where the span ends.
    #[must_use]
    pub fn end_column(&self) -> usize {
        self.end_column
    }

    /// Whether [`Span::begin_file`] and [`Span::end_file`] are the same file.
    #[must_use]
    pub fn is_single_file(&self) -> bool {
        self.begin_file == self.end_file
    }
}

impl fmt::Display for Span {
    /// gringo's own format: `file:line:column` when the span is a single
    /// point, `file:line:column-column` when it spans one line of one file,
    /// `file:line:column-line:column` when it spans more than one line of
    /// one file, or `file:line:column-end_file:line:column` when the end is
    /// in another file (the same format [`crate::Location`]'s own private
    /// parser reads, here in reverse as a writer).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.begin_file, self.begin_line, self.begin_column
        )?;
        if !self.is_single_file() {
            return write!(
                f,
                "-{}:{}:{}",
                self.end_file, self.end_line, self.end_column
            );
        }
        if self.begin_line != self.end_line {
            return write!(f, "-{}:{}", self.end_line, self.end_column);
        }
        if self.begin_column != self.end_column {
            return write!(f, "-{}", self.end_column);
        }
        Ok(())
    }
}

/// Converts a file path to a NUL-terminated string, rejecting a path that is
/// not valid UTF-8 (as [`crate::control::Control::load`]'s own private
/// helper does).
fn path_c_string(path: &Path) -> Result<CString> {
    let text = path.to_str().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("the file path {} is not valid UTF-8", path.display()),
        )
    })?;
    crate::raw::c_str(text)
}

/// Parses `program` and calls `f` once per top-level statement, including
/// the implicit leading `#program base.` clingo always emits first.
///
/// `f` runs on the caller's thread (DESIGN S10, matching
/// [`crate::Control::ground_with`]'s own callback shape) and may borrow
/// local state; a panic inside it is caught and resumes here (S8).
///
/// # Errors
///
/// - the error `f` returned, unchanged, once every earlier call to `f`
///   succeeded: the first failure stops every later call;
/// - [`ErrorKind::Parse`] for a syntax error, with clingo's messages and
///   their positions in [`Error::messages`] (the only runtime failure this
///   call can report, remapped from clingo's own `Runtime` the same way
///   [`crate::Control::add`] already remaps its own parser failure);
/// - [`ErrorKind::Nul`] if `program` contains a NUL byte, before anything is
///   parsed (for [`parse_files`], if a path does).
///
/// A panic inside `f` is caught and resumes on the caller once this call
/// returns, rather than reaching clingo's C++ frames.
///
/// # Editing the statements
///
/// `f` receives the statement clingo is going to keep using: some statements
/// are shared. An `#edge (u,v) : a, b.` directive is delivered as one `Edge`
/// statement per pair, and clingo deep-copies the shared body for each later
/// pair only after `f` has returned, so an in-place edit of the delivered
/// node (a setter, [`Ast::set_ast`] and the rest) leaks into the statements
/// that follow. Edit a [`copy`](Ast::copy) or [`deep_copy`](Ast::deep_copy)
/// of the statement, never the statement itself.
///
/// # Deep input
///
/// Parsing does not recurse per nesting level, but the nodes it hands to
/// `f` do when they are dropped, printed, copied or compared (see
/// [`Ast`]). For input that nests thousands of levels deep, run the parse
/// and everything that touches its nodes on a thread with a large stack.
///
/// # Examples
///
/// ```
/// use clingox::ast;
///
/// let mut statements = Vec::new();
/// ast::parse_string("a.\nb :- a.\n", |node| {
///     statements.push(node.to_string());
///     Ok(())
/// })?;
/// assert_eq!(statements, ["#program base.", "a.", "b :- a."]);
/// # Ok::<(), clingox::Error>(())
/// ```
pub fn parse_string(program: &str, mut f: impl FnMut(Ast) -> Result<()>) -> Result<()> {
    let program = crate::raw::c_str(program)?;
    raw::parse_string(&program, &mut f)
}

/// Parses `files` and calls `f` once per top-level statement, as
/// [`parse_string`].
///
/// **clingo visits `files` from last to first**, each with its own leading
/// `#program base.` node (checked directly against clingo 5.8.2, nothing in the
/// header documents this either way). Standard input is read for an empty
/// `files` and for the path `-`, as clingo does and as
/// [`Control::load`](crate::Control::load) with `"-"` does; neither is
/// exercised by the doctests.
///
/// # Errors
///
/// As [`parse_string`], for a syntax error or a file that cannot be opened
/// (clingo reports both the same way, as a logged error with no further
/// distinction at this layer); [`ErrorKind::InvalidInput`] if a path is not
/// valid UTF-8; [`ErrorKind::Nul`] if a path contains a NUL byte.
///
/// # Editing the statements
///
/// As [`parse_string`]: edit a copy of the delivered statement, never the
/// statement itself.
///
/// # Deep input
///
/// As [`parse_string`]: run deeply nested input on a large-stack thread.
///
/// # Examples
///
/// ```
/// use clingox::ast;
///
/// let dir = std::env::temp_dir();
/// let file = dir.join(format!("clingox_doctest_ast_parse_files_{}.lp", std::process::id()));
/// std::fs::write(&file, "a.\n").unwrap();
/// let mut statements = Vec::new();
/// ast::parse_files(&[&file], |node| {
///     statements.push(node.to_string());
///     Ok(())
/// })?;
/// std::fs::remove_file(&file).ok();
/// assert_eq!(statements, ["#program base.", "a."]);
/// # Ok::<(), clingox::Error>(())
/// ```
pub fn parse_files(files: &[impl AsRef<Path>], mut f: impl FnMut(Ast) -> Result<()>) -> Result<()> {
    let files: Vec<CString> = files
        .iter()
        .map(|file| path_c_string(file.as_ref()))
        .collect::<Result<_>>()?;
    raw::parse_files(&files, &mut f)
}

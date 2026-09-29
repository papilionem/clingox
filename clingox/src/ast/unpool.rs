//! `Ast::unpool` and its [`Unpool`] flags (`clingo_ast_unpool`).

use std::fmt;

use super::Ast;
use crate::error::Result;
use crate::raw;

/// Which pools [`Ast::unpool`] removes. Combine flags with `|`.
///
/// # Examples
///
/// ```
/// use clingox::ast::Unpool;
///
/// let both = Unpool::CONDITION | Unpool::OTHER;
/// assert_eq!(both, Unpool::ALL);
/// assert!(both.contains(Unpool::CONDITION));
/// assert_eq!(format!("{:?}", Unpool::OTHER), "Unpool(OTHER)");
/// assert_eq!(Unpool::NONE | Unpool::OTHER, Unpool::OTHER);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Unpool(u32);

impl Unpool {
    /// No pools: the empty set, `pyclingo`'s `unpool(other=False,
    /// condition=False)`.
    ///
    /// [`Ast::unpool`] then calls back exactly once, with a node
    /// [`ptr_eq`](Ast::ptr_eq) to the input, whatever the node is and even
    /// when a pool in it is empty (where [`Unpool::ALL`] gives no call).
    /// `contains(NONE)` is true for every set, and `NONE | x == x`.
    /// Its `Debug` text is `Unpool()`.
    pub const NONE: Unpool = Unpool(0);
    /// The pools in the condition of a conditional literal.
    ///
    /// Alone, it has an effect only when the node handed to
    /// [`Ast::unpool`] is itself a `ConditionalLiteral`; on any other node it
    /// changes nothing (measured against clingo 5.8.2, and different from
    /// what the C header says).
    pub const CONDITION: Unpool = Unpool(raw::UNPOOL_CONDITION);
    /// Every pool except the condition of a conditional literal, when the
    /// node handed to [`Ast::unpool`] is one; on any other node, every pool,
    /// including those in the conditions of the conditional literals it
    /// contains.
    pub const OTHER: Unpool = Unpool(raw::UNPOOL_OTHER);
    /// `CONDITION | OTHER`: every pool.
    pub const ALL: Unpool = Unpool(raw::UNPOOL_CONDITION | raw::UNPOOL_OTHER);

    /// The flags in the order `Debug` names them.
    const NAMED: [(Unpool, &'static str); 2] =
        [(Unpool::CONDITION, "CONDITION"), (Unpool::OTHER, "OTHER")];

    /// Whether every flag of `other` is set in `self`.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::Unpool;
    ///
    /// assert!(Unpool::ALL.contains(Unpool::OTHER));
    /// assert!(!Unpool::OTHER.contains(Unpool::ALL));
    /// ```
    pub const fn contains(self, other: Unpool) -> bool {
        self.0 & other.0 == other.0
    }

    /// clingo's bitset for these flags (clingo.h:4114-4120).
    pub(crate) fn bits(self) -> u32 {
        self.0
    }
}

impl std::ops::BitOr for Unpool {
    type Output = Unpool;

    fn bitor(self, other: Unpool) -> Unpool {
        Unpool(self.0 | other.0)
    }
}

impl std::ops::BitOrAssign for Unpool {
    fn bitor_assign(&mut self, other: Unpool) {
        self.0 |= other.0;
    }
}

impl fmt::Debug for Unpool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Unpool(")?;
        let mut first = true;
        for (flag, name) in Unpool::NAMED {
            if self.contains(flag) {
                if !first {
                    f.write_str(" | ")?;
                }
                first = false;
                f.write_str(name)?;
            }
        }
        f.write_str(")")
    }
}

impl Ast {
    /// Removes the pools from this node, calling `f` once per alternative
    /// (`clingo_ast_unpool`).
    ///
    /// A pool such as `p(1;2)` stands for several statements; unpooling
    /// gives one node per combination, in clingo's cross-product order. `f`
    /// runs on the caller's thread, may borrow local state, and receives each
    /// alternative as an owned `Ast`. It is called
    ///
    /// - once per alternative when there are pools;
    /// - **once with a node [`ptr_eq`](Ast::ptr_eq) to `self`** when there is
    ///   nothing to unpool;
    /// - **zero times** when a pool is empty. That cannot come from program
    ///   text: `p(()).` holds the empty tuple `()`, and unpool calls back once
    ///   for it. An empty pool only comes from a built tree, and it prints as
    ///   `(1/0)`.
    ///
    /// `self` is not changed. Alternatives share their unchanged children
    /// with `self`, so editing one through a setter edits `self` too:
    /// [`copy`](Ast::copy) or [`deep_copy`](Ast::deep_copy) it first. `f` may
    /// call `unpool`, [`parse_string`](super::parse_string) and every other
    /// `Ast` method, setters included: clingo has computed all alternatives
    /// before the first call.
    ///
    /// See [`Unpool::CONDITION`] and [`Unpool::OTHER`] for what each flag
    /// covers; the flags act on the node type handed to this method.
    ///
    /// The number of alternatives is the product of the pool sizes, and clingo
    /// computes all of them before the first call: ten pools of ten elements
    /// in one rule body make 10^10 alternatives. Measured on Linux, 10^6
    /// alternatives take about 380 MB; beyond that clingo reports
    /// [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) when an allocation
    /// fails, or the process runs out of memory. Unpool input you do not
    /// control only with a bound on its pools, as clingo's own grounder would
    /// need.
    ///
    /// # Errors
    ///
    /// - the error `f` returned, unchanged; the first failure stops the
    ///   iteration, and `f` is not called again;
    /// - [`ErrorKind::BadAlloc`](crate::ErrorKind::BadAlloc) from clingo.
    ///
    /// A panic in `f` is caught and resumes on the caller once this call
    /// returns. There is no control, so nothing poisons. No kind check is
    /// made: a node of the wrong kind unpools without error.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::ast::{self, AstType, Unpool};
    ///
    /// let mut rule = None;
    /// ast::parse_string("p(1;2) :- q(3;4).", |node| {
    ///     if node.ast_type() == AstType::Rule {
    ///         rule = Some(node);
    ///     }
    ///     Ok(())
    /// })?;
    /// let mut alternatives = Vec::new();
    /// rule.unwrap().unpool(Unpool::ALL, |node| {
    ///     alternatives.push(node.to_string());
    ///     Ok(())
    /// })?;
    /// assert_eq!(alternatives, ["p(1) :- q(3).", "p(2) :- q(3).", "p(1) :- q(4).", "p(2) :- q(4)."]);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn unpool(&self, what: Unpool, mut f: impl FnMut(Ast) -> Result<()>) -> Result<()> {
        raw::unpool(self.as_raw(), what.bits(), &mut f)
    }
}

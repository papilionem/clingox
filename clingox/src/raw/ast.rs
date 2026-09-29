//! Wrappers for the AST value type: acquire/release/copy, identity, order,
//! hash, `Display`, the read-only attribute accessors, and the standalone parse
//! functions (`clingo.h:3388-4147`, `libclingo/src/astv2.cc`,
//! `libclingo/src/control.cc:1424-1910`).
//!
//! `clingo_ast_t` is reference counted (DESIGN S12): [`acquire`]/[`release`]
//! are the only functions that change a node's reference count, and every other
//! wrapper here either reads a node without touching it or hands back a node
//! clingo has already incremented the count of, so the safe layer's
//! `Clone`/`Drop` are the only place a count actually moves.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::NonNull;

use clingox_sys as ffi;

use super::capture::{Capture, MESSAGE_LIMIT};
use super::trampoline::{PanicSlot, Slot, fail, guard, logger};
use super::{
    ClingoErrorState, ErrorState, ast_generated, borrowed_str, c_int_of, c_str, call, fill_string,
    query,
};
use crate::Symbol;
use crate::ast::{Ast, AstType, Attribute, AttributeType, Span};
use crate::error::{Error, ErrorKind};

/// An owned reference to a clingo AST node: `Ast`'s own representation.
///
/// `NonNull` is itself `!Send + !Sync` (it wraps a raw pointer without opting
/// back into either), which is exactly what `Ast` needs (S12): clingo's own
/// refcount field is a plain, non-atomic `unsigned` (`astv2.hh:125`).
pub(crate) type RawAst = NonNull<ffi::clingo_ast_t>;

fn non_null(ptr: *mut ffi::clingo_ast_t) -> Result<RawAst, Error> {
    NonNull::new(ptr).ok_or_else(|| {
        Error::new(
            ErrorKind::Unknown,
            "clingo reported success but returned no ast",
        )
    })
}

/// Increments `ast`'s reference count (`Clone`, `clingo_ast_acquire`).
///
/// Infallible: the header documents no error code, and the C++ body
/// (`astv2.cc:238`, `++refCount_`) cannot throw.
pub(crate) fn ast_acquire(ast: RawAst) {
    // SAFETY: `ast` is a live node the caller already holds a reference to;
    // `clingo_ast_acquire` only increments a counter (clingo.h:3660-3666).
    unsafe { ffi::clingo_ast_acquire(ast.as_ptr()) }
}

/// Decrements `ast`'s reference count, freeing the node once it reaches zero
/// (`Drop`, `clingo_ast_release`).
///
/// Infallible for the same reason [`ast_acquire`] is (`astv2.cc:240-243`,
/// `--refCount_`, then the conditional `delete` in `SAST::clear`,
/// `astv2.cc:297-304`, neither of which throws here).
pub(crate) fn ast_release(ast: RawAst) {
    // SAFETY: `ast` is a live node the caller owns a reference to, and this
    // is the one reference being given up; `clingo_ast_release` cannot fail
    // (clingo.h:3667-3672).
    unsafe { ffi::clingo_ast_release(ast.as_ptr()) }
}

/// A shallow copy: a new top node whose children are shared with `ast`
/// (`astv2.cc:150-154`).
pub(crate) fn ast_copy(ast: RawAst) -> Result<RawAst, Error> {
    let mut out: *mut ffi::clingo_ast_t = std::ptr::null_mut();
    // SAFETY: `ast` is a live node, and `out` is a valid out-pointer
    // (clingo.h:3679-3685).
    query(|| unsafe { ffi::clingo_ast_copy(ast.as_ptr(), &raw mut out) })?;
    non_null(out)
}

/// A deep copy: a new top node whose children are new nodes too
/// (`astv2.cc:156-161`).
pub(crate) fn ast_deep_copy(ast: RawAst) -> Result<RawAst, Error> {
    let mut out: *mut ffi::clingo_ast_t = std::ptr::null_mut();
    // SAFETY: as `ast_copy` (clingo.h:3686-3692).
    query(|| unsafe { ffi::clingo_ast_deep_copy(ast.as_ptr(), &raw mut out) })?;
    non_null(out)
}

/// Structural, location-ignoring order (`operator<`, `astv2.cc:176-205`).
///
/// Infallible: the header documents no error code for this comparison.
pub(crate) fn ast_less_than(a: RawAst, b: RawAst) -> bool {
    // SAFETY: both are live nodes; `clingo_ast_less_than` only reads them
    // (clingo.h:3699-3704).
    unsafe { ffi::clingo_ast_less_than(a.as_ptr(), b.as_ptr()) }
}

/// Structural, location-ignoring equality (`operator==`, `astv2.cc:207-236`).
///
/// Infallible for the same reason [`ast_less_than`] is.
pub(crate) fn ast_equal(a: RawAst, b: RawAst) -> bool {
    // SAFETY: as `ast_less_than` (clingo.h:3705-3710).
    unsafe { ffi::clingo_ast_equal(a.as_ptr(), b.as_ptr()) }
}

/// A structural, location-ignoring hash, agreeing with [`ast_equal`]
/// (`astv2.cc:47-50`, `:164-174`).
///
/// Infallible: the header documents no error code for this call.
pub(crate) fn ast_hash(ast: RawAst) -> usize {
    // SAFETY: `ast` is a live node; `clingo_ast_hash` only reads it
    // (clingo.h:3711-3715).
    unsafe { ffi::clingo_ast_hash(ast.as_ptr()) }
}

/// gringo's own textual form of `ast` (`Display`).
pub(crate) fn ast_to_string(ast: RawAst) -> Result<String, Error> {
    fill_string(
        // SAFETY: `size` is a valid out-pointer (clingo.h:3722-3728).
        |size| query(|| unsafe { ffi::clingo_ast_to_string_size(ast.as_ptr(), size) }),
        // SAFETY: `fill_string` passes a buffer of exactly the size clingo
        // asked for (clingo.h:3729-3736).
        |ptr, size| query(|| unsafe { ffi::clingo_ast_to_string(ast.as_ptr(), ptr, size) }),
    )
}

/// The node's type.
///
/// `AST::type()` (`astv2.cc:249`) is a plain field read that cannot throw,
/// so the `bool` the header still returns is asserted, not propagated,
/// matching `Model::number`'s own established pattern.
pub(crate) fn ast_type(ast: RawAst) -> AstType {
    let mut raw_type = 0;
    // SAFETY: `ast` is a live node, and `raw_type` is a valid out-pointer
    // (clingo.h:3743-3749).
    let ok = unsafe { ffi::clingo_ast_get_type(ast.as_ptr(), &raw mut raw_type) };
    debug_assert!(ok, "clingo_ast_get_type only copies a field (astv2.cc:249)");
    ast_type_from_raw(raw_type)
}

/// Whether `ast` carries `attribute`.
///
/// `AST::hasValue` (`astv2.cc:117`) is a linear scan over the node's own
/// attributes that cannot throw, asserted as [`ast_type`] is.
pub(crate) fn ast_has_attribute(ast: RawAst, attribute: Attribute) -> bool {
    let mut result = false;
    // SAFETY: `ast` is a live node, and `result` is a valid out-pointer
    // (clingo.h:3750-3758).
    let ok = unsafe {
        ffi::clingo_ast_has_attribute(ast.as_ptr(), attribute_to_raw(attribute), &raw mut result)
    };
    debug_assert!(
        ok,
        "clingo_ast_has_attribute only scans the node's own attributes (astv2.cc:117)"
    );
    result
}

/// The kind of `attribute` on `ast`.
///
/// Only ever called once the safe layer's [`crate::ast::Ast::has_attribute`]
/// has confirmed the attribute is present, since an absent attribute is the
/// only way this call can fail (`astv2.cc:119-130`).
pub(crate) fn ast_attribute_type(
    ast: RawAst,
    attribute: Attribute,
) -> Result<AttributeType, Error> {
    let mut raw_type = 0;
    // SAFETY: `ast` is a live node, and `raw_type` is a valid out-pointer
    // (clingo.h:3759-3767).
    query(|| unsafe {
        ffi::clingo_ast_attribute_type(ast.as_ptr(), attribute_to_raw(attribute), &raw mut raw_type)
    })?;
    Ok(attribute_type_from_raw(raw_type))
}

/// Reads a `number` attribute.
pub(crate) fn ast_attribute_get_number(ast: RawAst, attribute: Attribute) -> Result<i32, Error> {
    let mut value = 0;
    // SAFETY: `ast` is a live node with `attribute` confirmed of type
    // `number` by the caller, and `value` is a valid out-pointer
    // (clingo.h:3774-3782).
    query(|| unsafe {
        ffi::clingo_ast_attribute_get_number(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            &raw mut value,
        )
    })?;
    Ok(value)
}

/// Reads a `symbol` attribute.
pub(crate) fn ast_attribute_get_symbol(ast: RawAst, attribute: Attribute) -> Result<Symbol, Error> {
    let mut value: ffi::clingo_symbol_t = 0;
    // SAFETY: as `ast_attribute_get_number`, for a `symbol`-typed attribute
    // (clingo.h:3798-3806).
    query(|| unsafe {
        ffi::clingo_ast_attribute_get_symbol(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            &raw mut value,
        )
    })?;
    Ok(Symbol::from_clingo(value))
}

/// Reads a `location` attribute.
pub(crate) fn ast_attribute_get_location(ast: RawAst, attribute: Attribute) -> Result<Span, Error> {
    let mut value = ffi::clingo_location_t {
        begin_file: std::ptr::null(),
        end_file: std::ptr::null(),
        begin_line: 0,
        end_line: 0,
        begin_column: 0,
        end_column: 0,
    };
    // SAFETY: as `ast_attribute_get_number`, for a `location`-typed
    // attribute (clingo.h:3822-3830).
    query(|| unsafe {
        ffi::clingo_ast_attribute_get_location(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            &raw mut value,
        )
    })?;
    span_from_raw(value)
}

/// Reads a `string` attribute, copied into an owned `String`.
///
/// Strict UTF-8: invalid bytes are `ErrorKind::Utf8`, never a
/// lossy conversion.
pub(crate) fn ast_attribute_get_string(ast: RawAst, attribute: Attribute) -> Result<String, Error> {
    let mut value: *const c_char = std::ptr::null();
    // SAFETY: as `ast_attribute_get_number`, for a `string`-typed attribute
    // (clingo.h:3846-3854).
    query(|| unsafe {
        ffi::clingo_ast_attribute_get_string(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            &raw mut value,
        )
    })?;
    // SAFETY: `value` is a NUL-terminated string owned by `ast`'s own
    // attribute storage (`String::c_str()`, `control.cc:1677-1681`), valid
    // for at least the duration of this call, which copies it at once.
    let text = unsafe { borrowed_str(value) };
    text.map(str::to_owned)
}

/// Reads an `ast` attribute: a new, independently owned child (clingo `incRef`s
/// it before returning, `control.cc:1708-1715`).
pub(crate) fn ast_attribute_get_ast(ast: RawAst, attribute: Attribute) -> Result<RawAst, Error> {
    let mut out: *mut ffi::clingo_ast_t = std::ptr::null_mut();
    // SAFETY: as `ast_attribute_get_number`, for an `ast`-typed attribute
    // (clingo.h:3870-3878).
    query(|| unsafe {
        ffi::clingo_ast_attribute_get_ast(ast.as_ptr(), attribute_to_raw(attribute), &raw mut out)
    })?;
    non_null(out)
}

/// Reads an `optional_ast` attribute: `None` for clingo's own null value
/// (`control.cc:1689-1698`), otherwise an owned child as
/// [`ast_attribute_get_ast`].
pub(crate) fn ast_attribute_get_optional_ast(
    ast: RawAst,
    attribute: Attribute,
) -> Result<Option<RawAst>, Error> {
    let mut out: *mut ffi::clingo_ast_t = std::ptr::null_mut();
    // SAFETY: as `ast_attribute_get_ast`; a null `out` is a documented,
    // valid "no value" here (clingo.h:3894-3904).
    query(|| unsafe {
        ffi::clingo_ast_attribute_get_optional_ast(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            &raw mut out,
        )
    })?;
    Ok(NonNull::new(out))
}

/// Reads one element of a `string_array` attribute.
///
/// The caller has already checked `index` against
/// [`ast_attribute_size_string_array`] (clingox's own bounds check runs before
/// this call, since `std::vector::at`'s own `Logic` error must never reach a
/// caller here).
pub(crate) fn ast_attribute_get_string_at(
    ast: RawAst,
    attribute: Attribute,
    index: usize,
) -> Result<String, Error> {
    let mut value: *const c_char = std::ptr::null();
    // SAFETY: as `ast_attribute_get_string`, with `index` already checked
    // by the caller (clingo.h:3922-3931).
    query(|| unsafe {
        ffi::clingo_ast_attribute_get_string_at(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            index,
            &raw mut value,
        )
    })?;
    // SAFETY: `value` is a NUL-terminated string owned by `ast`'s own array
    // storage (`StrVec::at(index).c_str()`, `control.cc:1727-1731`), valid
    // for at least the duration of this call, which copies it at once.
    let text = unsafe { borrowed_str(value) };
    text.map(str::to_owned)
}

/// Reads one element of an `ast_array` attribute, an owned child as
/// [`ast_attribute_get_ast`]. As [`ast_attribute_get_string_at`], the caller
/// has already checked `index`.
pub(crate) fn ast_attribute_get_ast_at(
    ast: RawAst,
    attribute: Attribute,
    index: usize,
) -> Result<RawAst, Error> {
    let mut out: *mut ffi::clingo_ast_t = std::ptr::null_mut();
    // SAFETY: as `ast_attribute_get_ast`, with `index` already checked by
    // the caller (clingo.h:3979-3988).
    query(|| unsafe {
        ffi::clingo_ast_attribute_get_ast_at(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            index,
            &raw mut out,
        )
    })?;
    non_null(out)
}

/// The length of a `string_array` attribute.
pub(crate) fn ast_attribute_size_string_array(
    ast: RawAst,
    attribute: Attribute,
) -> Result<usize, Error> {
    let mut size = 0;
    // SAFETY: `ast` is a live node with `attribute` confirmed of type
    // `string_array`, and `size` is a valid out-pointer (clingo.h:3952-3960).
    query(|| unsafe {
        ffi::clingo_ast_attribute_size_string_array(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            &raw mut size,
        )
    })?;
    Ok(size)
}

/// The length of an `ast_array` attribute.
pub(crate) fn ast_attribute_size_ast_array(
    ast: RawAst,
    attribute: Attribute,
) -> Result<usize, Error> {
    let mut size = 0;
    // SAFETY: as `ast_attribute_size_string_array`, for an `ast_array`-typed
    // attribute (clingo.h:4009-4017).
    query(|| unsafe {
        ffi::clingo_ast_attribute_size_ast_array(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            &raw mut size,
        )
    })?;
    Ok(size)
}

// ---------------------------------------------------------------------------
// Enum conversions (S17: matched by bindgen constant, never by literal)
// ---------------------------------------------------------------------------

/// Maps `clingo_ast_type_e` (clingo.h:3481-3535) to [`AstType`], in the
/// header's own declaration order. A value this build does not know is
/// [`AstType::Other`] (S17): only reachable by reading an already-existing
/// node from a newer clingo, since nothing in the crate ever constructs one.
fn ast_type_from_raw(raw: ffi::clingo_ast_type_t) -> AstType {
    match u32::try_from(raw) {
        Ok(ffi::clingo_ast_type_id) => AstType::Id,
        Ok(ffi::clingo_ast_type_variable) => AstType::Variable,
        Ok(ffi::clingo_ast_type_symbolic_term) => AstType::SymbolicTerm,
        Ok(ffi::clingo_ast_type_unary_operation) => AstType::UnaryOperation,
        Ok(ffi::clingo_ast_type_binary_operation) => AstType::BinaryOperation,
        Ok(ffi::clingo_ast_type_interval) => AstType::Interval,
        Ok(ffi::clingo_ast_type_function) => AstType::Function,
        Ok(ffi::clingo_ast_type_pool) => AstType::Pool,
        Ok(ffi::clingo_ast_type_boolean_constant) => AstType::BooleanConstant,
        Ok(ffi::clingo_ast_type_symbolic_atom) => AstType::SymbolicAtom,
        Ok(ffi::clingo_ast_type_comparison) => AstType::Comparison,
        Ok(ffi::clingo_ast_type_guard) => AstType::Guard,
        Ok(ffi::clingo_ast_type_conditional_literal) => AstType::ConditionalLiteral,
        Ok(ffi::clingo_ast_type_aggregate) => AstType::Aggregate,
        Ok(ffi::clingo_ast_type_body_aggregate_element) => AstType::BodyAggregateElement,
        Ok(ffi::clingo_ast_type_body_aggregate) => AstType::BodyAggregate,
        Ok(ffi::clingo_ast_type_head_aggregate_element) => AstType::HeadAggregateElement,
        Ok(ffi::clingo_ast_type_head_aggregate) => AstType::HeadAggregate,
        Ok(ffi::clingo_ast_type_disjunction) => AstType::Disjunction,
        Ok(ffi::clingo_ast_type_theory_sequence) => AstType::TheorySequence,
        Ok(ffi::clingo_ast_type_theory_function) => AstType::TheoryFunction,
        Ok(ffi::clingo_ast_type_theory_unparsed_term_element) => AstType::TheoryUnparsedTermElement,
        Ok(ffi::clingo_ast_type_theory_unparsed_term) => AstType::TheoryUnparsedTerm,
        Ok(ffi::clingo_ast_type_theory_guard) => AstType::TheoryGuard,
        Ok(ffi::clingo_ast_type_theory_atom_element) => AstType::TheoryAtomElement,
        Ok(ffi::clingo_ast_type_theory_atom) => AstType::TheoryAtom,
        Ok(ffi::clingo_ast_type_literal) => AstType::Literal,
        Ok(ffi::clingo_ast_type_theory_operator_definition) => AstType::TheoryOperatorDefinition,
        Ok(ffi::clingo_ast_type_theory_term_definition) => AstType::TheoryTermDefinition,
        Ok(ffi::clingo_ast_type_theory_guard_definition) => AstType::TheoryGuardDefinition,
        Ok(ffi::clingo_ast_type_theory_atom_definition) => AstType::TheoryAtomDefinition,
        Ok(ffi::clingo_ast_type_rule) => AstType::Rule,
        Ok(ffi::clingo_ast_type_definition) => AstType::Definition,
        Ok(ffi::clingo_ast_type_show_signature) => AstType::ShowSignature,
        Ok(ffi::clingo_ast_type_show_term) => AstType::ShowTerm,
        Ok(ffi::clingo_ast_type_minimize) => AstType::Minimize,
        Ok(ffi::clingo_ast_type_script) => AstType::Script,
        Ok(ffi::clingo_ast_type_program) => AstType::Program,
        Ok(ffi::clingo_ast_type_external) => AstType::External,
        Ok(ffi::clingo_ast_type_edge) => AstType::Edge,
        Ok(ffi::clingo_ast_type_heuristic) => AstType::Heuristic,
        Ok(ffi::clingo_ast_type_project_atom) => AstType::ProjectAtom,
        Ok(ffi::clingo_ast_type_project_signature) => AstType::ProjectSignature,
        Ok(ffi::clingo_ast_type_defined) => AstType::Defined,
        Ok(ffi::clingo_ast_type_theory_definition) => AstType::TheoryDefinition,
        Ok(ffi::clingo_ast_type_comment) => AstType::Comment,
        _ => AstType::Other(raw),
    }
}

/// Maps [`Attribute`] to `clingo_ast_attribute_e` (clingo.h:3554-3600), in the
/// header's own declaration order (which is also
/// `g_clingo_ast_attribute_names`'s own order, checked against the live
/// oracle).
fn attribute_to_raw(attribute: Attribute) -> ffi::clingo_ast_attribute_t {
    let raw = match attribute {
        Attribute::Argument => ffi::clingo_ast_attribute_argument,
        Attribute::Arguments => ffi::clingo_ast_attribute_arguments,
        Attribute::Arity => ffi::clingo_ast_attribute_arity,
        Attribute::Atom => ffi::clingo_ast_attribute_atom,
        Attribute::Atoms => ffi::clingo_ast_attribute_atoms,
        Attribute::AtomType => ffi::clingo_ast_attribute_atom_type,
        Attribute::Bias => ffi::clingo_ast_attribute_bias,
        Attribute::Body => ffi::clingo_ast_attribute_body,
        Attribute::Code => ffi::clingo_ast_attribute_code,
        Attribute::Coefficient => ffi::clingo_ast_attribute_coefficient,
        Attribute::Comparison => ffi::clingo_ast_attribute_comparison,
        Attribute::Condition => ffi::clingo_ast_attribute_condition,
        Attribute::Elements => ffi::clingo_ast_attribute_elements,
        Attribute::External => ffi::clingo_ast_attribute_external,
        Attribute::ExternalType => ffi::clingo_ast_attribute_external_type,
        Attribute::Function => ffi::clingo_ast_attribute_function,
        Attribute::Guard => ffi::clingo_ast_attribute_guard,
        Attribute::Guards => ffi::clingo_ast_attribute_guards,
        Attribute::Head => ffi::clingo_ast_attribute_head,
        Attribute::IsDefault => ffi::clingo_ast_attribute_is_default,
        Attribute::Left => ffi::clingo_ast_attribute_left,
        Attribute::LeftGuard => ffi::clingo_ast_attribute_left_guard,
        Attribute::Literal => ffi::clingo_ast_attribute_literal,
        Attribute::Location => ffi::clingo_ast_attribute_location,
        Attribute::Modifier => ffi::clingo_ast_attribute_modifier,
        Attribute::Name => ffi::clingo_ast_attribute_name,
        Attribute::NodeU => ffi::clingo_ast_attribute_node_u,
        Attribute::NodeV => ffi::clingo_ast_attribute_node_v,
        Attribute::OperatorName => ffi::clingo_ast_attribute_operator_name,
        Attribute::OperatorType => ffi::clingo_ast_attribute_operator_type,
        Attribute::Operators => ffi::clingo_ast_attribute_operators,
        Attribute::Parameters => ffi::clingo_ast_attribute_parameters,
        Attribute::Positive => ffi::clingo_ast_attribute_positive,
        Attribute::Priority => ffi::clingo_ast_attribute_priority,
        Attribute::Right => ffi::clingo_ast_attribute_right,
        Attribute::RightGuard => ffi::clingo_ast_attribute_right_guard,
        Attribute::SequenceType => ffi::clingo_ast_attribute_sequence_type,
        Attribute::Sign => ffi::clingo_ast_attribute_sign,
        Attribute::Symbol => ffi::clingo_ast_attribute_symbol,
        Attribute::Term => ffi::clingo_ast_attribute_term,
        Attribute::Terms => ffi::clingo_ast_attribute_terms,
        Attribute::Value => ffi::clingo_ast_attribute_value,
        Attribute::Variable => ffi::clingo_ast_attribute_variable,
        Attribute::Weight => ffi::clingo_ast_attribute_weight,
        Attribute::CommentType => ffi::clingo_ast_attribute_comment_type,
    };
    c_int_of(raw)
}

/// Maps `clingo_ast_attribute_type_e` (clingo.h:3540-3549) to
/// [`AttributeType`].
///
/// `AttributeType` names exactly the 8 kinds this header defines (no
/// `Other`, unlike [`AstType`]/[`Attribute`]): a value outside them can only
/// come from `clingo_ast_attribute_type` succeeding, which only ever
/// happens after clingox's own `has_attribute` check has already confirmed
/// the query is well-formed. An out-of-range value
/// here would mean this build's bindings and the linked clingo disagree
/// about the 8-kind enum itself, which the version check (S18) already
/// rules out for the pinned 5.8.x range this crate accepts.
fn attribute_type_from_raw(raw: ffi::clingo_ast_attribute_type_t) -> AttributeType {
    match u32::try_from(raw) {
        Ok(ffi::clingo_ast_attribute_type_number) => AttributeType::Number,
        Ok(ffi::clingo_ast_attribute_type_symbol) => AttributeType::Symbol,
        Ok(ffi::clingo_ast_attribute_type_location) => AttributeType::Location,
        Ok(ffi::clingo_ast_attribute_type_string) => AttributeType::String,
        Ok(ffi::clingo_ast_attribute_type_ast) => AttributeType::Ast,
        Ok(ffi::clingo_ast_attribute_type_optional_ast) => AttributeType::OptionalAst,
        Ok(ffi::clingo_ast_attribute_type_string_array) => AttributeType::StringArray,
        Ok(ffi::clingo_ast_attribute_type_ast_array) => AttributeType::AstArray,
        _ => unreachable!(
            "clingo reported attribute type {raw}, outside the 8 kinds \
             clingo_ast_attribute_type_e defines for the pinned 5.8.x range"
        ),
    }
}

/// Converts a `clingo_location_t` to a [`Span`]. The file names are read as
/// process-lifetime interned strings, the same flyweight `Signature::name`
/// already uses.
///
/// Strict UTF-8: a file name that is not UTF-8, such
/// as an `#include` of a file whose name is not, is `ErrorKind::Utf8`. A lossy
/// name would match no file. A null name, which clingo never reports for a
/// parsed node, reads as the empty name.
fn span_from_raw(loc: ffi::clingo_location_t) -> Result<Span, Error> {
    let file = |ptr: *const c_char| -> Result<&'static str, Error> {
        if ptr.is_null() {
            return Ok("");
        }
        // SAFETY: location file names are internalized and valid for the
        // duration of the process (clingo.h:205-209, `libgringo/gringo/
        // locatable.hh:44-45`); `ptr` is non-null.
        unsafe { borrowed_str(ptr) }
    };
    Ok(Span::from_raw_parts(
        file(loc.begin_file)?,
        file(loc.end_file)?,
        loc.begin_line,
        loc.end_line,
        loc.begin_column,
        loc.end_column,
    ))
}

// ---------------------------------------------------------------------------
// parse_string / parse_files
// ---------------------------------------------------------------------------

/// The user's function behind `parse_string`/`parse_files`: it receives each
/// top-level statement the parser reports.
pub(crate) type ParseFunction<'f> = dyn FnMut(Ast) -> Result<(), Error> + 'f;

/// What the data pointer of the parse callback points to (DESIGN S8).
///
/// Parsing runs on the thread that called `parse_string`/`parse_files`, so
/// the function is borrowed and need not be `Send` (S10), mirroring
/// `raw::trampoline::GroundContext`.
struct ParseContext<'f> {
    function: RefCell<&'f mut ParseFunction<'f>>,
    error: Slot<Error>,
    panic: PanicSlot,
}

impl<'f> ParseContext<'f> {
    fn new(function: &'f mut ParseFunction<'f>) -> Self {
        ParseContext {
            function: RefCell::new(function),
            error: Slot::default(),
            panic: PanicSlot::default(),
        }
    }

    fn call(&self, ast: Ast) -> Result<(), Error> {
        // Nothing reachable from the function can parse anything else on
        // this context, since `run_parse` borrows it for the one call; the
        // error only guards that reasoning (mirrors `GroundContext::call`).
        let mut function = self.function.try_borrow_mut().map_err(|_| {
            Error::new(
                ErrorKind::Logic,
                "a parse callback was entered while it was running",
            )
        })?;
        (*function)(ast)
    }
}

/// The Rust-only part of the parse/unpool trampoline: runs the user's
/// function inside [`guard`], recording its error or panic in `context`'s
/// slots (S8). Separated from [`parse_function`] so it can run under Miri
/// with a fake `Ast` and a fake [`ErrorState`], without linking clingo: the
/// `clingo_ast_acquire` call that must happen before an `Ast` is built at
/// all is real FFI and stays in [`parse_function`], exercised instead by
/// the ASan/LSan integration test.
fn run_parse_callback<S: ErrorState>(context: &ParseContext<'_>, ast: Ast) -> bool {
    if context.error.is_set() {
        fail::<S>(c"an earlier parse callback failed");
        return false;
    }
    match guard(&context.panic, || context.call(ast)) {
        Some(Ok(())) => true,
        Some(Err(err)) => {
            context.error.store(err);
            fail::<S>(c"a parse callback returned an error");
            false
        }
        None => {
            fail::<S>(c"a parse callback panicked");
            false
        }
    }
}

/// The `clingo_ast_callback_t` trampoline for `parse_string`/`parse_files`.
///
/// # Safety
///
/// `data` must point to a live [`ParseContext`]. `ast`, if non-null, is a
/// node whose reference count clingo has **not** incremented for this call
/// (`clingo_ast_acquire`'s own header note, clingo.h:3662-3663, confirmed
/// for parsing specifically at `control.cc:1826`/`:1851`): this trampoline
/// acquires its own reference before building the owned `Ast` the closure
/// receives, exactly once, before any other use of `ast`.
pub(crate) unsafe extern "C" fn parse_function<S: ErrorState>(
    ast: *mut ffi::clingo_ast_t,
    data: *mut c_void,
) -> bool {
    // SAFETY: `data` is the context `run_parse` registered for this call,
    // which outlives it (the caller's contract). Parsing runs on one
    // thread, and only shared access is taken.
    let context = unsafe { &*data.cast::<ParseContext<'_>>().cast_const() };
    let Some(ptr) = NonNull::new(ast) else {
        fail::<S>(c"clingo passed a null ast to the parse callback");
        return false;
    };
    // SAFETY: `ptr` is the borrowed node clingo just handed this callback
    // (the caller's contract); acquiring our own reference before wrapping
    // it keeps the owned `Ast` below sound to drop.
    unsafe { ffi::clingo_ast_acquire(ptr.as_ptr()) };
    run_parse_callback::<S>(context, Ast::from_raw(ptr))
}

/// Runs `call_ffi` with a fresh [`ParseContext`] and a fresh, throwaway
/// [`Capture`], and resolves the precedence common to `parse_string` and
/// `parse_files` (S8): a caught panic in the callback resumes first; a
/// callback error, with clingo's own logged messages attached, wins over
/// clingo's own generic report that "the callback failed"; otherwise
/// clingo's own result is reported, messages attached, a `Runtime` failure
/// remapped to `Parse` (the only runtime error either function
/// raises is a syntax error or a file that could not be opened, both from
/// the same `log.hasError()` path `Control::add` already remaps).
///
/// `call_ffi` receives the parse context's data pointer and the capture's
/// logger data pointer, already prepared for `clingo_ast_parse_string`/
/// `_files`'s own trailing parameters.
fn run_parse(
    f: &mut ParseFunction<'_>,
    call_ffi: impl FnOnce(*mut c_void, *mut c_void) -> bool,
) -> Result<(), Error> {
    let context = ParseContext::new(f);
    let data = std::ptr::from_ref(&context).cast_mut().cast::<c_void>();
    let capture = Capture::default();
    let logger_data = std::ptr::from_ref(&capture).cast_mut().cast::<c_void>();
    let result = call(|| call_ffi(data, logger_data));
    capture.resume_panic();
    let messages = capture.take();
    context.panic.resume();
    match context.error.take() {
        Some(err) => Err(err.with_messages(messages)),
        None => result.map_err(|err| {
            let err = match err.kind() {
                ErrorKind::Runtime => err.with_kind(ErrorKind::Parse),
                _ => err,
            };
            err.with_messages(messages)
        }),
    }
}

/// Parses `program` and calls `f` once per top-level statement
/// (`clingo_ast_parse_string`, no `control`: `H:4043`'s own note that a null
/// control only disables reading input in aspif format).
pub(crate) fn parse_string(program: &CStr, f: &mut ParseFunction<'_>) -> Result<(), Error> {
    run_parse(f, |data, logger_data| {
        // SAFETY: `program` is a NUL-terminated string outliving the call.
        // `data` points to the live `ParseContext` `run_parse` just built,
        // and `logger_data` to its live `Capture`, both outliving the call
        // and not moved while borrowed. `control` is null, which the
        // header documents as valid (clingo.h:4043).
        unsafe {
            ffi::clingo_ast_parse_string(
                program.as_ptr(),
                Some(parse_function::<ClingoErrorState>),
                data,
                std::ptr::null_mut(),
                Some(logger::<Capture>),
                logger_data,
                MESSAGE_LIMIT,
            )
        }
    })
}

/// Parses `files` in clingo's own order (last to first)
/// and calls `f` once per top-level statement (`clingo_ast_parse_files`).
pub(crate) fn parse_files(files: &[CString], f: &mut ParseFunction<'_>) -> Result<(), Error> {
    let pointers: Vec<*const c_char> = files.iter().map(|file| file.as_ptr()).collect();
    run_parse(f, |data, logger_data| {
        // SAFETY: `pointers` holds `pointers.len()` NUL-terminated strings
        // outliving the call (borrowed from `files`, which outlives this
        // function). `data`, `logger_data` and `control` as `parse_string`.
        unsafe {
            ffi::clingo_ast_parse_files(
                pointers.as_ptr(),
                pointers.len(),
                Some(parse_function::<ClingoErrorState>),
                data,
                std::ptr::null_mut(),
                Some(logger::<Capture>),
                logger_data,
                MESSAGE_LIMIT,
            )
        }
    })
}

/// `clingo_ast_unpool_type_condition` (clingo.h:4100).
pub(crate) const UNPOOL_CONDITION: u32 = ffi::clingo_ast_unpool_type_condition;
/// `clingo_ast_unpool_type_other` (clingo.h:4100).
pub(crate) const UNPOOL_OTHER: u32 = ffi::clingo_ast_unpool_type_other;

/// Unpools `ast` and calls `f` once per alternative
/// (`clingo_ast_unpool`).
///
/// The trampoline is [`parse_function`], which acquires each borrowed node
/// before wrapping it. The precedence is `run_parse`'s, but with no logger
/// and no `Runtime`-to-`Parse` remap: unpooling raises only `BadAlloc`
/// (clingo.h:4137-4142). A caught panic in `f` resumes first; then `f`'s own
/// error, which wins over clingo's generic report that a callback failed;
/// then clingo's result.
pub(crate) fn unpool(ast: RawAst, what: u32, f: &mut ParseFunction<'_>) -> Result<(), Error> {
    let context = ParseContext::new(f);
    let data = std::ptr::from_ref(&context).cast_mut().cast::<c_void>();
    let result = call(|| {
        // SAFETY: `ast` is a live node the caller borrows for this call.
        // `data` points to `context`, which outlives the call and is not
        // moved while borrowed; the callback runs on this thread only, and
        // clingo has finished computing the alternatives before the first
        // report (control.cc:1886).
        unsafe {
            ffi::clingo_ast_unpool(
                ast.as_ptr(),
                c_int_of(what),
                Some(parse_function::<ClingoErrorState>),
                data,
            )
        }
    });
    context.panic.resume();
    match context.error.take() {
        Some(err) => Err(err),
        None => result,
    }
}

// ---------------------------------------------------------------------------
// Building nodes
// ---------------------------------------------------------------------------

/// A `clingo_location_t` together with the C strings its two file pointers
/// point to: clingo copies (interns) both while reading the location, so they
/// only need to outlive the call the location is passed to.
struct RawLocation {
    raw: ffi::clingo_location_t,
    _files: [CString; 2],
}

impl RawLocation {
    fn new(span: &Span) -> Result<RawLocation, Error> {
        let begin = c_str(span.begin_file())?;
        let end = c_str(span.end_file())?;
        Ok(RawLocation {
            raw: ffi::clingo_location_t {
                begin_file: begin.as_ptr(),
                end_file: end.as_ptr(),
                begin_line: span.begin_line(),
                end_line: span.end_line(),
                begin_column: span.begin_column(),
                end_column: span.end_column(),
            },
            _files: [begin, end],
        })
    }
}

/// One argument of a node constructor, in the safe layer's terms. The
/// generated constructors pass one per entry of clingo's constructor table.
pub(crate) enum BuildArg<'a> {
    Number(i32),
    Symbol(Symbol),
    Span(&'a Span),
    Str(&'a str),
    Ast(&'a Ast),
    OptionalAst(Option<&'a Ast>),
    StrArray(&'a [&'a str]),
    AstArray(&'a [Ast]),
}

/// The value of one [`BuildArg`], in the form `clingo_ast_build` reads it.
///
/// Everything a pointer points to is owned here (C strings, arrays, the
/// location) or borrowed from the caller's `Ast` handles for the length of
/// the call; moving an item moves only its handle, never the heap data behind
/// the pointers, so a pointer taken after `Prepared::new` returns stays valid
/// for as long as the `Prepared` does.
enum Item {
    Number(c_int),
    Symbol(ffi::clingo_symbol_t),
    Location(Box<RawLocation>),
    String(CString),
    Ast(*mut ffi::clingo_ast_t),
    OptionalAst(*mut ffi::clingo_ast_t),
    Strings {
        _owned: Vec<CString>,
        pointers: Vec<*const c_char>,
    },
    Asts(Vec<*mut ffi::clingo_ast_t>),
}

/// The C values of a constructor's arguments, read by index by the generated
/// `ast_generated::build`.
pub(super) struct Prepared {
    items: Vec<Item>,
}

impl Prepared {
    fn new(args: &[BuildArg<'_>]) -> Result<Prepared, Error> {
        let mut items = Vec::with_capacity(args.len());
        for arg in args {
            items.push(match arg {
                BuildArg::Number(value) => Item::Number(*value),
                BuildArg::Symbol(symbol) => Item::Symbol(symbol.raw()),
                BuildArg::Span(span) => Item::Location(Box::new(RawLocation::new(span)?)),
                BuildArg::Str(text) => Item::String(c_str(text)?),
                BuildArg::Ast(node) => Item::Ast(node.as_raw().as_ptr()),
                BuildArg::OptionalAst(node) => Item::OptionalAst(
                    node.map_or(std::ptr::null_mut(), |node| node.as_raw().as_ptr()),
                ),
                BuildArg::StrArray(texts) => {
                    let owned = texts
                        .iter()
                        .map(|text| c_str(text))
                        .collect::<Result<Vec<_>, _>>()?;
                    let pointers = owned.iter().map(|text| text.as_ptr()).collect();
                    Item::Strings {
                        _owned: owned,
                        pointers,
                    }
                }
                BuildArg::AstArray(nodes) => {
                    Item::Asts(nodes.iter().map(|node| node.as_raw().as_ptr()).collect())
                }
            });
        }
        Ok(Prepared { items })
    }

    /// The item at `index`. A wrong index or kind is a bug in the generated
    /// code, which is produced from the same table as the safe constructors,
    /// and panics here before any pointer is read.
    fn item(&self, index: usize, wanted: &str) -> &Item {
        match self.items.get(index) {
            Some(item) => item,
            None => panic!("the constructor has no argument {index} (wanted {wanted})"),
        }
    }

    fn mismatch(index: usize, wanted: &str) -> ! {
        panic!("argument {index} of the constructor is not a {wanted}")
    }

    pub(super) fn number(&self, index: usize) -> c_int {
        match self.item(index, "number") {
            Item::Number(value) => *value,
            _ => Self::mismatch(index, "number"),
        }
    }

    pub(super) fn symbol(&self, index: usize) -> ffi::clingo_symbol_t {
        match self.item(index, "symbol") {
            Item::Symbol(value) => *value,
            _ => Self::mismatch(index, "symbol"),
        }
    }

    pub(super) fn location(&self, index: usize) -> *const ffi::clingo_location_t {
        match self.item(index, "location") {
            Item::Location(location) => &raw const location.raw,
            _ => Self::mismatch(index, "location"),
        }
    }

    pub(super) fn string(&self, index: usize) -> *const c_char {
        match self.item(index, "string") {
            Item::String(text) => text.as_ptr(),
            _ => Self::mismatch(index, "string"),
        }
    }

    pub(super) fn ast(&self, index: usize) -> *mut ffi::clingo_ast_t {
        match self.item(index, "ast") {
            Item::Ast(node) => *node,
            _ => Self::mismatch(index, "ast"),
        }
    }

    pub(super) fn optional_ast(&self, index: usize) -> *mut ffi::clingo_ast_t {
        match self.item(index, "optional ast") {
            Item::OptionalAst(node) => *node,
            _ => Self::mismatch(index, "optional ast"),
        }
    }

    pub(super) fn string_array(&self, index: usize) -> *const *const c_char {
        match self.item(index, "string array") {
            Item::Strings { pointers, .. } => pointers.as_ptr(),
            _ => Self::mismatch(index, "string array"),
        }
    }

    pub(super) fn string_array_len(&self, index: usize) -> usize {
        match self.item(index, "string array") {
            Item::Strings { pointers, .. } => pointers.len(),
            _ => Self::mismatch(index, "string array"),
        }
    }

    pub(super) fn ast_array(&self, index: usize) -> *const *mut ffi::clingo_ast_t {
        match self.item(index, "ast array") {
            Item::Asts(nodes) => nodes.as_ptr(),
            _ => Self::mismatch(index, "ast array"),
        }
    }

    pub(super) fn ast_array_len(&self, index: usize) -> usize {
        match self.item(index, "ast array") {
            Item::Asts(nodes) => nodes.len(),
            _ => Self::mismatch(index, "ast array"),
        }
    }
}

/// Builds a node of type `kind` (`clingo_ast_build`), through the generated
/// call that knows each type's argument list. The new node is owned by the
/// caller, and holds its own reference to every node argument.
pub(crate) fn ast_build(kind: AstType, args: &[BuildArg<'_>]) -> Result<RawAst, Error> {
    if let AstType::Other(raw) = kind {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("clingox cannot build a node of the unknown type {raw}"),
        ));
    }
    let prepared = Prepared::new(args)?;
    let mut out: *mut ffi::clingo_ast_t = std::ptr::null_mut();
    call(|| ast_generated::build(kind, &prepared, &mut out))?;
    non_null(out)
}

/// Interns `text` with `clingo_add_string`: the same flyweight clingo keeps
/// location file names in, valid for the rest of the process
/// (`clingo.h:129-133`).
pub(crate) fn intern(text: &str) -> Result<&'static str, Error> {
    let text = c_str(text)?;
    let mut result: *const c_char = std::ptr::null();
    // SAFETY: `text` is NUL-terminated and outlives the call, and `result` is
    // a valid out-pointer; clingo copies the string into its table, which is
    // never freed (clingo.h:497).
    call(|| unsafe { ffi::clingo_add_string(text.as_ptr(), &raw mut result) })?;
    // SAFETY: the interned string is valid, unchanged and NUL-terminated for
    // the rest of the process, and it is valid UTF-8 because `text` was.
    unsafe { borrowed_str(result) }
}

// ---------------------------------------------------------------------------
// Setters and array editors
//
// Every function here assumes the safe layer has already checked the
// attribute's presence and kind, the value, the cycle and the index, in that
// order: they are the thin calls.
// ---------------------------------------------------------------------------

/// Writes a `number` attribute.
pub(crate) fn ast_attribute_set_number(
    ast: RawAst,
    attribute: Attribute,
    value: i32,
) -> Result<(), Error> {
    // SAFETY: `ast` is a live node with `attribute` confirmed of type
    // `number` by the caller (clingo.h:3790).
    call(|| unsafe {
        ffi::clingo_ast_attribute_set_number(ast.as_ptr(), attribute_to_raw(attribute), value)
    })
}

/// Writes a `symbol` attribute.
pub(crate) fn ast_attribute_set_symbol(
    ast: RawAst,
    attribute: Attribute,
    value: Symbol,
) -> Result<(), Error> {
    // SAFETY: as `ast_attribute_set_number`, for a `symbol`-typed attribute
    // (clingo.h:3814).
    call(|| unsafe {
        ffi::clingo_ast_attribute_set_symbol(ast.as_ptr(), attribute_to_raw(attribute), value.raw())
    })
}

/// Writes a `location` attribute.
pub(crate) fn ast_attribute_set_location(
    ast: RawAst,
    attribute: Attribute,
    value: &Span,
) -> Result<(), Error> {
    let location = RawLocation::new(value)?;
    // SAFETY: as `ast_attribute_set_number`, for a `location`-typed
    // attribute; `location` and the strings it points to outlive the call,
    // and clingo copies them (clingo.h:3838).
    call(|| unsafe {
        ffi::clingo_ast_attribute_set_location(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            &raw const location.raw,
        )
    })
}

/// Writes a `string` attribute.
pub(crate) fn ast_attribute_set_string(
    ast: RawAst,
    attribute: Attribute,
    value: &CStr,
) -> Result<(), Error> {
    // SAFETY: as `ast_attribute_set_number`, for a `string`-typed attribute;
    // `value` is NUL-terminated and clingo copies it (clingo.h:3862).
    call(|| unsafe {
        ffi::clingo_ast_attribute_set_string(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            value.as_ptr(),
        )
    })
}

/// Writes an `ast` attribute; clingo takes its own reference to `value`.
pub(crate) fn ast_attribute_set_ast(
    ast: RawAst,
    attribute: Attribute,
    value: RawAst,
) -> Result<(), Error> {
    // SAFETY: as `ast_attribute_set_number`, for an `ast`-typed attribute;
    // `value` is a live node, and clingo `incRef`s it (`control.cc:1717-1725`,
    // clingo.h:3886).
    call(|| unsafe {
        ffi::clingo_ast_attribute_set_ast(ast.as_ptr(), attribute_to_raw(attribute), value.as_ptr())
    })
}

/// Writes an `optional_ast` attribute; `None` clears it.
pub(crate) fn ast_attribute_set_optional_ast(
    ast: RawAst,
    attribute: Attribute,
    value: Option<RawAst>,
) -> Result<(), Error> {
    // SAFETY: as `ast_attribute_set_ast`, and a null `value` is the
    // documented "no value" (clingo.h:3915).
    call(|| unsafe {
        ffi::clingo_ast_attribute_set_optional_ast(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            value.map_or(std::ptr::null_mut(), RawAst::as_ptr),
        )
    })
}

/// Replaces one element of a `string_array` attribute. The caller has already
/// checked `index < len`: clingo does not (`control.cc:1733-1737`).
pub(crate) fn ast_attribute_set_string_at(
    ast: RawAst,
    attribute: Attribute,
    index: usize,
    value: &CStr,
) -> Result<(), Error> {
    // SAFETY: as `ast_attribute_set_string`, for a `string_array`-typed
    // attribute, with `index` checked by the caller (clingo.h:3941).
    call(|| unsafe {
        ffi::clingo_ast_attribute_set_string_at(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            index,
            value.as_ptr(),
        )
    })
}

/// Removes one element of a `string_array` attribute. The caller has already
/// checked `index < len` (`control.cc:1739-1745`).
pub(crate) fn ast_attribute_delete_string_at(
    ast: RawAst,
    attribute: Attribute,
    index: usize,
) -> Result<(), Error> {
    // SAFETY: as `ast_attribute_set_string_at` (clingo.h:3950).
    call(|| unsafe {
        ffi::clingo_ast_attribute_delete_string_at(ast.as_ptr(), attribute_to_raw(attribute), index)
    })
}

/// Inserts an element into a `string_array` attribute. The caller has already
/// checked `index <= len` (`control.cc:1754-1761`).
pub(crate) fn ast_attribute_insert_string_at(
    ast: RawAst,
    attribute: Attribute,
    index: usize,
    value: &CStr,
) -> Result<(), Error> {
    // SAFETY: as `ast_attribute_set_string_at` (clingo.h:3970).
    call(|| unsafe {
        ffi::clingo_ast_attribute_insert_string_at(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            index,
            value.as_ptr(),
        )
    })
}

/// Replaces one element of an `ast_array` attribute. The caller has already
/// checked `index < len` (`control.cc:1772-1781`).
pub(crate) fn ast_attribute_set_ast_at(
    ast: RawAst,
    attribute: Attribute,
    index: usize,
    value: RawAst,
) -> Result<(), Error> {
    // SAFETY: as `ast_attribute_set_ast`, for an `ast_array`-typed attribute,
    // with `index` checked by the caller (clingo.h:3998).
    call(|| unsafe {
        ffi::clingo_ast_attribute_set_ast_at(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            index,
            value.as_ptr(),
        )
    })
}

/// Removes one element of an `ast_array` attribute. The caller has already
/// checked `index < len` (`control.cc:1783-1789`).
pub(crate) fn ast_attribute_delete_ast_at(
    ast: RawAst,
    attribute: Attribute,
    index: usize,
) -> Result<(), Error> {
    // SAFETY: as `ast_attribute_set_ast_at` (clingo.h:4007).
    call(|| unsafe {
        ffi::clingo_ast_attribute_delete_ast_at(ast.as_ptr(), attribute_to_raw(attribute), index)
    })
}

/// Inserts an element into an `ast_array` attribute. The caller has already
/// checked `index <= len` (`control.cc:1796-1805`).
pub(crate) fn ast_attribute_insert_ast_at(
    ast: RawAst,
    attribute: Attribute,
    index: usize,
    value: RawAst,
) -> Result<(), Error> {
    // SAFETY: as `ast_attribute_set_ast_at` (clingo.h:4027).
    call(|| unsafe {
        ffi::clingo_ast_attribute_insert_ast_at(
            ast.as_ptr(),
            attribute_to_raw(attribute),
            index,
            value.as_ptr(),
        )
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::{c_int, c_uint};

    use super::*;

    thread_local! {
        /// The code the trampoline set last, in place of clingo's error state.
        static SET: Cell<Option<c_uint>> = const { Cell::new(None) };
    }

    /// An error state that records the code, so `run_parse_callback` runs
    /// under Miri without clingo.
    enum Recorded {}

    impl ErrorState for Recorded {
        fn set(code: c_uint, _: &CStr) {
            SET.with(|s| s.set(Some(code)));
        }
        fn code() -> c_int {
            0
        }
        fn message() -> Option<String> {
            None
        }
    }

    /// A fake node handle: `run_parse_callback` never dereferences the
    /// `Ast` it is given, only moves it into the closure, so any non-null,
    /// well-aligned address is fine here, mirroring `raw::trampoline`'s own
    /// `Recorded` fake-pointer pattern. The caller must `mem::forget` every
    /// `Ast` built from it: `Ast::drop` calls the real `clingo_ast_release`,
    /// which Miri cannot run.
    fn fake_ast() -> Ast {
        Ast::from_raw(NonNull::dangling())
    }

    #[test]
    fn a_successful_callback_returns_true_and_sets_no_error() {
        SET.with(|s| s.set(None));
        let calls = Cell::new(0);
        let mut function: &mut ParseFunction<'_> = &mut |ast: Ast| {
            calls.set(calls.get() + 1);
            std::mem::forget(ast);
            Ok(())
        };
        let context = ParseContext::new(&mut function);
        assert!(run_parse_callback::<Recorded>(&context, fake_ast()));
        assert_eq!(calls.get(), 1);
        assert_eq!(SET.with(Cell::get), None);
        assert!(context.error.take().is_none());
    }

    #[test]
    fn an_error_from_the_callback_is_recorded() {
        // The "an earlier failure stops every later call" half of S8 (the
        // `context.error.is_set()` short-circuit at the top of
        // `run_parse_callback`) is not exercised here: that branch returns
        // without ever passing its own `ast` argument to the closure, so `ast`
        // is dropped in the ordinary way when this function returns, which
        // calls the real `clingo_ast_release` on it. A fake pointer from
        // `fake_ast` cannot survive that drop, unlike the paths above and
        // below, whose closures always take `ast` and `mem::forget` it first.
        // The short-circuit is exercised end to end instead by
        // `api_ast_parse.rs::
        // an_error_from_the_callback_is_returned_with_its_own_kind_and_later_calls_are_skipped`,
        // against a real, parsed node.
        SET.with(|s| s.set(None));
        let calls = Cell::new(0);
        let mut function: &mut ParseFunction<'_> = &mut |ast: Ast| {
            calls.set(calls.get() + 1);
            std::mem::forget(ast);
            Err(Error::new(ErrorKind::Conversion, "stop here"))
        };
        let context = ParseContext::new(&mut function);
        assert!(!run_parse_callback::<Recorded>(&context, fake_ast()));
        assert_eq!(SET.with(Cell::get), Some(ffi::clingo_error_unknown));
        assert_eq!(calls.get(), 1);
        let err = context.error.take().unwrap();
        assert_eq!(err.kind(), ErrorKind::Conversion);
    }

    #[test]
    fn a_panic_in_the_callback_is_caught() {
        SET.with(|s| s.set(None));
        let mut function: &mut ParseFunction<'_> = &mut |ast: Ast| -> Result<(), Error> {
            std::mem::forget(ast);
            panic!("stop here")
        };
        let context = ParseContext::new(&mut function);
        assert!(!run_parse_callback::<Recorded>(&context, fake_ast()));
        assert_eq!(SET.with(Cell::get), Some(ffi::clingo_error_unknown));
        let payload = context.panic.take().unwrap();
        assert_eq!(*payload.downcast::<&str>().unwrap(), "stop here");
    }
}

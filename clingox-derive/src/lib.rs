//! The procedural macros of [clingox](https://docs.rs/clingox):
//! `#[derive(ToSymbol)]`, `#[derive(FromSymbol)]` and `sym!`.
//!
//! Do not depend on this crate directly. clingox re-exports each macro under the
//! name of its trait, with its `derive` feature (on by default), and documents
//! the mapping and the `#[clingo(..)]` attributes there. The generated code
//! names clingox only through `::clingox::__private` and contains no `unsafe`,
//! so it compiles in a crate that forbids `unsafe_code`.

use proc_macro::TokenStream;

mod attrs;
mod derive;
mod name;
mod sym;

/// Derives `clingox::ToSymbol`. The documentation of the trait in clingox
/// describes the mapping and the `#[clingo(..)]` attributes.
#[proc_macro_derive(ToSymbol, attributes(clingo))]
pub fn derive_to_symbol(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    derive::to_symbol(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derives `clingox::FromSymbol`, and `clingox::Predicate` for a struct. The
/// documentation of the traits in clingox describes the mapping.
#[proc_macro_derive(FromSymbol, attributes(clingo))]
pub fn derive_from_symbol(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    derive::from_symbol(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Builds a `clingox::Symbol` from clingo's term syntax. The documentation of
/// `clingox::sym` describes the syntax.
#[proc_macro]
pub fn sym(input: TokenStream) -> TokenStream {
    sym::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

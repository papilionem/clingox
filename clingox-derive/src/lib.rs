//! The procedural macros of clingox: `#[derive(ToSymbol)]`,
//! `#[derive(FromSymbol)]` and `sym!`.
//!
//! Use them through `clingox`, which re-exports each one under the name of its
//! trait and documents them there. The generated code names clingox only
//! through `::clingox::__private` and contains no `unsafe`, so it compiles in a
//! crate that forbids `unsafe_code` (RULES 11.5, DESIGN S18).

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

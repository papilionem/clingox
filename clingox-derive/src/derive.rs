//! `#[derive(ToSymbol)]` and `#[derive(FromSymbol)]`.

use heck::ToSnakeCase;
use proc_macro2::{Ident, Literal, Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, LitStr, Member, Type};

use crate::attrs::{self, Place, Text};
use crate::name::{self, Word};

/// A struct or an enum variant: one clingo term.
struct Term {
    /// The Rust name for error messages: `Wide` or `Shape::Circle`.
    rust_name: String,
    /// The clingo name: `wide` or `circle`.
    name: String,
    /// `Self` or `Self::Circle`.
    path: TokenStream,
    style: Style,
    fields: Vec<Field>,
}

#[derive(Clone, Copy)]
enum Style {
    Named,
    Unnamed,
    Unit,
}

struct Field {
    member: Member,
    /// The field's name, or its index in a tuple struct.
    label: String,
    ty: Type,
    text: Option<Text>,
}

/// Which trait an impl is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    To,
    From,
}

pub(crate) fn to_symbol(input: &DeriveInput) -> syn::Result<TokenStream> {
    let terms = terms(input)?;
    let ident = &input.ident;
    let generics = bounded(input, Direction::To);
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let body = if is_struct(input) {
        let term = &terms[0];
        let values = term.fields.iter().map(|field| {
            let member = &field.member;
            quote!(&self.#member)
        });
        term_to_symbol(term, values)
    } else {
        let arms = terms.iter().map(|term| {
            let bindings = bindings(term);
            let pattern = pattern(term, &bindings);
            let body = term_to_symbol(term, bindings.iter().map(|b| quote!(#b)));
            quote!(#pattern => #body,)
        });
        quote!(match self { #(#arms)* })
    };
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics ::clingox::ToSymbol for #ident #ty_generics #where_clause {
            fn to_symbol(&self) -> ::clingox::Result<::clingox::Symbol> {
                #body
            }
        }
    })
}

pub(crate) fn from_symbol(input: &DeriveInput) -> syn::Result<TokenStream> {
    let terms = terms(input)?;
    let ident = &input.ident;
    let generics = bounded(input, Direction::From);
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let symbol = Ident::new("symbol", Span::mixed_site());
    let arms = terms.iter().map(|term| {
        let arguments: Vec<Ident> = (0..term.fields.len())
            .map(|i| Ident::new(&format!("__argument{i}"), Span::mixed_site()))
            .collect();
        let name = &term.name;
        let rust_name = &term.rust_name;
        let values = term.fields.iter().zip(&arguments).map(|(field, argument)| {
            let label = &field.label;
            let conversion = match field.text {
                Some(Text::String) => quote!(::clingox::__private::string_from(*#argument)),
                Some(Text::Constant) => quote!(::clingox::__private::constant_from(*#argument)),
                None => {
                    let ty = &field.ty;
                    quote_spanned!(ty.span()=> <#ty as ::clingox::FromSymbol>::from_symbol(*#argument))
                }
            };
            quote!(::clingox::__private::field(#conversion, #symbol, #rust_name, #label)?)
        });
        let path = &term.path;
        let value = match term.style {
            Style::Named => {
                let members = term.fields.iter().map(|f| &f.member);
                quote!(#path { #(#members: #values),* })
            }
            Style::Unnamed => quote!(#path(#(#values),*)),
            Style::Unit => quote!(#path),
        };
        quote! {
            ::core::option::Option::Some((#name, [#(#arguments),*])) => {
                ::core::result::Result::Ok(#value)
            }
        }
    });
    let (mismatch, predicate) = if is_struct(input) {
        let term = &terms[0];
        let (rust_name, name) = (&term.rust_name, &term.name);
        let arity = term.fields.len();
        let mismatch = quote!(::clingox::__private::mismatch(#symbol, #rust_name, #name, #arity));
        let arity = u32::try_from(arity)
            .map_err(|_| syn::Error::new_spanned(ident, "a predicate has too many fields"))?;
        let arity = Literal::u32_unsuffixed(arity);
        let predicate = quote! {
            #[automatically_derived]
            impl #impl_generics ::clingox::Predicate for #ident #ty_generics #where_clause {
                const NAME: &'static str = #name;
                const ARITY: u32 = #arity;
            }
        };
        (mismatch, predicate)
    } else {
        let rust_name = ident.to_string();
        let mismatch = quote!(::clingox::__private::no_variant(#symbol, #rust_name));
        (mismatch, TokenStream::new())
    };
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics ::clingox::FromSymbol for #ident #ty_generics #where_clause {
            fn from_symbol(#symbol: ::clingox::Symbol) -> ::clingox::Result<Self> {
                match ::clingox::__private::parts(#symbol) {
                    #(#arms)*
                    _ => ::core::result::Result::Err(#mismatch),
                }
            }
        }
        #predicate
    })
}

/// The call that builds the term's symbol from its field values, each an
/// expression of type `&FieldType`.
fn term_to_symbol(term: &Term, values: impl Iterator<Item = TokenStream>) -> TokenStream {
    let name = &term.name;
    let arguments = term
        .fields
        .iter()
        .zip(values)
        .map(|(field, value)| match field.text {
            Some(Text::String) => quote!(::clingox::__private::string_field(#value)?),
            Some(Text::Constant) => quote!(::clingox::__private::constant_field(#value)?),
            None => {
                let ty = &field.ty;
                quote_spanned!(ty.span()=> <#ty as ::clingox::ToSymbol>::to_symbol(#value)?)
            }
        });
    quote!(::clingox::__private::function(#name, &[#(#arguments),*]))
}

fn is_struct(input: &DeriveInput) -> bool {
    matches!(input.data, Data::Struct(_))
}

/// One binding per field of an enum variant.
fn bindings(term: &Term) -> Vec<Ident> {
    (0..term.fields.len())
        .map(|i| format_ident!("__field{}", i, span = Span::mixed_site()))
        .collect()
}

/// The pattern that matches a variant and binds its fields.
fn pattern(term: &Term, bindings: &[Ident]) -> TokenStream {
    let path = &term.path;
    match term.style {
        Style::Named => {
            let members = term.fields.iter().map(|f| &f.member);
            quote!(#path { #(#members: #bindings),* })
        }
        Style::Unnamed => quote!(#path(#(#bindings),*)),
        Style::Unit => quote!(#path),
    }
}

/// The generics of the input, with `T: ToSymbol` or `T: FromSymbol` for each
/// type parameter, as serde does.
fn bounded(input: &DeriveInput, direction: Direction) -> syn::Generics {
    let mut generics = input.generics.clone();
    let parameters: Vec<Ident> = generics.type_params().map(|p| p.ident.clone()).collect();
    if !parameters.is_empty() {
        let bound = match direction {
            Direction::To => quote!(::clingox::ToSymbol),
            Direction::From => quote!(::clingox::FromSymbol),
        };
        let where_clause = generics.make_where_clause();
        for parameter in parameters {
            where_clause
                .predicates
                .push(syn::parse_quote!(#parameter: #bound));
        }
    }
    generics
}

/// The terms of a struct (one) or an enum (one per variant), checked.
fn terms(input: &DeriveInput) -> syn::Result<Vec<Term>> {
    match &input.data {
        Data::Struct(data) => {
            let name = attrs::name(&input.attrs, Place::Struct)?;
            let term = term(
                &input.ident,
                name.as_ref(),
                &data.fields,
                input.ident.to_string(),
                quote!(Self),
            )?;
            Ok(vec![term])
        }
        Data::Enum(data) => {
            attrs::name(&input.attrs, Place::Enum)?;
            if data.variants.is_empty() {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "an enum without variants has no values to convert",
                ));
            }
            let mut terms: Vec<Term> = Vec::new();
            for variant in &data.variants {
                let name = attrs::name(&variant.attrs, Place::Variant)?;
                let ident = &variant.ident;
                let term = term(
                    ident,
                    name.as_ref(),
                    &variant.fields,
                    format!("{}::{}", input.ident, ident),
                    quote!(Self::#ident),
                )?;
                if let Some(other) = terms
                    .iter()
                    .find(|t| t.name == term.name && t.fields.len() == term.fields.len())
                {
                    return Err(syn::Error::new_spanned(
                        ident,
                        format!(
                            "`{}` and `{}` are both `{}/{}`, so a symbol could not tell them \
                             apart; rename one with `#[clingo(name = \"..\")]`",
                            other.rust_name,
                            term.rust_name,
                            term.name,
                            term.fields.len()
                        ),
                    ));
                }
                terms.push(term);
            }
            Ok(terms)
        }
        Data::Union(data) => Err(syn::Error::new_spanned(
            data.union_token,
            "clingo symbols cannot be derived for a union; use a struct or an enum",
        )),
    }
}

fn term(
    ident: &Ident,
    name: Option<&LitStr>,
    fields: &Fields,
    rust_name: String,
    path: TokenStream,
) -> syn::Result<Term> {
    let name = if let Some(literal) = name {
        let value = literal.value();
        check_name(&value, literal, "the name")?;
        value
    } else {
        let value = snake_name(&name::unraw(ident));
        check_name(&value, ident, "the name derived from the Rust name")?;
        value
    };
    let style = match fields {
        Fields::Named(_) => Style::Named,
        Fields::Unnamed(_) => Style::Unnamed,
        Fields::Unit => Style::Unit,
    };
    let fields = fields
        .iter()
        .enumerate()
        .map(|(index, field)| checked_field(index, field))
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(Term {
        rust_name,
        name,
        path,
        style,
        fields,
    })
}

/// The clingo name for a Rust name: heck's snake case, with the leading
/// underscores kept, since clingo allows them and heck drops them
/// (`_Hidden` becomes `_hidden`, not `hidden`).
fn snake_name(rust_name: &str) -> String {
    let rest = rust_name.trim_start_matches('_');
    let underscores = &rust_name[..rust_name.len() - rest.len()];
    format!("{underscores}{}", rest.to_snake_case())
}

fn check_name(value: &str, at: &impl quote::ToTokens, what: &str) -> syn::Result<()> {
    let problem = match name::classify(value) {
        Word::Identifier => return Ok(()),
        Word::Variable => "clingo reads a name that starts with an uppercase letter as a variable",
        Word::Invalid if value == "not" => "`not` is a keyword in clingo",
        Word::Invalid => "a clingo name matches `_*[a-z][A-Za-z0-9_']*`",
    };
    Err(syn::Error::new_spanned(
        at,
        format!("{what} `{value}` is not a clingo name: {problem}"),
    ))
}

fn checked_field(index: usize, field: &syn::Field) -> syn::Result<Field> {
    let (member, label) = match &field.ident {
        Some(ident) => (Member::Named(ident.clone()), name::unraw(ident)),
        None => (Member::Unnamed(index.into()), index.to_string()),
    };
    let is_string = is_string(&field.ty);
    let text = match attrs::text(&field.attrs)? {
        Some((kind, attr)) if !is_string => {
            let key = match kind {
                Text::String => "string",
                Text::Constant => "constant",
            };
            return Err(syn::Error::new_spanned(
                attr,
                format!(
                    "`#[clingo({key})]` applies only to a `String` field, and `{label}` is not \
                     one; other types convert with their own `ToSymbol` and `FromSymbol`"
                ),
            ));
        }
        Some((kind, _)) => Some(kind),
        None if is_string => {
            let at: &dyn quote::ToTokens = match &field.ident {
                Some(ident) => ident,
                None => &field.ty,
            };
            return Err(syn::Error::new_spanned(
                at,
                format!(
                    "the `String` field `{label}` must say what it is in clingo: \
                     `#[clingo(string)]` for a string such as \"comp13\", or \
                     `#[clingo(constant)]` for a constant such as comp13; the two never \
                     match each other in rules"
                ),
            ));
        }
        None => None,
    };
    Ok(Field {
        member,
        label,
        ty: field.ty.clone(),
        text,
    })
}

/// Whether `ty` names `String` by one of its usual paths.
fn is_string(ty: &Type) -> bool {
    match ty {
        Type::Group(group) => is_string(&group.elem),
        Type::Paren(paren) => is_string(&paren.elem),
        Type::Path(path) if path.qself.is_none() => {
            let segments: Vec<String> = path
                .path
                .segments
                .iter()
                .map(|s| match s.arguments {
                    syn::PathArguments::None => s.ident.to_string(),
                    _ => String::new(),
                })
                .collect();
            let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
            matches!(
                segments.as_slice(),
                ["String"] | ["std" | "alloc", "string", "String"]
            ) && (path.path.leading_colon.is_none() || segments.len() == 3)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::snake_name;

    #[test]
    fn names_are_snake_case_with_leading_underscores_kept() {
        for (rust, clingo) in [
            ("Asserted", "asserted"),
            ("TaskAssignment", "task_assignment"),
            ("HTTPCheck", "http_check"),
            ("Point3D", "point3_d"),
            ("Vec2", "vec2"),
            ("_Hidden", "_hidden"),
            ("__P", "__p"),
            ("DarkRed", "dark_red"),
        ] {
            assert_eq!(snake_name(rust), clingo, "{rust}");
        }
    }
}

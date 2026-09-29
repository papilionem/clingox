//! The `#[clingo(..)]` attributes of the derives.

use syn::{Attribute, LitStr};

/// How a `String` field maps to clingo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Text {
    /// `#[clingo(string)]`: a clingo string, `"comp13"`.
    String,
    /// `#[clingo(constant)]`: a constant, `comp13`.
    Constant,
}

/// Where an attribute sits, which decides the keys it may have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    Struct,
    Enum,
    Variant,
}

/// The `name = ".."` of a struct or variant, if given.
///
/// On an enum, any `#[clingo(..)]` is an error: each variant has its own name.
pub(crate) fn name(attrs: &[Attribute], place: Place) -> syn::Result<Option<LitStr>> {
    let mut name = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("clingo")) {
        if place == Place::Enum {
            return Err(syn::Error::new_spanned(
                attr,
                "`#[clingo(..)]` does not apply to an enum: each variant is its own term, \
                 so put `#[clingo(name = \"..\")]` on the variant",
            ));
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let value: LitStr = meta.value()?.parse()?;
                if name.is_some() {
                    return Err(syn::Error::new_spanned(
                        &value,
                        "`name` is given twice in `#[clingo(..)]`",
                    ));
                }
                name = Some(value);
                Ok(())
            } else if meta.path.is_ident("string") || meta.path.is_ident("constant") {
                Err(meta.error(
                    "`string` and `constant` apply to a `String` field, not to a struct or variant",
                ))
            } else {
                Err(meta.error(format!(
                    "unknown key `{}` in `#[clingo(..)]`; a struct or variant takes \
                     `name = \"..\"`",
                    key(&meta.path)
                )))
            }
        })?;
    }
    Ok(name)
}

/// The text kind a field declares, with the attribute that declares it.
pub(crate) fn text(attrs: &[Attribute]) -> syn::Result<Option<(Text, &Attribute)>> {
    let mut found: Option<(Text, &Attribute)> = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("clingo")) {
        attr.parse_nested_meta(|meta| {
            let kind = if meta.path.is_ident("string") {
                Text::String
            } else if meta.path.is_ident("constant") {
                Text::Constant
            } else if meta.path.is_ident("name") {
                return Err(meta.error(
                    "`name` applies to a struct or variant; a field takes `string` or `constant`",
                ));
            } else {
                return Err(meta.error(format!(
                    "unknown key `{}` in `#[clingo(..)]`; a field takes `string` or `constant`",
                    key(&meta.path)
                )));
            };
            if found.is_some() {
                return Err(meta.error(
                    "a field is either `#[clingo(string)]` or `#[clingo(constant)]`, not both",
                ));
            }
            found = Some((kind, attr));
            Ok(())
        })?;
    }
    Ok(found)
}

/// A key as written, for error messages.
fn key(path: &syn::Path) -> String {
    quote::ToTokens::to_token_stream(path)
        .to_string()
        .replace(' ', "")
}

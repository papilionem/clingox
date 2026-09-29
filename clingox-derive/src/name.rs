//! clingo's identifiers and variables, as its lexer reads them.

/// What a word is in clingo's term syntax.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Word {
    /// `_*[a-z][A-Za-z0-9_']*`, other than `not`: a name.
    Identifier,
    /// `_*[A-Z][A-Za-z0-9_']*`, or `_` alone: a variable.
    Variable,
    /// Anything else, including `not`, which is a keyword.
    Invalid,
}

/// Classifies `text` as clingo's lexer would.
pub(crate) fn classify(text: &str) -> Word {
    if text == "_" {
        return Word::Variable;
    }
    let rest = text.trim_start_matches('_');
    let mut chars = rest.chars();
    let kind = match chars.next() {
        Some(c) if c.is_ascii_lowercase() => Word::Identifier,
        Some(c) if c.is_ascii_uppercase() => Word::Variable,
        _ => return Word::Invalid,
    };
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '\'') {
        return Word::Invalid;
    }
    if text == "not" {
        return Word::Invalid;
    }
    kind
}

/// The text of an identifier token, without the `r#` of a raw identifier.
pub(crate) fn unraw(ident: &proc_macro2::Ident) -> String {
    let text = ident.to_string();
    match text.strip_prefix("r#") {
        Some(plain) => plain.to_owned(),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_classified_as_clingo_lexes_them() {
        for name in ["p", "_p", "__p", "x'", "pQ_9'", "type", "true", "a1"] {
            assert_eq!(classify(name), Word::Identifier, "{name}");
        }
        for variable in ["X", "_", "_X", "Foo", "X1"] {
            assert_eq!(classify(variable), Word::Variable, "{variable}");
        }
        for invalid in ["", "__", "not", "1a", "é", "a b", "a-b", "_1"] {
            assert_eq!(classify(invalid), Word::Invalid, "{invalid}");
        }
    }
}

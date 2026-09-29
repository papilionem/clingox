//! `sym!`: one ground term in clingo's syntax, with `{expr}` splices.

use proc_macro2::{Delimiter, Group, Ident, Literal, Span, TokenStream, TokenTree};
use quote::{ToTokens, quote, quote_spanned};

use crate::name::{self, Word};

/// A parsed term.
enum Term {
    Number(i32),
    String(syn::LitStr),
    Supremum,
    Infimum,
    /// A function; without arguments, a constant.
    Function {
        name: String,
        negative: bool,
        arguments: Vec<Term>,
    },
    Tuple(Vec<Term>),
    /// The splice with this index, in the order the splices appear.
    Splice(usize),
}

/// A splice's expression and the span of its braces.
struct Splice {
    tokens: TokenStream,
    span: Span,
}

pub(crate) fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    if tokens.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "`sym!` needs one term, such as `sym!(p(1, \"x\"))`",
        ));
    }
    let mut parser = Parser {
        tokens: &tokens,
        position: 0,
        splices: Vec::new(),
        end: "the end of the term",
    };
    let term = parser.term()?;
    if let Some(extra) = parser.peek() {
        return Err(error_at(
            extra,
            format!(
                "expected the end of the term, found {}; `sym!` takes one term",
                describe(extra)
            ),
        ));
    }

    // Each splice is evaluated and converted in its own `let`, in order and
    // in the caller's context, so `?` and `return` inside it act on the
    // caller's function. The bindings have mixed-site hygiene, so they never
    // capture a name the caller uses inside a splice. The conversion takes
    // the span of the splice's braces, so a missing `ToSymbol` is reported
    // there. Only then is the symbol built, inside a closure whose `?` stays
    // in the macro.
    let values: Vec<Ident> = (0..parser.splices.len())
        .map(|i| Ident::new(&format!("__clingox_splice{i}"), Span::mixed_site()))
        .collect();
    let lets = parser.splices.iter().zip(&values).map(|(splice, value)| {
        let tokens = &splice.tokens;
        let span = splice.span;
        // A statement list keeps its braces; an expression is borrowed, so
        // the splice does not move the value it names.
        let has_statements = splice
            .tokens
            .clone()
            .into_iter()
            .any(|t| matches!(&t, TokenTree::Punct(p) if p.as_char() == ';'));
        let borrowed = if has_statements {
            quote_spanned!(span=> &{ #tokens })
        } else {
            quote_spanned!(span=> &(#tokens))
        };
        let conversion = quote_spanned!(span=> ::clingox::__private::to_symbol(#borrowed));
        quote!(let #value = #conversion;)
    });
    let body = build(&term, &values, true);
    Ok(quote! {
        {
            #(#lets)*
            ::clingox::__private::build(|| #body)
        }
    })
}

/// The expression that builds `term`. At the top it is a
/// `Result<Symbol>`; below it, a `Symbol` after `?`.
fn build(term: &Term, values: &[Ident], top: bool) -> TokenStream {
    let fallible = |call: TokenStream| {
        if top { call } else { quote!(#call?) }
    };
    let infallible = |value: TokenStream| {
        if top {
            quote!(::core::result::Result::Ok(#value))
        } else {
            value
        }
    };
    match term {
        Term::Number(n) => {
            let n = Literal::i32_unsuffixed(*n);
            infallible(quote!(::clingox::__private::number(#n)))
        }
        Term::Supremum => infallible(quote!(::clingox::__private::supremum())),
        Term::Infimum => infallible(quote!(::clingox::__private::infimum())),
        Term::String(literal) => {
            let value = syn::LitStr::new(&literal.value(), literal.span());
            fallible(quote!(::clingox::__private::string(#value)))
        }
        Term::Function {
            name,
            negative,
            arguments,
        } => {
            let arguments = arguments.iter().map(|a| build(a, values, false));
            let constructor = if *negative {
                quote!(negative_function)
            } else {
                quote!(function)
            };
            fallible(quote!(::clingox::__private::#constructor(#name, &[#(#arguments),*])))
        }
        Term::Tuple(elements) => {
            let elements = elements.iter().map(|e| build(e, values, false));
            fallible(quote!(::clingox::__private::tuple(&[#(#elements),*])))
        }
        Term::Splice(index) => {
            let value = &values[*index];
            fallible(quote!(#value))
        }
    }
}

struct Parser<'t> {
    tokens: &'t [TokenTree],
    position: usize,
    splices: Vec<Splice>,
    /// What may follow the term being parsed, for error messages.
    end: &'static str,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&TokenTree> {
        self.tokens.get(self.position)
    }

    fn next(&mut self) -> Option<&TokenTree> {
        let token = self.tokens.get(self.position);
        self.position += 1;
        token
    }

    fn term(&mut self) -> syn::Result<Term> {
        let Some(token) = self.next().cloned() else {
            let span = self
                .tokens
                .last()
                .map_or_else(Span::call_site, TokenTree::span);
            return Err(syn::Error::new(
                span,
                format!("expected a term before {}", self.end),
            ));
        };
        match &token {
            TokenTree::Literal(literal) => literal_term(literal, false, literal.span()),
            TokenTree::Ident(ident) => self.function(ident, false),
            TokenTree::Punct(punct) if punct.as_char() == '#' => match self.next() {
                Some(TokenTree::Ident(ident)) if ident == "sup" => Ok(Term::Supremum),
                Some(TokenTree::Ident(ident)) if ident == "inf" => Ok(Term::Infimum),
                Some(other) => Err(error_at(
                    other,
                    format!(
                        "expected `sup` or `inf` after `#`, found {}",
                        describe(other)
                    ),
                )),
                None => Err(syn::Error::new(
                    punct.span(),
                    "expected `sup` or `inf` after `#`",
                )),
            },
            TokenTree::Punct(punct) if punct.as_char() == '-' => match self.next().cloned() {
                Some(TokenTree::Ident(ident)) => self.function(&ident, true),
                Some(TokenTree::Literal(literal)) => {
                    literal_term(&literal, true, join(punct.span(), literal.span()))
                }
                _ => Err(syn::Error::new(
                    punct.span(),
                    "`-` must be followed by a name (classical negation, `-p`) or an integer \
                     (`-5`); `sym!` does not negate splices or tuples",
                )),
            },
            TokenTree::Group(group) => match group.delimiter() {
                Delimiter::Parenthesis => self.tuple(group),
                Delimiter::Brace => {
                    let index = self.splices.len();
                    self.splices.push(Splice {
                        tokens: group.stream(),
                        span: group.span(),
                    });
                    Ok(Term::Splice(index))
                }
                _ => Err(unexpected(&token)),
            },
            TokenTree::Punct(_) => Err(unexpected(&token)),
        }
    }

    /// A name, with its argument list if one follows.
    fn function(&mut self, ident: &Ident, negative: bool) -> syn::Result<Term> {
        let name = name::unraw(ident);
        match name::classify(&name) {
            Word::Identifier => {}
            Word::Variable => {
                return Err(syn::Error::new(
                    ident.span(),
                    format!(
                        "`{name}` is a variable in clingo, and a symbol is a ground term \
                         without variables; write a lowercase name, or splice a value with \
                         `{{expr}}`"
                    ),
                ));
            }
            Word::Invalid => {
                return Err(syn::Error::new(
                    ident.span(),
                    format!(
                        "`{name}` is not a clingo name, which matches `_*[a-z][A-Za-z0-9_']*` \
                         and is not `not`"
                    ),
                ));
            }
        }
        let arguments = match self.peek() {
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => {
                let group = group.clone();
                self.position += 1;
                let (arguments, trailing) = self.list(&group, "`)` after the arguments")?;
                if let Some(comma) = trailing {
                    return Err(syn::Error::new(
                        comma,
                        "a trailing comma after the last argument is not allowed, as in clingo",
                    ));
                }
                arguments
            }
            _ => Vec::new(),
        };
        Ok(Term::Function {
            name,
            negative,
            arguments,
        })
    }

    /// `()`, `(t,)`, `(t1, t2, ..)`, or `(t)`, which is `t`.
    fn tuple(&mut self, group: &Group) -> syn::Result<Term> {
        let (mut elements, trailing) = self.list(group, "`)` after the tuple")?;
        if elements.len() == 1 && trailing.is_none() {
            return Ok(elements.remove(0));
        }
        Ok(Term::Tuple(elements))
    }

    /// The comma-separated terms inside `group`, and the span of a trailing
    /// comma if there is one.
    fn list(&mut self, group: &Group, end: &'static str) -> syn::Result<(Vec<Term>, Option<Span>)> {
        let tokens: Vec<TokenTree> = group.stream().into_iter().collect();
        let mut inner = Parser {
            tokens: &tokens,
            position: 0,
            splices: std::mem::take(&mut self.splices),
            end,
        };
        let mut terms = Vec::new();
        let mut trailing = None;
        while inner.peek().is_some() {
            if let Some(TokenTree::Punct(comma)) = inner.peek()
                && comma.as_char() == ','
            {
                return Err(syn::Error::new(comma.span(), "expected a term, found `,`"));
            }
            terms.push(inner.term()?);
            trailing = None;
            match inner.next() {
                None => break,
                Some(TokenTree::Punct(comma)) if comma.as_char() == ',' => {
                    trailing = Some(comma.span());
                }
                Some(other) => {
                    return Err(error_at(
                        other,
                        format!("expected `,` or `)`, found {}", describe(other)),
                    ));
                }
            }
        }
        self.splices = inner.splices;
        Ok((terms, trailing))
    }
}

/// A number or a string from a literal token, with a leading `-` if
/// `negative`.
fn literal_term(literal: &Literal, negative: bool, span: Span) -> syn::Result<Term> {
    match syn::Lit::new(literal.clone()) {
        syn::Lit::Int(int) => {
            if !int.suffix().is_empty() {
                return Err(syn::Error::new(
                    literal.span(),
                    format!(
                        "`{literal}` has a type suffix; clingo numbers are plain integers \
                         within `i32`"
                    ),
                ));
            }
            let magnitude: Option<i64> = int.base10_digits().parse::<u32>().ok().map(i64::from);
            let value = magnitude
                .map(|m| if negative { -m } else { m })
                .and_then(|v| i32::try_from(v).ok());
            match value {
                Some(value) => Ok(Term::Number(value)),
                None => Err(syn::Error::new(
                    span,
                    format!(
                        "`{}{literal}` is outside the range of `i32`, clingo's number type \
                         ({} to {})",
                        if negative { "-" } else { "" },
                        i32::MIN,
                        i32::MAX
                    ),
                )),
            }
        }
        syn::Lit::Str(string) if !negative => {
            if !string.suffix().is_empty() {
                return Err(syn::Error::new(
                    literal.span(),
                    "a string literal in `sym!` takes no suffix",
                ));
            }
            Ok(Term::String(string))
        }
        syn::Lit::Str(_) => Err(syn::Error::new(
            span,
            format!(
                "`-{literal}`: classical negation applies to functions and constants, such as \
                 `-p` or `-p(1)`, not to strings; a `-` before an integer makes it negative"
            ),
        )),
        _ => Err(syn::Error::new(
            literal.span(),
            format!(
                "expected a term, found `{literal}`; `sym!` takes integers within `i32`, string \
                 literals, names, tuples, `#sup`, `#inf` and `{{expr}}` splices"
            ),
        )),
    }
}

/// The span from `a` to `b`, or `b` where spans cannot be joined.
fn join(a: Span, b: Span) -> Span {
    a.join(b).unwrap_or(b)
}

fn describe(token: &TokenTree) -> String {
    match token {
        TokenTree::Group(group) => match group.delimiter() {
            Delimiter::Parenthesis => "`(..)`".to_owned(),
            Delimiter::Brace => "`{..}`".to_owned(),
            Delimiter::Bracket => "`[..]`".to_owned(),
            Delimiter::None => "a group".to_owned(),
        },
        other => format!("`{other}`"),
    }
}

fn error_at(token: &TokenTree, message: String) -> syn::Error {
    syn::Error::new_spanned(token.to_token_stream(), message)
}

fn unexpected(token: &TokenTree) -> syn::Error {
    error_at(
        token,
        format!(
            "expected a term, found {}; `sym!` takes integers within `i32`, string literals, \
             names, tuples, `#sup`, `#inf` and `{{expr}}` splices",
            describe(token)
        ),
    )
}

//! Splits a template into literal text and `{{ ... }}` placeholders.

/// A token extracted from a template source string.
///
/// Contents borrow from the input; only the returned vector allocates.
#[derive(Debug, Clone, PartialEq)]
pub enum Token<'a> {
    /// Text outside any placeholder.
    Lit(&'a str),
    /// A plain identifier placeholder like `{{ accent }}`.
    Var { key: &'a str, raw: &'a str },
    /// An expression placeholder like `{{ accent | lighten 0.1 }}`.
    Expr { expr: &'a str, raw: &'a str },
    /// A block opener like `{{#if dark}}` or `{{#each colors as c}}`.
    Block { spec: &'a str, raw: &'a str },
    /// A block closer like `{{/if}}`.
    Close { name: &'a str, raw: &'a str },
    /// An `{{else}}` marker (the whole token, including any trailing branch).
    Else { raw: &'a str },
    /// A `{{! comment }}` placeholder that renders nothing.
    Comment,
}

/// Tokenize a template.
///
/// Empty or unclosed placeholders remain literal text, and unknown expression
/// tokens keep their original spelling when rendered so partial output stays
/// inspectable.
pub fn parse(input: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut rest = input;

    while !rest.is_empty() {
        let Some(open) = rest.find("{{") else {
            tokens.push(Token::Lit(rest));
            break;
        };

        if open > 0 {
            tokens.push(Token::Lit(&rest[..open]));
        }
        let after_open = &rest[open + 2..];

        let Some(close) = after_open.find("}}") else {
            tokens.push(Token::Lit(&rest[open..]));
            break;
        };

        let inner = &after_open[..close];
        let raw = &rest[open..open + 2 + close + 2];
        let trimmed = inner.trim();

        let token = if trimmed.is_empty() {
            // `{{ }}` and `{{}}` pass through as literal text.
            Token::Lit(raw)
        } else if let Some(name) = trimmed.strip_prefix('/') {
            let name = name.trim();
            if name.is_empty() {
                Token::Lit(raw)
            } else {
                Token::Close { name, raw }
            }
        } else if let Some(spec) = trimmed.strip_prefix('#') {
            let spec = spec.trim();
            if spec.is_empty() {
                Token::Lit(raw)
            } else {
                Token::Block { spec, raw }
            }
        } else if trimmed.starts_with('!') {
            Token::Comment
        } else if is_block_keyword(trimmed, "else") {
            Token::Else { raw }
        } else if is_plain_identifier(trimmed) {
            Token::Var { key: trimmed, raw }
        } else {
            Token::Expr { expr: trimmed, raw }
        };

        tokens.push(token);
        rest = &after_open[close + 2..];
    }

    tokens
}

fn is_block_keyword(s: &str, word: &str) -> bool {
    s == word || s.strip_prefix(word).is_some_and(|r| r.starts_with(' '))
}

fn is_plain_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The original source text for the token span `[from, to)`.
pub fn raw_span(tokens: &[Token], from: usize, to: usize) -> String {
    let mut out = String::new();
    for token in &tokens[from..to] {
        match token {
            Token::Lit(t) => out.push_str(t),
            Token::Var { raw, .. }
            | Token::Expr { raw, .. }
            | Token::Block { raw, .. }
            | Token::Close { raw, .. } => out.push_str(raw),
            Token::Else { raw } => out.push_str(raw),
            // Comments never appear in passthrough output.
            Token::Comment => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Token, parse, raw_span};

    fn keys(src: &str) -> Vec<String> {
        parse(src)
            .iter()
            .filter_map(|t| match t {
                Token::Var { key, .. } | Token::Expr { expr: key, .. } => {
                    Some((*key).to_owned())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn plain_identifier_is_a_var() {
        assert_eq!(keys("color={{ bg }}!"), ["bg"]);
    }

    #[test]
    fn expression_is_detected_and_spacing_trimmed() {
        let src = "{{ accent | lighten 0.1 }}";
        assert_eq!(keys(src), ["accent | lighten 0.1"]);
        assert_eq!(raw_span(&parse(src), 0, parse(src).len()), src);
    }

    #[test]
    fn block_openers_and_closers() {
        let tokens = parse("{{#if dark}}a{{else}}b{{/if}}");
        assert!(matches!(&tokens[0], Token::Block { spec, .. } if *spec == "if dark"));
        assert!(matches!(&tokens[2], Token::Else { .. }));
        assert!(matches!(&tokens[4], Token::Close { name, .. } if *name == "if"));
    }

    #[test]
    fn comments_are_tokens() {
        assert!(matches!(&parse("a{{! note }}b")[1], Token::Comment));
    }

    #[test]
    fn literal_passthrough() {
        assert_eq!(keys("no tokens here"), Vec::<String>::new());
    }

    #[test]
    fn unclosed_brace_is_literal() {
        let tokens = parse("oops {{ unclosed");
        assert!(matches!(&tokens[1], Token::Lit(t) if *t == "{{ unclosed"));
    }

    #[test]
    fn empty_braces_are_literal() {
        let tokens = parse("{{}}");
        assert!(matches!(&tokens[0], Token::Lit(t) if *t == "{{}}"));
    }

    #[test]
    fn whitespace_inside_braces_is_trimmed() {
        assert_eq!(keys("{{  key  }}"), ["key"]);
    }
}

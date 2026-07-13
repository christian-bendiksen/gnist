//! Parses `{{ key }}` placeholders while borrowing text from the input.

/// A borrowed piece of a parsed template.
#[derive(Debug, Clone)]
pub enum Segment<'a> {
    /// Text outside a placeholder.
    Lit(&'a str),
    /// The trimmed name inside a placeholder.
    Var(&'a str),
}

/// Split a template into literal text and variable names.
///
/// Segment contents borrow from `input`; only the returned vector allocates.
/// Empty or unclosed placeholders remain literal text.
pub fn parse(input: &str) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    let mut rest = input;

    while !rest.is_empty() {
        match rest.find("{{") {
            None => {
                segments.push(Segment::Lit(rest));
                break;
            }
            Some(open) => {
                if open > 0 {
                    segments.push(Segment::Lit(&rest[..open]));
                }
                let after_open = &rest[open + 2..];

                match after_open.find("}}") {
                    None => {
                        segments.push(Segment::Lit(&rest[open..]));
                        break;
                    }
                    Some(close) => {
                        let key = after_open[..close].trim();
                        if key.is_empty() {
                            segments.push(Segment::Lit(&rest[open..open + 2 + close + 2]));
                        } else {
                            segments.push(Segment::Var(key));
                        }
                        rest = &after_open[close + 2..];
                    }
                }
            }
        }
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render<'a>(segs: &[Segment<'a>], vars: &[(&str, &str)]) -> String {
        let map: std::collections::HashMap<_, _> = vars.iter().copied().collect();
        segs.iter()
            .map(|s| match s {
                Segment::Lit(t) => *t,
                Segment::Var(k) => map.get(k).copied().unwrap_or("MISSING"),
            })
            .collect()
    }

    #[test]
    fn simple_substitution() {
        let segs = parse("color={{ bg }}!");
        assert_eq!(render(&segs, &[("bg", "#1e1e2e")]), "color=#1e1e2e!");
    }

    #[test]
    fn literal_passthrough() {
        let segs = parse("no tokens here");
        assert_eq!(render(&segs, &[]), "no tokens here");
    }

    #[test]
    fn unclosed_brace_is_literal() {
        let segs = parse("oops {{ unclosed");
        assert_eq!(render(&segs, &[]), "oops {{ unclosed");
    }

    #[test]
    fn empty_braces_are_literal() {
        let segs = parse("{{}}");
        assert_eq!(render(&segs, &[]), "{{}}");
    }

    #[test]
    fn whitespace_inside_braces_is_trimmed() {
        let segs = parse("{{  key  }}");
        assert_eq!(render(&segs, &[("key", "val")]), "val");
    }
}

//! A tiny expression language for templates: filters, math, and comparisons.
//!
//! Resolving an expression returns a [`Value`]; unresolvable names, bad syntax,
//! and unknown filters resolve to [`Value::Missing`] so callers can leave the
//! original token visible.

use std::collections::HashMap;

use super::value::Value;
use crate::theme::colors::{Rgb, parse_color};

/// Evaluate an expression against the theme context and loop bindings.
pub fn eval_expr(
    expr: &str,
    values: &HashMap<String, Value>,
    overlay: &HashMap<String, Value>,
) -> Value {
    let Ok(tokens) = lex(expr) else {
        return Value::Missing;
    };
    let mut parser = Parser { tokens, pos: 0 };
    let Ok(ast) = parser.parse_expr() else {
        return Value::Missing;
    };
    if parser.pos != parser.tokens.len() {
        return Value::Missing;
    }
    eval(&ast, values, overlay)
}

/// Evaluate an expression as a condition.
pub fn eval_condition(
    expr: &str,
    values: &HashMap<String, Value>,
    overlay: &HashMap<String, Value>,
) -> bool {
    eval_expr(expr, values, overlay).is_truthy()
}

fn lookup<'a>(
    values: &'a HashMap<String, Value>,
    overlay: &'a HashMap<String, Value>,
    name: &str,
) -> Option<&'a Value> {
    overlay.get(name).or_else(|| values.get(name))
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Op(&'static str),
    And,
    Or,
    Not,
    True,
    False,
    Nil,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < n {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        match c {
            '+' | '-' | '*' | '/' | '%' | '(' | ')' | ',' | '.' | '[' | ']' | '>' | '<' | '='
            | '!' | '|' => {
                if i + 1 < n {
                    let pair: String = [c, chars[i + 1]].iter().collect();
                    if let Some(op) = match pair.as_str() {
                        "==" => Some("=="),
                        "!=" => Some("!="),
                        ">=" => Some(">="),
                        "<=" => Some("<="),
                        _ => None,
                    } {
                        tokens.push(Tok::Op(op));
                        i += 2;
                        continue;
                    }
                }
                let op = match c {
                    '+' => "+",
                    '-' => "-",
                    '*' => "*",
                    '/' => "/",
                    '%' => "%",
                    '(' => "(",
                    ')' => ")",
                    ',' => ",",
                    '.' => ".",
                    '[' => "[",
                    ']' => "]",
                    '>' => ">",
                    '<' => "<",
                    '=' => "=",
                    '!' => "!",
                    _ => "|",
                };
                tokens.push(Tok::Op(op));
                i += 1;
            }
            '"' | '\'' => {
                let quote = c;
                let mut out = String::new();
                let mut closed = false;
                i += 1;
                while i < n {
                    if chars[i] == '\\' && i + 1 < n {
                        let next = chars[i + 1];
                        out.push(match next {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            other => other,
                        });
                        i += 2;
                        continue;
                    }
                    if chars[i] == quote {
                        closed = true;
                        i += 1;
                        break;
                    }
                    out.push(chars[i]);
                    i += 1;
                }
                if !closed {
                    return Err("unclosed string literal".into());
                }
                tokens.push(Tok::Str(out));
            }
            _ => {
                if c.is_ascii_digit() {
                    let start = i;
                    let mut saw_dot = false;
                    while i < n {
                        if chars[i].is_ascii_digit() {
                            i += 1;
                        } else if chars[i] == '.' && !saw_dot {
                            saw_dot = true;
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    let num: String = chars[start..i].iter().collect();
                    let value: f64 = num
                        .parse()
                        .map_err(|_| format!("invalid number `{num}`"))?;
                    tokens.push(Tok::Num(value));
                } else if c.is_ascii_alphabetic() || c == '_' {
                    let start = i;
                    while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect();
                    tokens.push(match word.as_str() {
                        "and" => Tok::And,
                        "or" => Tok::Or,
                        "not" => Tok::Not,
                        "true" => Tok::True,
                        "false" => Tok::False,
                        "nil" => Tok::Nil,
                        _ => Tok::Ident(word),
                    });
                } else {
                    return Err(format!("unexpected character `{c}`"));
                }
            }
        }
    }

    Ok(tokens)
}

// ---------------------------------------------------------------------------
// AST and parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    And,
    Or,
}

#[derive(Debug, Clone)]
enum Ast {
    Num(f64),
    Str(String),
    Bool(bool),
    Nil,
    Var(String),
    Index(Box<Ast>, Box<Ast>),
    Neg(Box<Ast>),
    Not(Box<Ast>),
    Bin(BinOp, Box<Ast>, Box<Ast>),
    Pipe(Box<Ast>, String, Vec<Ast>),
    Call(String, Vec<Ast>),
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Tok> {
        let tok = self.tokens.get(self.pos).cloned();
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn expect_op(&mut self, op: &'static str) -> Result<(), String> {
        match self.peek() {
            Some(Tok::Op(o)) if *o == op => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(format!("expected `{op}`")),
        }
    }

    fn parse_expr(&mut self) -> Result<Ast, String> {
        self.parse_pipeline()
    }

    fn parse_pipeline(&mut self) -> Result<Ast, String> {
        let mut base = self.parse_or()?;
        while matches!(self.peek(), Some(Tok::Op("|"))) {
            self.pos += 1;
            let name = self.parse_ident()?;
            let mut args = Vec::new();
            while self.at_term_start() {
                args.push(self.parse_or()?);
            }
            base = Ast::Pipe(Box::new(base), name, args);
        }
        Ok(base)
    }

    fn parse_or(&mut self) -> Result<Ast, String> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Tok::Or)) {
            self.pos += 1;
            let right = self.parse_and()?;
            left = Ast::Bin(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Ast, String> {
        let mut left = self.parse_cmp()?;
        while matches!(self.peek(), Some(Tok::And)) {
            self.pos += 1;
            let right = self.parse_cmp()?;
            left = Ast::Bin(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_cmp(&mut self) -> Result<Ast, String> {
        // `not` binds looser than comparisons, so `not a == b` means `not (a == b)`.
        if matches!(self.peek(), Some(Tok::Not)) {
            self.pos += 1;
            let inner = self.parse_cmp()?;
            return Ok(Ast::Not(Box::new(inner)));
        }
        let mut left = self.parse_add()?;
        while let Some(Tok::Op(op)) = self.peek() {
            let bin = match *op {
                "==" => Some(BinOp::Eq),
                "!=" => Some(BinOp::Ne),
                ">" => Some(BinOp::Gt),
                "<" => Some(BinOp::Lt),
                ">=" => Some(BinOp::Ge),
                "<=" => Some(BinOp::Le),
                _ => None,
            };
            let Some(bin) = bin else { break };
            self.pos += 1;
            let right = self.parse_add()?;
            left = Ast::Bin(bin, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Ast, String> {
        let mut left = self.parse_mul()?;
        while let Some(Tok::Op(op @ ("+" | "-"))) = self.peek() {
            let bin = if *op == "+" { BinOp::Add } else { BinOp::Sub };
            self.pos += 1;
            let right = self.parse_mul()?;
            left = Ast::Bin(bin, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Ast, String> {
        let mut left = self.parse_unary()?;
        while let Some(Tok::Op(op @ ("*" | "/" | "%"))) = self.peek() {
            let bin = match *op {
                "*" => BinOp::Mul,
                "/" => BinOp::Div,
                _ => BinOp::Rem,
            };
            self.pos += 1;
            let right = self.parse_unary()?;
            left = Ast::Bin(bin, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Ast, String> {
        match self.peek() {
            Some(Tok::Op("-")) => {
                self.pos += 1;
                Ok(Ast::Neg(Box::new(self.parse_unary()?)))
            }
            Some(Tok::Op("!")) => {
                self.pos += 1;
                Ok(Ast::Not(Box::new(self.parse_unary()?)))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Ast, String> {
        let base = self.parse_primary()?;
        if matches!(self.peek(), Some(Tok::Op("["))) {
            self.pos += 1;
            let index = self.parse_expr()?;
            self.expect_op("]")?;
            Ok(Ast::Index(Box::new(base), Box::new(index)))
        } else {
            Ok(base)
        }
    }

    fn parse_primary(&mut self) -> Result<Ast, String> {
        match self.bump() {
            Some(Tok::Num(v)) => Ok(Ast::Num(v)),
            Some(Tok::Str(s)) => Ok(Ast::Str(s)),
            Some(Tok::True) => Ok(Ast::Bool(true)),
            Some(Tok::False) => Ok(Ast::Bool(false)),
            Some(Tok::Nil) => Ok(Ast::Nil),
            Some(Tok::Op("(")) => {
                let inner = self.parse_expr()?;
                self.expect_op(")")?;
                Ok(inner)
            }
            Some(Tok::Ident(name)) => {
                if matches!(self.peek(), Some(Tok::Op("("))) {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Tok::Op(")"))) {
                        loop {
                            args.push(self.parse_or()?);
                            if matches!(self.peek(), Some(Tok::Op(","))) {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect_op(")")?;
                    Ok(Ast::Call(name, args))
                } else {
                    Ok(Ast::Var(name))
                }
            }
            _ => Err("expected a value".into()),
        }
    }

    fn parse_ident(&mut self) -> Result<String, String> {
        match self.bump() {
            Some(Tok::Ident(name)) => Ok(name),
            _ => Err("expected an identifier".into()),
        }
    }

    fn at_term_start(&self) -> bool {
        matches!(
            self.peek(),
            Some(
                Tok::Num(_)
                    | Tok::Str(_)
                    | Tok::Ident(_)
                    | Tok::True
                    | Tok::False
                    | Tok::Nil
                    | Tok::Op("(")
            )
        )
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

fn eval(
    ast: &Ast,
    values: &HashMap<String, Value>,
    overlay: &HashMap<String, Value>,
) -> Value {
    match ast {
        Ast::Num(v) => Value::Float(*v),
        Ast::Str(s) => Value::Str(s.clone()),
        Ast::Bool(b) => Value::Bool(*b),
        Ast::Nil => Value::Missing,
        Ast::Var(name) => lookup(values, overlay, name).cloned().unwrap_or(Value::Missing),
        Ast::Index(base, index) => match eval(base, values, overlay) {
            Value::List(items) => match eval(index, values, overlay).as_f64() {
                Some(i) if i >= 0.0 && i.fract() == 0.0 => {
                    items.get(i as usize).cloned().unwrap_or(Value::Missing)
                }
                _ => Value::Missing,
            },
            _ => Value::Missing,
        },
        Ast::Neg(inner) => match eval(inner, values, overlay).as_f64() {
            Some(f) => Value::Float(-f),
            None => Value::Missing,
        },
        Ast::Not(inner) => Value::Bool(!eval(inner, values, overlay).is_truthy()),
        Ast::Bin(op, left, right) => {
            eval_bin(*op, eval(left, values, overlay), eval(right, values, overlay))
        }
        Ast::Pipe(base, name, args) => {
            let input = eval(base, values, overlay);
            let args: Vec<Value> = args.iter().map(|a| eval(a, values, overlay)).collect();
            apply_filter(name, input, args)
        }
        Ast::Call(name, args) => {
            let args: Vec<Value> = args.iter().map(|a| eval(a, values, overlay)).collect();
            apply_filter(name, Value::Missing, args)
        }
    }
}

fn eval_bin(op: BinOp, left: Value, right: Value) -> Value {
    match op {
        BinOp::And => Value::Bool(left.is_truthy() && right.is_truthy()),
        BinOp::Or => Value::Bool(left.is_truthy() || right.is_truthy()),
        BinOp::Eq => Value::Bool(left == right),
        BinOp::Ne => Value::Bool(left != right),
        BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le => compare(&left, &right, op),
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => arith(op, left, right),
    }
}

fn compare(left: &Value, right: &Value, op: BinOp) -> Value {
    let ordering = match (left.as_f64(), right.as_f64()) {
        (Some(a), Some(b)) => a.partial_cmp(&b),
        _ => left.display().partial_cmp(&right.display()),
    };
    let result = match ordering {
        Some(std::cmp::Ordering::Greater) => matches!(op, BinOp::Gt | BinOp::Ge),
        Some(std::cmp::Ordering::Less) => matches!(op, BinOp::Lt | BinOp::Le),
        Some(std::cmp::Ordering::Equal) => matches!(op, BinOp::Eq | BinOp::Ge | BinOp::Le),
        None => false,
    };
    Value::Bool(result)
}

fn arith(op: BinOp, left: Value, right: Value) -> Value {
    if let (Value::Int(a), Value::Int(b)) = (&left, &right) {
        let out = match op {
            BinOp::Add => Some(Value::Int(a + b)),
            BinOp::Sub => Some(Value::Int(a - b)),
            BinOp::Mul => Some(Value::Int(a * b)),
            BinOp::Div => Some(Value::Float(*a as f64 / *b as f64)),
            BinOp::Rem => (*b != 0).then(|| Value::Int(a % b)),
            _ => None,
        };
        if let Some(v) = out {
            return v;
        }
    }
    if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
        let v = match op {
            BinOp::Add => a + b,
            BinOp::Sub => a - b,
            BinOp::Mul => a * b,
            BinOp::Div => a / b,
            BinOp::Rem => a % b,
            _ => return Value::Missing,
        };
        return Value::Float(v);
    }
    if op == BinOp::Add {
        return Value::Str(format!("{}{}", left.display(), right.display()));
    }
    Value::Missing
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

fn as_color(v: &Value) -> Option<Rgb> {
    match v {
        Value::Color(c) => Some(*c),
        Value::Str(s) => parse_color(s),
        _ => None,
    }
}

fn string_filter(input: Value, f: impl FnOnce(&str) -> String) -> Value {
    Value::Str(f(&input.display()))
}

fn numeric_filter(input: Value, f: impl FnOnce(f64) -> f64) -> Value {
    match input.as_f64() {
        Some(v) => Value::Int(f(v) as i64),
        None => Value::Missing,
    }
}

fn color_filter(input: Value, f: impl FnOnce(Rgb) -> Rgb) -> Value {
    match as_color(&input) {
        Some(c) => Value::Color(f(c)),
        None => Value::Missing,
    }
}

fn color_amount(input: Value, args: &[Value], f: impl FnOnce(Rgb, f32) -> Rgb) -> Value {
    match (as_color(&input), args.first().and_then(|a| a.as_f64())) {
        (Some(c), Some(amount)) => Value::Color(f(c, amount as f32)),
        _ => Value::Missing,
    }
}

/// Extract one RGB channel as 0-255 (`normalized == false`) or 0..1.
fn channel_filter(input: Value, idx: usize, normalized: bool) -> Value {
    match as_color(&input) {
        Some(c) => {
            let part = [c.r, c.g, c.b][idx];
            if normalized {
                Value::Str(format!("{part:.4}"))
            } else {
                Value::Str(((part * 255.0).round() as u8).to_string())
            }
        }
        None => Value::Missing,
    }
}

fn apply_filter(name: &str, input: Value, args: Vec<Value>) -> Value {
    match name {
        "default" => {
            if input.is_missing() {
                args.into_iter().next().unwrap_or(Value::Missing)
            } else {
                input
            }
        }
        "upper" => string_filter(input, |s| s.to_uppercase()),
        "lower" => string_filter(input, |s| s.to_lowercase()),
        "titlecase" => string_filter(input, |s| {
            s.split_whitespace()
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }),
        "trim" => string_filter(input, |s| s.trim().to_owned()),
        "length" => Value::Int(input.display().chars().count() as i64),
        "replace" => {
            let old = args.first().map(|a| a.display()).unwrap_or_default();
            let new = args.get(1).map(|a| a.display()).unwrap_or_default();
            Value::Str(input.display().replace(&old, &new))
        }
        "int" => match input.as_f64() {
            Some(v) => Value::Int(v as i64),
            None => Value::Missing,
        },
        "round" => numeric_filter(input, f64::round),
        "floor" => numeric_filter(input, f64::floor),
        "ceil" => numeric_filter(input, f64::ceil),
        "to_string" => Value::Str(input.display()),

        "hex" => match as_color(&input) {
            Some(c) => Value::Str(c.to_hex()),
            None => Value::Missing,
        },
        "rgb" => match as_color(&input) {
            Some(c) => Value::Str(c.to_rgb_string()),
            None => Value::Missing,
        },
        "rgba" => match as_color(&input) {
            Some(c) => Value::Str(c.to_rgba_string()),
            None => Value::Missing,
        },
        "strip" => match as_color(&input) {
            Some(c) => Value::Str(c.to_hex().trim_start_matches('#').to_owned()),
            None => Value::Str(input.display().trim_start_matches('#').to_owned()),
        },
        "red" | "green" | "blue" => {
            let idx = match name {
                "red" => 0,
                "green" => 1,
                _ => 2,
            };
            channel_filter(input, idx, false)
        }
        "red_n" | "green_n" | "blue_n" => {
            let idx = match name {
                "red_n" => 0,
                "green_n" => 1,
                _ => 2,
            };
            channel_filter(input, idx, true)
        }
        "lighten" => color_amount(input, &args, Rgb::lighten),
        "darken" => color_amount(input, &args, Rgb::darken),
        "saturate" => color_amount(input, &args, Rgb::saturate),
        "adjust_hue" => color_amount(input, &args, Rgb::adjust_hue),
        "alpha" => match (as_color(&input), args.first().and_then(|a| a.as_f64())) {
            (Some(c), Some(a)) => Value::Color(c.with_alpha(a as f32)),
            (Some(c), None) => Value::Float(c.a as f64),
            _ => Value::Missing,
        },
        "mix" => {
            let other = args.first().and_then(as_color);
            let amount = args.get(1).and_then(|a| a.as_f64());
            match (as_color(&input), other, amount) {
                (Some(c), Some(o), Some(t)) => Value::Color(c.mix(o, t as f32)),
                _ => Value::Missing,
            }
        }
        "invert" => color_filter(input, Rgb::invert),
        "complement" => color_filter(input, Rgb::complement),
        "grayscale" => color_filter(input, Rgb::grayscale),
        "contrast" => color_filter(input, Rgb::contrast),
        "luminance" => match as_color(&input) {
            Some(c) => Value::Float(c.luminance() as f64),
            None => Value::Missing,
        },
        "hsl" => match as_color(&input) {
            Some(c) => {
                let (h, s, l) = c.to_hsl();
                Value::Str(format!("{h:.0},{:.0}%,{:.0}%", s * 100.0, l * 100.0))
            }
            None => Value::Missing,
        },
        "oklab" => match as_color(&input) {
            Some(c) => {
                let (l, a, b) = c.to_oklab();
                Value::Str(format!("{l:.4},{a:.4},{b:.4}"))
            }
            None => Value::Missing,
        },
        "join" => match input {
            Value::List(items) => {
                let sep = args.first().map(|a| a.display()).unwrap_or_else(|| ",".into());
                Value::Str(items.iter().map(Value::display).collect::<Vec<_>>().join(&sep))
            }
            other => Value::Str(other.display()),
        },
        _ => Value::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::{eval_condition, eval_expr};
    use crate::render::value::Value;
    use std::collections::HashMap;

    fn context() -> HashMap<String, Value> {
        HashMap::from([
            ("accent".to_string(), Value::Str("#89b4fa".into())),
            ("background".to_string(), Value::Str("#1e1e2e".into())),
            ("radius".to_string(), Value::Int(8)),
            ("ratio".to_string(), Value::Float(0.5)),
            ("mode".to_string(), Value::Str("dark".into())),
            ("count".to_string(), Value::Int(3)),
            (
                "colors".to_string(),
                Value::List(vec![
                    Value::Str("#45475a".into()),
                    Value::Str("#f38ba8".into()),
                ]),
            ),
        ])
    }

    fn eval_to_string(expr: &str) -> String {
        let ctx = context();
        let empty = HashMap::new();
        eval_expr(expr, &ctx, &empty).display()
    }

    #[test]
    fn variables_and_literals() {
        assert_eq!(eval_to_string("accent"), "#89b4fa");
        assert_eq!(eval_to_string("radius"), "8");
        assert_eq!(eval_to_string("2 + 3"), "5");
        assert_eq!(eval_to_string("2 * 3"), "6");
        assert_eq!(eval_to_string("radius * 2 + 1"), "17");
        assert_eq!(eval_to_string("10 / 4"), "2.5");
        assert_eq!(eval_to_string("1 == 1"), "true");
        assert_eq!(eval_to_string("'a' + 'b'"), "ab");
    }

    #[test]
    fn comparison_and_boolean_ops() {
        let ctx = context();
        let empty = HashMap::new();
        assert!(eval_condition("mode == \"dark\"", &ctx, &empty));
        assert!(!eval_condition("mode != \"dark\"", &ctx, &empty));
        assert!(eval_condition("count > 2 and radius < 10", &ctx, &empty));
        assert!(eval_condition("count > 2 or radius > 100", &ctx, &empty));
        assert!(eval_condition("not count == 0", &ctx, &empty));
        assert!(eval_condition("radius >= 8", &ctx, &empty));
    }

    #[test]
    fn numeric_filters() {
        assert_eq!(eval_to_string("ratio | round"), "1");
        assert_eq!(eval_to_string("radius | to_string"), "8");
        assert_eq!(eval_to_string("3.7 | floor"), "3");
        assert_eq!(eval_to_string("3.2 | ceil"), "4");
    }

    #[test]
    fn string_filters() {
        assert_eq!(eval_to_string("'gnist' | upper"), "GNIST");
        assert_eq!(eval_to_string("'GNIST' | lower"), "gnist");
        assert_eq!(eval_to_string("'gnist' | length"), "5");
        assert_eq!(eval_to_string("'a-b' | replace '-' '_'"), "a_b");
    }

    #[test]
    fn color_format_filters() {
        assert_eq!(eval_to_string("accent | rgb"), "137,180,250");
        assert_eq!(eval_to_string("accent | rgba"), "rgba(137,180,250,1)");
        assert_eq!(eval_to_string("accent | hex"), "#89b4fa");
        assert_eq!(eval_to_string("accent | strip"), "89b4fa");
    }

    #[test]
    fn per_channel_filters() {
        assert_eq!(eval_to_string("accent | red"), "137");
        assert_eq!(eval_to_string("accent | green"), "180");
        assert_eq!(eval_to_string("accent | blue"), "250");
        assert_eq!(eval_to_string("accent | red_n"), "0.5373");
        assert_eq!(eval_to_string("accent | green_n"), "0.7059");
        assert_eq!(eval_to_string("accent | blue_n"), "0.9804");
    }

    #[test]
    fn color_arithmetic_filters() {
        assert_eq!(eval_to_string("accent | mix background 0.5"), "#546994");
        assert_eq!(eval_to_string("accent | lighten 0.1"), "#95bcfb");
        assert_eq!(eval_to_string("accent | darken 0.1"), "#7ba2e1");
        assert_eq!(eval_to_string("accent | alpha 0.5 | rgba"), "rgba(137,180,250,0.5)");
        assert_eq!(eval_to_string("accent | invert"), "#764b05");
        assert_eq!(eval_to_string("background | contrast"), "#ffffff");
        assert_eq!(eval_to_string("accent | complement"), "#facf89");
        assert_eq!(eval_to_string("accent | grayscale"), "#727272");
    }

    #[test]
    fn missing_resolves_to_missing_and_default_replaces_it() {
        let empty = HashMap::new();
        let ctx = HashMap::new();
        assert!(eval_expr("nope", &ctx, &empty).is_missing());
        assert_eq!(
            eval_expr("nope | default \"#fff\"", &ctx, &empty).display(),
            "#fff"
        );
        assert_eq!(eval_expr("nope | upper", &ctx, &empty).display(), "");
    }

    #[test]
    fn list_indexing() {
        assert_eq!(eval_to_string("colors[0]"), "#45475a");
        assert_eq!(eval_to_string("colors[1] | rgb"), "243,139,168");
        assert!(eval_to_string("colors[9]").is_empty());
    }
}

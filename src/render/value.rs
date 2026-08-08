//! Runtime values used by template expressions.

use crate::theme::colors::Rgb;

/// A value stored in the theme context or produced by an expression.
///
/// Plain `{{ key }}` placeholders substitute the raw source text, so colors
/// keep their original spelling. Filters and math operate on the typed form.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Color(Rgb),
    List(Vec<Value>),
    /// An unresolved name inside an expression.
    Missing,
}

impl Value {
    /// Text form when substituted directly into a template.
    pub fn display(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Color(c) => c.to_hex(),
            Value::List(items) => items
                .iter()
                .map(Value::display)
                .collect::<Vec<_>>()
                .join(","),
            Value::Missing => String::new(),
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Str(s) => !s.is_empty(),
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Color(_) => true,
            Value::List(items) => !items.is_empty(),
            Value::Missing => false,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn is_missing(&self) -> bool {
        matches!(self, Value::Missing)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Str(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Str(v.to_owned())
    }
}

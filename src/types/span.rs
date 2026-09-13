use crate::Predicate;
use serde::{Deserialize, Serialize};
use serde_json::Number;

/// A time span for derived types. Convention: UTC milliseconds, [start, end),
/// `end` None while open, `start == end` for an instant. The writer converts
/// phrases like "today" and keeps them in `text`; the core only sees numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub start: i64,
    #[serde(default)]
    pub end: Option<i64>,
}

impl Span {
    pub fn new(start: i64, end: Option<i64>) -> Self {
        Self {
            text: None,
            start,
            end,
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Spans at `field` sharing any instant with [start, end).
    pub fn overlap(field: &str, start: i64, end: i64) -> Predicate {
        Predicate::Overlap {
            field: field.into(),
            start: Number::from(start),
            end: Number::from(end),
        }
    }

    /// Spans at `field` holding the instant `t`.
    pub fn at(field: &str, t: i64) -> Predicate {
        Predicate::At {
            field: field.into(),
            value: Number::from(t),
        }
    }
}

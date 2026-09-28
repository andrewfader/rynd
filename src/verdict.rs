//! `Verdict` enum + `filter` builtin for filtermap-style scripts.
//!
//! In Roto, a `filtermap` block produces a [`Verdict::Accept`] or
//! [`Verdict::Reject`] value. Rynd ships the same enum and a `filter`
//! builtin so embedders can write the equivalent:
//!
//! ```ignore
//! filter(x)   // -> Verdict::Accept if x is non-zero/non-nil, else Reject
//! ```
use crate::vm::value::Value;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    Accept,
    Reject,
}

impl Verdict {
    pub fn from_value(value: &Value) -> Self {
        match value {
            Value::Nil => Self::Reject,
            Value::Int(n) => {
                if *n != 0 {
                    Self::Accept
                } else {
                    Self::Reject
                }
            }
            Value::Float(n) => {
                if *n != 0.0 {
                    Self::Accept
                } else {
                    Self::Reject
                }
            }
            Value::Bool(b) => {
                if *b {
                    Self::Accept
                } else {
                    Self::Reject
                }
            }
            _ => Self::Accept,
        }
    }
    pub fn to_value(self) -> Value {
        match self {
            Verdict::Accept => Value::string("accept"),
            Verdict::Reject => Value::string("reject"),
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Accept => write!(f, "accept"),
            Verdict::Reject => write!(f, "reject"),
        }
    }
}
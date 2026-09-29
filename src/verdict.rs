//! Filter-map verdicts: a script decides whether to accept or reject an input
//! and may transform it on the way through.
//!
//! Scripts return `Accept(value)` or `Reject(value)`; any other value is judged
//! by truthiness and passed through unchanged. `Reject` is falsey, so verdict
//! functions also work with `filter`, and `filter_map` keeps accepted payloads.
//!
//! ```
//! use rynd::{RyndEngine, Value, Verdict};
//!
//! let mut engine = RyndEngine::new();
//! engine.eval(r#"
//!     fn route(prefix) {
//!         if prefix == "0.0.0.0/0" { Reject("default route") } else { Accept(upper(prefix)) }
//!     }
//! "#).unwrap();
//!
//! let (verdict, value) = engine.filtermap("route", &[Value::from("10.0.0.0/8")]).unwrap();
//! assert_eq!(verdict, Verdict::Accept);
//! assert_eq!(value, Value::from("10.0.0.0/8"));
//! let (verdict, _) = engine.filtermap("route", &[Value::from("0.0.0.0/0")]).unwrap();
//! assert_eq!(verdict, Verdict::Reject);
//! ```
use crate::vm::value::Value;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    Accept,
    Reject,
}

impl Verdict {
    /// Judge a script result: `Accept(_)`/`Reject(_)` decide explicitly,
    /// anything else accepts when truthy.
    pub fn from_value(value: &Value) -> Self {
        Self::split(value.clone()).0
    }

    /// Split a script result into its verdict and payload. `Accept(x)` and
    /// `Reject(x)` yield `x`; other values are their own payload.
    pub fn split(value: Value) -> (Self, Value) {
        match value {
            Value::Variant { name, values } if values.len() == 1 && name == "Accept" => {
                (Self::Accept, values[0].clone())
            }
            Value::Variant { name, values } if values.len() == 1 && name == "Reject" => {
                (Self::Reject, values[0].clone())
            }
            other if other.is_truthy() => (Self::Accept, other),
            other => (Self::Reject, other),
        }
    }

    /// Wrap `payload` as the script-level `Accept(payload)`/`Reject(payload)`.
    pub fn to_value(self, payload: Value) -> Value {
        Value::variant(self.to_string(), vec![payload])
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Verdict::Accept => "Accept",
            Verdict::Reject => "Reject",
        })
    }
}

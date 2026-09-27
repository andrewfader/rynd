use crate::error::RyndResult;
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::fmt;
use std::rc::Rc;

pub type NativeFunction = fn(&[Value]) -> RyndResult<Value>;
pub type CompiledFunction =
    dyn Fn(&mut dyn super::runtime::Runtime, &[Value], &Value) -> RyndResult<Value>;

#[derive(Clone)]
pub enum Value {
    Builtin {
        name: Rc<str>,
        arity: usize,
    },
    Compiled {
        name: Rc<str>,
        arity: usize,
        func: Rc<CompiledFunction>,
    },
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Rc<String>),
    Tuple(Rc<Vec<Value>>),
    List(Rc<Vec<Value>>),
    Map(Rc<BTreeMap<String, Value>>),
    Variant {
        name: String,
        values: Rc<Vec<Value>>,
    },
    Function {
        name: String,
        arity: usize,
        chunk_index: usize,
    },
    Closure {
        chunk_index: usize,
        arity: usize,
        name: Rc<str>,
        upvalues: Rc<Vec<Value>>,
    },
    Native {
        name: String,
        arity: usize,
        func: NativeFunction,
    },
}

impl Value {
    pub fn string(s: impl Into<String>) -> Self {
        Value::String(Rc::new(s.into()))
    }

    pub fn list(elements: Vec<Value>) -> Self {
        Value::List(Rc::new(elements))
    }

    pub fn tuple(elements: Vec<Value>) -> Self {
        Value::Tuple(Rc::new(elements))
    }

    pub fn map(pairs: BTreeMap<String, Value>) -> Self {
        Value::Map(Rc::new(pairs))
    }

    pub fn variant(name: impl Into<String>, values: Vec<Value>) -> Self {
        Value::Variant {
            name: name.into(),
            values: Rc::new(values),
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Nil => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0 && !f.is_nan(),
            Value::String(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Map(m) => !m.is_empty(),
            Value::Tuple(_) => true,
            Value::Variant { name, values } => {
                if name == "None" || name == "Err" {
                    false
                } else if name == "Some" && values.len() == 1 {
                    values[0].is_truthy()
                } else {
                    true
                }
            }
            Value::Function { .. }
            | Value::Closure { .. }
            | Value::Native { .. }
            | Value::Builtin { .. }
            | Value::Compiled { .. } => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Builtin { .. } => "native_function",
            Value::Compiled { .. } => "closure",
            Value::Nil => "nil",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Tuple(_) => "tuple",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Variant { .. } => "variant",
            Value::Function { .. } => "function",
            Value::Closure { .. } => "closure",
            Value::Native { .. } => "native_function",
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
                super::runtime::compare_int_float(*a, *b) == Some(std::cmp::Ordering::Equal)
            }
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Tuple(a), Value::Tuple(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => a == b,
            (
                Value::Variant {
                    name: n1,
                    values: v1,
                },
                Value::Variant {
                    name: n2,
                    values: v2,
                },
            ) => n1 == n2 && v1 == v2,
            (
                Value::Function {
                    chunk_index: c1, ..
                },
                Value::Function {
                    chunk_index: c2, ..
                },
            ) => c1 == c2,
            (Value::Native { name: n1, .. }, Value::Native { name: n2, .. }) => n1 == n2,
            (Value::Builtin { name: a, .. }, Value::Builtin { name: b, .. }) => a == b,
            (Value::Compiled { func: a, .. }, Value::Compiled { func: b, .. }) => Rc::ptr_eq(a, b),
            (
                Value::Closure {
                    upvalues: a,
                    chunk_index: i,
                    ..
                },
                Value::Closure {
                    upvalues: b,
                    chunk_index: j,
                    ..
                },
            ) => i == j && Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Builtin { name, arity } => write!(f, "<native fn {name}/{arity}>"),
            Value::Compiled { name, arity, .. } => write!(f, "<closure {name}/{arity}>"),
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(fl) => write!(f, "{fl}"),
            Value::String(s) => write!(f, "{s}"),
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")
            }
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            Value::Map(map) => {
                write!(f, "{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{k}\": {v}")?;
                }
                write!(f, "}}")
            }
            Value::Variant { name, values } => {
                if values.is_empty() {
                    write!(f, "{name}")
                } else {
                    write!(f, "{name}(")?;
                    for (i, v) in values.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{v}")?;
                    }
                    write!(f, ")")
                }
            }
            Value::Function { name, arity, .. } => {
                write!(f, "<fn {name}/{arity}>")
            }
            Value::Closure { name, arity, .. } => {
                write!(f, "<closure {name}/{arity}>")
            }
            Value::Native { name, arity, .. } => {
                write!(f, "<native fn {name}/{arity}>")
            }
        }
    }
}

// Exact scalar conversions keep adapter validation explicit and lossless.
macro_rules! scalar_conversion {
    ($rust:ty, $variant:ident, $expected:literal) => {
        impl From<$rust> for Value {
            fn from(value: $rust) -> Self {
                Self::$variant(value)
            }
        }
        impl TryFrom<&Value> for $rust {
            type Error = crate::error::RyndError;
            fn try_from(value: &Value) -> Result<Self, Self::Error> {
                match value {
                    Value::$variant(inner) => Ok(*inner),
                    _ => Err(super::runtime::error(format!(
                        "Expected {}, got {}",
                        $expected,
                        value.type_name()
                    ))),
                }
            }
        }
        impl TryFrom<Value> for $rust {
            type Error = crate::error::RyndError;
            fn try_from(value: Value) -> Result<Self, Self::Error> {
                Self::try_from(&value)
            }
        }
    };
}
scalar_conversion!(i64, Int, "int");
scalar_conversion!(f64, Float, "float");
scalar_conversion!(bool, Bool, "bool");

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::string(value)
    }
}
impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::string(value)
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(values: Vec<T>) -> Self {
        Self::list(values.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Value>> From<BTreeMap<String, T>> for Value {
    fn from(values: BTreeMap<String, T>) -> Self {
        Self::map(
            values
                .into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect(),
        )
    }
}
impl<'a> TryFrom<&'a Value> for &'a str {
    type Error = crate::error::RyndError;
    fn try_from(value: &'a Value) -> Result<Self, Self::Error> {
        match value {
            Value::String(inner) => Ok(inner.as_str()),
            _ => Err(super::runtime::error(format!(
                "Expected string, got {}",
                value.type_name()
            ))),
        }
    }
}
impl TryFrom<&Value> for String {
    type Error = crate::error::RyndError;
    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        <&str>::try_from(value).map(str::to_owned)
    }
}
impl TryFrom<Value> for String {
    type Error = crate::error::RyndError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::String(inner) => Ok(Rc::unwrap_or_clone(inner)),
            other => Self::try_from(&other),
        }
    }
}

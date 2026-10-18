use crate::error::RyndResult;
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::fmt;
use std::rc::Rc;

pub type NativeFunction = fn(&[Value]) -> RyndResult<Value>;
pub type CompiledFunction =
    dyn Fn(&mut dyn super::runtime::Runtime, &[Value], &Value) -> RyndResult<Value>;

/// A script value: scalars inline, everything else behind one `Rc`, so a
/// `Value` is two words and clones are a reference-count increment.
#[derive(Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Rc<String>),
    Tuple(Rc<Items>),
    List(Rc<Items>),
    Map(Rc<Entries>),
    /// A tagged value such as `Some(x)`, `Err(e)`, or `None`.
    Variant(Rc<Variant>),
    /// A builtin, identified by its entry in a static builtin table.
    Builtin(&'static (&'static str, usize)),
    /// A Rust closure: host callbacks and generated native code.
    Compiled(Rc<Compiled>),
    Function(Rc<Function>),
    Closure(Rc<Closure>),
    Native(Rc<Native>),
}

/// The items of a list or tuple. Derefs to `Vec<Value>`.
#[derive(Clone, Default)]
pub struct Items(Vec<Value>);

/// The entries of a map. Derefs to `BTreeMap<String, Value>`.
#[derive(Clone, Default)]
pub struct Entries(BTreeMap<String, Value>);

impl Items {
    pub fn into_vec(mut self) -> Vec<Value> {
        std::mem::take(&mut self.0)
    }
}
impl Entries {
    pub fn into_map(mut self) -> BTreeMap<String, Value> {
        std::mem::take(&mut self.0)
    }
}
impl std::ops::Deref for Items {
    type Target = Vec<Value>;
    fn deref(&self) -> &Vec<Value> {
        &self.0
    }
}
impl std::ops::DerefMut for Items {
    fn deref_mut(&mut self) -> &mut Vec<Value> {
        &mut self.0
    }
}
impl std::ops::Deref for Entries {
    type Target = BTreeMap<String, Value>;
    fn deref(&self) -> &BTreeMap<String, Value> {
        &self.0
    }
}
impl std::ops::DerefMut for Entries {
    fn deref_mut(&mut self) -> &mut BTreeMap<String, Value> {
        &mut self.0
    }
}
impl From<Vec<Value>> for Items {
    fn from(items: Vec<Value>) -> Self {
        Items(items)
    }
}
impl From<BTreeMap<String, Value>> for Entries {
    fn from(entries: BTreeMap<String, Value>) -> Self {
        Entries(entries)
    }
}
impl PartialEq for Items {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl PartialEq for Entries {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Drop for Items {
    fn drop(&mut self) {
        release(std::mem::take(&mut self.0));
    }
}
impl Drop for Entries {
    fn drop(&mut self) {
        release(std::mem::take(&mut self.0).into_values());
    }
}

pub struct Variant {
    pub name: Rc<str>,
    pub values: Vec<Value>,
}
impl Drop for Variant {
    fn drop(&mut self) {
        release(std::mem::take(&mut self.values));
    }
}

pub struct Compiled {
    pub name: Rc<str>,
    pub arity: usize,
    pub func: Box<CompiledFunction>,
}

/// A bytecode function without captures.
pub struct Function {
    pub name: String,
    pub arity: usize,
    pub chunk_index: usize,
}

/// A bytecode function with the values it captured when it was created.
pub struct Closure {
    pub name: Rc<str>,
    pub arity: usize,
    pub chunk_index: usize,
    pub upvalues: Vec<Value>,
}
impl Drop for Closure {
    fn drop(&mut self) {
        release(std::mem::take(&mut self.upvalues));
    }
}

pub struct Native {
    pub name: String,
    pub arity: usize,
    pub func: NativeFunction,
}

impl Value {
    // Constructors report each new value to the run's budget (a no-op
    // unless the engine has limits; see `safety::created`).

    pub fn string(s: impl Into<String>) -> Self {
        let s = s.into();
        super::safety::created(0, s.len() + 32);
        Value::String(Rc::new(s))
    }

    pub fn list(elements: Vec<Value>) -> Self {
        super::safety::created(elements.len(), elements.len() * 16 + 48);
        Value::List(Rc::new(Items(elements)))
    }

    pub fn tuple(elements: Vec<Value>) -> Self {
        super::safety::created(elements.len(), elements.len() * 16 + 48);
        Value::Tuple(Rc::new(Items(elements)))
    }

    pub fn map(pairs: BTreeMap<String, Value>) -> Self {
        super::safety::created(pairs.len(), pairs.len() * 64 + 48);
        Value::Map(Rc::new(Entries(pairs)))
    }

    pub fn variant(name: impl Into<Rc<str>>, values: Vec<Value>) -> Self {
        super::safety::created(values.len(), values.len() * 16 + 64);
        Value::Variant(Rc::new(Variant {
            name: name.into(),
            values,
        }))
    }

    pub fn compiled(
        name: impl Into<Rc<str>>,
        arity: usize,
        func: impl Fn(&mut dyn super::runtime::Runtime, &[Value], &Value) -> RyndResult<Value> + 'static,
    ) -> Self {
        Value::Compiled(Rc::new(Compiled {
            name: name.into(),
            arity,
            func: Box::new(func),
        }))
    }

    pub fn native(name: impl Into<String>, arity: usize, func: NativeFunction) -> Self {
        Value::Native(Rc::new(Native {
            name: name.into(),
            arity,
            func,
        }))
    }

    pub fn closure(
        name: impl Into<Rc<str>>,
        arity: usize,
        chunk_index: usize,
        upvalues: Vec<Value>,
    ) -> Self {
        Value::Closure(Rc::new(Closure {
            name: name.into(),
            arity,
            chunk_index,
            upvalues,
        }))
    }

    pub fn function(name: impl Into<String>, arity: usize, chunk_index: usize) -> Self {
        Value::Function(Rc::new(Function {
            name: name.into(),
            arity,
            chunk_index,
        }))
    }

    /// The tag and payload of a variant value.
    pub fn as_variant(&self) -> Option<(&str, &[Value])> {
        match self {
            Value::Variant(v) => Some((&v.name, &v.values)),
            _ => None,
        }
    }

    /// The single payload of `Tag(value)`.
    pub fn variant_payload(&self, tag: &str) -> Option<&Value> {
        match self.as_variant() {
            Some((name, [value])) if name == tag => Some(value),
            _ => None,
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
            Value::Variant(v) => match (&*v.name, v.values.as_slice()) {
                ("None" | "Err" | "Reject", _) => false,
                ("Some", [inner]) => inner.is_truthy(),
                _ => true,
            },
            Value::Function(_)
            | Value::Closure(_)
            | Value::Native(_)
            | Value::Builtin(_)
            | Value::Compiled(_) => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Builtin(_) => "native_function",
            Value::Compiled(_) => "closure",
            Value::Nil => "nil",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Tuple(_) => "tuple",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Variant(_) => "variant",
            Value::Function(_) => "function",
            Value::Closure(_) => "closure",
            Value::Native(_) => "native_function",
        }
    }

    /// Callable arity, or `None` for values that cannot be called.
    pub fn arity(&self) -> Option<usize> {
        match self {
            Value::Builtin(def) => Some(def.1),
            Value::Compiled(f) => Some(f.arity),
            Value::Function(f) => Some(f.arity),
            Value::Closure(f) => Some(f.arity),
            Value::Native(f) => Some(f.arity),
            _ => None,
        }
    }

    /// Equality of everything but nested containers, which `eq` walks.
    fn shallow_eq(&self, other: &Self) -> Option<bool> {
        Some(match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
                super::runtime::compare_int_float(*a, *b) == Some(std::cmp::Ordering::Equal)
            }
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Tuple(a), Value::Tuple(b)) | (Value::List(a), Value::List(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Some(true);
                }
                if a.len() != b.len() {
                    return Some(false);
                }
                return None;
            }
            (Value::Map(a), Value::Map(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Some(true);
                }
                if a.len() != b.len() || !a.keys().eq(b.keys()) {
                    return Some(false);
                }
                return None;
            }
            (Value::Variant(a), Value::Variant(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Some(true);
                }
                if a.name != b.name || a.values.len() != b.values.len() {
                    return Some(false);
                }
                return None;
            }
            (Value::Function(a), Value::Function(b)) => a.chunk_index == b.chunk_index,
            (Value::Native(a), Value::Native(b)) => a.name == b.name,
            (Value::Builtin(a), Value::Builtin(b)) => a.0 == b.0,
            (Value::Compiled(a), Value::Compiled(b)) => Rc::ptr_eq(a, b),
            (Value::Closure(a), Value::Closure(b)) => Rc::ptr_eq(a, b),
            _ => false,
        })
    }
}

thread_local! {
    /// Nesting of `Value` drops in progress on this thread, and container
    /// contents set aside once that nesting is deep (see `Drop for Value`).
    static DROPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static DEFERRED: std::cell::RefCell<Vec<Value>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Containers nested deeper than this are released iteratively.
const DROP_NESTING: usize = 256;

/// Release a container's contents while bounding native recursion: a list
/// nested a million levels deep must not overflow the stack when it goes out
/// of scope. Past `DROP_NESTING` levels, contents are set aside and the
/// outermost release drops them iteratively.
fn release(values: impl IntoIterator<Item = Value>) {
    let depth = DROPS.with(|d| d.get());
    if depth >= DROP_NESTING {
        // During thread teardown the list may be gone; drop in place then.
        let mut values = Some(values);
        let _ = DEFERRED.try_with(|deferred| {
            if let Ok(mut deferred) = deferred.try_borrow_mut() {
                deferred.extend(values.take().into_iter().flatten());
            }
        });
        drop(values);
        return;
    }
    DROPS.with(|d| d.set(depth + 1));
    for value in values {
        drop(value);
    }
    if depth == 0 {
        // The outermost release finishes whatever deeper levels set aside,
        // still counted as one level deep so nested releases never drain.
        while let Some(value) = DEFERRED
            .try_with(|d| d.try_borrow_mut().ok().and_then(|mut d| d.pop()))
            .ok()
            .flatten()
        {
            drop(value);
        }
    }
    DROPS.with(|d| d.set(depth));
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        if let Some(equal) = self.shallow_eq(other) {
            return equal;
        }
        // Containers compare with an explicit work list, not recursion.
        let mut pending: Vec<(&Value, &Value)> = vec![(self, other)];
        while let Some((a, b)) = pending.pop() {
            match a.shallow_eq(b) {
                Some(true) => {}
                Some(false) => return false,
                None => match (a, b) {
                    (Value::Tuple(x), Value::Tuple(y)) | (Value::List(x), Value::List(y)) => {
                        pending.extend(x.iter().zip(y.iter()));
                    }
                    (Value::Map(x), Value::Map(y)) => pending.extend(x.values().zip(y.values())),
                    (Value::Variant(x), Value::Variant(y)) => {
                        pending.extend(x.values.iter().zip(y.values.iter()));
                    }
                    _ => return false,
                },
            }
        }
        true
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

thread_local! {
    static DISPLAY_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Containers nested deeper than this print as `...`.
const DISPLAY_NESTING: usize = 512;

fn write_items(f: &mut fmt::Formatter<'_>, items: &[Value]) -> fmt::Result {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{item}")?;
    }
    Ok(())
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nested = matches!(
            self,
            Value::Tuple(_) | Value::List(_) | Value::Map(_) | Value::Variant(_)
        );
        let depth = DISPLAY_DEPTH.with(|d| d.get());
        if nested && depth >= DISPLAY_NESTING {
            return write!(f, "...");
        }
        DISPLAY_DEPTH.with(|d| d.set(depth + usize::from(nested)));
        let result = match self {
            Value::Builtin((name, arity)) => write!(f, "<native fn {name}/{arity}>"),
            Value::Compiled(c) => write!(f, "<closure {}/{}>", c.name, c.arity),
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(fl) => write!(f, "{fl}"),
            Value::String(s) => write!(f, "{s}"),
            Value::Tuple(items) => write!(f, "(")
                .and_then(|_| write_items(f, items))
                .and_then(|_| write!(f, ")")),
            Value::List(items) => write!(f, "[")
                .and_then(|_| write_items(f, items))
                .and_then(|_| write!(f, "]")),
            Value::Map(map) => (|| {
                write!(f, "{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{k}\": {v}")?;
                }
                write!(f, "}}")
            })(),
            Value::Variant(v) => {
                if v.values.is_empty() {
                    write!(f, "{}", v.name)
                } else {
                    write!(f, "{}(", v.name)
                        .and_then(|_| write_items(f, &v.values))
                        .and_then(|_| write!(f, ")"))
                }
            }
            Value::Function(func) => write!(f, "<fn {}/{}>", func.name, func.arity),
            Value::Closure(c) => write!(f, "<closure {}/{}>", c.name, c.arity),
            Value::Native(n) => write!(f, "<native fn {}/{}>", n.name, n.arity),
        };
        DISPLAY_DEPTH.with(|d| d.set(depth));
        result
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
        match value.into_string() {
            Ok(inner) => Ok(Rc::unwrap_or_clone(inner)),
            Err(other) => Self::try_from(&other),
        }
    }
}

impl Value {
    /// Move a string's `Rc` out without cloning its contents.
    pub fn into_string(self) -> Result<Rc<String>, Value> {
        match self {
            Value::String(rc) => Ok(rc),
            other => Err(other),
        }
    }
    /// Move a map out, cloning it only if it is shared.
    pub fn into_map(self) -> Result<BTreeMap<String, Value>, Value> {
        match self {
            Value::Map(rc) => Ok(Rc::unwrap_or_clone(rc).into_map()),
            other => Err(other),
        }
    }
    /// Move a list's or tuple's items out, cloning them only if shared.
    pub fn into_items(self) -> Result<Vec<Value>, Value> {
        match self {
            Value::List(rc) | Value::Tuple(rc) => Ok(Rc::unwrap_or_clone(rc).into_vec()),
            other => Err(other),
        }
    }
}

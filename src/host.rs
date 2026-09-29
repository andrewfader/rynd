//! Typed host boundary: register ordinary Rust functions and closures, move
//! Rust structs in and out of scripts, and bundle host APIs into a [`Library`].
//!
//! Arguments convert with [`FromValue`] and results with [`IntoValue`], so a
//! host function is just a Rust function:
//!
//! ```
//! use rynd::{RyndEngine, RyndResult, Value};
//!
//! fn clamp(x: i64, lo: i64, hi: i64) -> i64 { x.clamp(lo, hi) }
//!
//! let mut engine = RyndEngine::new();
//! engine.register("clamp", clamp);
//! engine.register("greet", |name: String| format!("hello {name}"));
//! engine.register("parse_port", |text: String| -> RyndResult<u16> {
//!     text.parse().map_err(|_| rynd::vm::runtime::error(format!("bad port {text}")))
//! });
//! engine.register("first_even", |xs: Vec<i64>| xs.into_iter().find(|x| x % 2 == 0));
//!
//! assert_eq!(engine.eval("clamp(99, 0, 10)").unwrap(), Value::Int(10));
//! assert_eq!(engine.eval("greet('Ada')").unwrap(), Value::from("hello Ada"));
//! assert_eq!(engine.eval("parse_port('8080')").unwrap(), Value::Int(8080));
//! assert!(engine.eval("parse_port('http')").is_err());
//! assert_eq!(engine.eval("first_even([1, 3, 4])").unwrap().to_string(), "Some(4)");
//! ```
//!
//! [`record!`](crate::record) derives the conversions for structs, and
//! [`register!`](crate::register) registers functions under their own names.
use crate::RyndEngine;
use crate::error::RyndResult;
use crate::vm::runtime::error;
use crate::vm::value::Value;
use std::collections::{BTreeMap, HashMap};
use std::ops::Deref;
use std::rc::Rc;

/// Convert a script value into a Rust argument.
pub trait FromValue: Sized {
    fn from_value(value: &Value) -> RyndResult<Self>;
}

/// Convert a Rust result into a script value. `Err` from a `RyndResult`
/// becomes a script runtime error.
pub trait IntoValue {
    fn into_value(self) -> RyndResult<Value>;
}

impl FromValue for Value {
    fn from_value(value: &Value) -> RyndResult<Self> {
        Ok(value.clone())
    }
}
impl IntoValue for Value {
    fn into_value(self) -> RyndResult<Value> {
        Ok(self)
    }
}

macro_rules! exact {
    ($($ty:ty),*) => {$(
        impl FromValue for $ty {
            fn from_value(value: &Value) -> RyndResult<Self> {
                <$ty>::try_from(value)
            }
        }
        impl IntoValue for $ty {
            fn into_value(self) -> RyndResult<Value> {
                Ok(self.into())
            }
        }
    )*};
}
exact!(i64, bool, String);

// Integers narrower or wider than i64 are range-checked in both directions.
macro_rules! integer {
    ($($ty:ty),*) => {$(
        impl FromValue for $ty {
            fn from_value(value: &Value) -> RyndResult<Self> {
                let n = i64::try_from(value)?;
                <$ty>::try_from(n).map_err(|_| {
                    error(format!("{n} is out of range for {}", stringify!($ty)))
                })
            }
        }
        impl IntoValue for $ty {
            fn into_value(self) -> RyndResult<Value> {
                i64::try_from(self)
                    .map(Value::Int)
                    .map_err(|_| error(format!("{self} does not fit in a Rynd int")))
            }
        }
    )*};
}
integer!(i8, i16, i32, isize, u8, u16, u32, u64, usize);

/// Hosts asking for a float accept script ints too, since `2` and `2.0`
/// are the same quantity in Rynd arithmetic.
impl FromValue for f64 {
    fn from_value(value: &Value) -> RyndResult<Self> {
        match value {
            Value::Int(n) => Ok(*n as f64),
            other => f64::try_from(other),
        }
    }
}
impl IntoValue for f64 {
    fn into_value(self) -> RyndResult<Value> {
        Ok(Value::Float(self))
    }
}

impl IntoValue for &str {
    fn into_value(self) -> RyndResult<Value> {
        Ok(Value::string(self))
    }
}

impl IntoValue for () {
    fn into_value(self) -> RyndResult<Value> {
        Ok(Value::Nil)
    }
}

impl<T: IntoValue> IntoValue for RyndResult<T> {
    fn into_value(self) -> RyndResult<Value> {
        self?.into_value()
    }
}

/// `Option` maps to Rynd's `Some(value)` / `None`; `nil` also reads as `None`.
impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: &Value) -> RyndResult<Self> {
        match value {
            Value::Nil => Ok(None),
            Value::Variant { name, values } if name == "None" && values.is_empty() => Ok(None),
            Value::Variant { name, values } if name == "Some" && values.len() == 1 => {
                T::from_value(&values[0]).map(Some)
            }
            other => T::from_value(other).map(Some),
        }
    }
}
impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> RyndResult<Value> {
        Ok(match self {
            Some(value) => Value::variant("Some", vec![value.into_value()?]),
            None => Value::variant("None", vec![]),
        })
    }
}

impl<T: FromValue> FromValue for Vec<T> {
    fn from_value(value: &Value) -> RyndResult<Self> {
        match value {
            Value::List(items) | Value::Tuple(items) => items.iter().map(T::from_value).collect(),
            other => Err(error(format!("Expected list, got {}", other.type_name()))),
        }
    }
}
impl<T: IntoValue> IntoValue for Vec<T> {
    fn into_value(self) -> RyndResult<Value> {
        Ok(Value::list(
            self.into_iter()
                .map(IntoValue::into_value)
                .collect::<RyndResult<_>>()?,
        ))
    }
}

impl<T: FromValue> FromValue for BTreeMap<String, T> {
    fn from_value(value: &Value) -> RyndResult<Self> {
        match value {
            Value::Map(map) => map
                .iter()
                .map(|(key, value)| Ok((key.clone(), T::from_value(value)?)))
                .collect(),
            other => Err(error(format!("Expected map, got {}", other.type_name()))),
        }
    }
}
impl<T: IntoValue> IntoValue for BTreeMap<String, T> {
    fn into_value(self) -> RyndResult<Value> {
        Ok(Value::map(
            self.into_iter()
                .map(|(key, value)| Ok((key, value.into_value()?)))
                .collect::<RyndResult<_>>()?,
        ))
    }
}
impl<T: FromValue> FromValue for HashMap<String, T> {
    fn from_value(value: &Value) -> RyndResult<Self> {
        Ok(BTreeMap::<String, T>::from_value(value)?
            .into_iter()
            .collect())
    }
}
impl<T: IntoValue> IntoValue for HashMap<String, T> {
    fn into_value(self) -> RyndResult<Value> {
        self.into_iter().collect::<BTreeMap<_, _>>().into_value()
    }
}

macro_rules! tuple {
    ($len:literal: $($name:ident $index:tt),+) => {
        impl<$($name: FromValue),+> FromValue for ($($name,)+) {
            fn from_value(value: &Value) -> RyndResult<Self> {
                match value {
                    Value::Tuple(items) | Value::List(items) if items.len() == $len => {
                        Ok(($($name::from_value(&items[$index])?,)+))
                    }
                    other => Err(error(format!(
                        "Expected {}-tuple, got {other}",
                        $len
                    ))),
                }
            }
        }
        impl<$($name: IntoValue),+> IntoValue for ($($name,)+) {
            fn into_value(self) -> RyndResult<Value> {
                Ok(Value::tuple(vec![$(self.$index.into_value()?),+]))
            }
        }
    };
}
tuple!(2: A 0, B 1);
tuple!(3: A 0, B 1, C 2);
tuple!(4: A 0, B 1, C 2, D 3);

/// A typed wrapper naming the Rust type exchanged with a script.
#[derive(Clone, Debug, PartialEq)]
pub struct Val<T> {
    inner: T,
}

impl<T> Val<T> {
    pub fn new(value: T) -> Self {
        Self { inner: value }
    }
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> Deref for Val<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> From<T> for Val<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: IntoValue + Clone> Val<T> {
    /// The dynamic [`Value`] a script sees for this wrapper.
    pub fn to_value(&self) -> RyndResult<Value> {
        self.inner.clone().into_value()
    }
}

impl<T: FromValue> TryFrom<&Value> for Val<T> {
    type Error = crate::RyndError;
    fn try_from(value: &Value) -> RyndResult<Self> {
        T::from_value(value).map(Self::new)
    }
}
impl<T: FromValue> FromValue for Val<T> {
    fn from_value(value: &Value) -> RyndResult<Self> {
        T::from_value(value).map(Self::new)
    }
}
impl<T: IntoValue> IntoValue for Val<T> {
    fn into_value(self) -> RyndResult<Value> {
        self.inner.into_value()
    }
}

/// A Rust function or closure callable from scripts. `Args` is the tuple of
/// argument types; it exists only to keep implementations for different
/// arities apart and is always inferred.
pub trait HostFn<Args>: 'static {
    const ARITY: usize;
    fn call(&self, args: &[Value]) -> RyndResult<Value>;
}

macro_rules! host_fn {
    ($arity:literal $(; $($arg:ident $index:tt),*)?) => { host_fn!(@impl $arity; $($($arg $index),*)?); };
    (@impl $arity:literal; $($arg:ident $index:tt),*) => {
        impl<Func, Ret, $($arg),*> HostFn<($($arg,)*)> for Func
        where
            Func: Fn($($arg),*) -> Ret + 'static,
            Ret: IntoValue,
            $($arg: FromValue,)*
        {
            const ARITY: usize = $arity;
            #[allow(unused_variables)]
            fn call(&self, args: &[Value]) -> RyndResult<Value> {
                self($($arg::from_value(&args[$index])?),*).into_value()
            }
        }
    };
}
host_fn!(0);
host_fn!(1; A 0);
host_fn!(2; A 0, B 1);
host_fn!(3; A 0, B 1, C 2);
host_fn!(4; A 0, B 1, C 2, D 3);
host_fn!(5; A 0, B 1, C 2, D 3, E 4);
host_fn!(6; A 0, B 1, C 2, D 3, E 4, F 5);
host_fn!(7; A 0, B 1, C 2, D 3, E 4, F 5, G 6);
host_fn!(8; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);

/// Anything that can be installed into an engine under a name. Implemented
/// for every [`HostFn`]; `engine.register(name, f)` is the usual spelling.
pub trait Registerable<Args> {
    fn register(engine: &mut RyndEngine, name: &str, func: Self);
}

impl<Args, F: HostFn<Args>> Registerable<Args> for F {
    fn register(engine: &mut RyndEngine, name: &str, func: Self) {
        engine.register(name, func);
    }
}

/// Host data installed as script globals, one per field. Implemented by
/// [`record!`](crate::record) structs.
pub trait Context {
    fn install(&self, engine: &mut RyndEngine) -> RyndResult<()>;
}

type Callback = Rc<dyn Fn(&[Value]) -> RyndResult<Value>>;

/// A reusable set of host functions and constants. Build it once, then
/// install it into any number of engines or hand it to a [`crate::ScriptRuntime`].
///
/// ```
/// use rynd::{Library, RyndEngine, Value};
///
/// let mut lib = Library::new();
/// lib.register("double", |x: i64| x * 2);
/// lib.constant("LIMIT", 10_i64).unwrap();
///
/// let mut engine = RyndEngine::with_library(&lib);
/// assert_eq!(engine.eval("double(LIMIT)").unwrap(), Value::Int(20));
/// ```
#[derive(Clone, Default)]
pub struct Library {
    functions: BTreeMap<String, (usize, Callback)>,
    constants: BTreeMap<String, Value>,
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a host function; replaces any earlier item with the same name.
    pub fn register<Args, F: HostFn<Args>>(&mut self, name: impl Into<String>, func: F) {
        let name = name.into();
        self.constants.remove(&name);
        self.functions
            .insert(name, (F::ARITY, Rc::new(move |args| func.call(args))));
    }

    /// Add a constant global; replaces any earlier item with the same name.
    pub fn constant(&mut self, name: impl Into<String>, value: impl IntoValue) -> RyndResult<()> {
        let name = name.into();
        self.functions.remove(&name);
        self.constants.insert(name, value.into_value()?);
        Ok(())
    }

    /// Add every item of `other`, which wins on name clashes.
    pub fn extend(&mut self, other: &Library) {
        for (name, function) in &other.functions {
            self.constants.remove(name);
            self.functions.insert(name.clone(), function.clone());
        }
        for (name, value) in &other.constants {
            self.functions.remove(name);
            self.constants.insert(name.clone(), value.clone());
        }
    }

    /// Sorted names of every function and constant.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .functions
            .keys()
            .chain(self.constants.keys())
            .cloned()
            .collect();
        names.sort();
        names
    }

    pub fn contains(&self, name: &str) -> bool {
        self.functions.contains_key(name) || self.constants.contains_key(name)
    }

    /// Install every item. Functions survive [`RyndEngine::reset`]; constants
    /// are ordinary globals.
    pub fn install(&self, engine: &mut RyndEngine) {
        for (name, (arity, callback)) in &self.functions {
            let callback = callback.clone();
            engine.register_closure(name, *arity, move |args| callback(args));
        }
        for (name, value) in &self.constants {
            engine.set_global(name, value.clone());
        }
    }
}

/// Prefix a conversion error with the record field it came from.
#[doc(hidden)]
pub fn field_error(record: &str, field: &str, error: crate::RyndError) -> crate::RyndError {
    let message = match error {
        crate::RyndError::RuntimeError { message } => message,
        other => other.to_string(),
    };
    crate::vm::runtime::error(format!("{record}.{field}: {message}"))
}

/// Register functions under their Rust names:
/// `rynd::register!(engine, double, clamp)` is
/// `engine.register("double", double); engine.register("clamp", clamp);`.
/// Works with a [`RyndEngine`] or a [`Library`].
///
/// ```
/// fn double(x: i64) -> i64 { x * 2 }
/// fn shout(s: String) -> String { s.to_uppercase() }
///
/// let mut engine = rynd::RyndEngine::new();
/// rynd::register!(engine, double, shout);
/// assert_eq!(engine.eval("shout('hi') + to_string(double(2))").unwrap().to_string(), "HI4");
/// ```
#[macro_export]
macro_rules! register {
    ($target:expr, $($function:path),+ $(,)?) => {
        $( $target.register(stringify!($function).rsplit("::").next().unwrap().trim(), $function); )+
    };
}

/// Define a struct that crosses the script boundary as a map with one entry
/// per field. Generates the struct plus [`FromValue`], [`IntoValue`], and
/// [`Context`] (each field becomes a global). A missing field reads as `nil`,
/// so `Option` fields are optional.
/// Field types need `FromValue + IntoValue + Clone`, so records nest.
///
/// ```
/// rynd::record! {
///     #[derive(Debug, Clone, PartialEq)]
///     pub struct Order { pub id: i64, pub total: f64, pub tags: Vec<String> }
/// }
///
/// let mut engine = rynd::RyndEngine::new();
/// engine.register("discount", |mut order: Order| { order.total *= 0.9; order });
/// let out = engine.eval("discount({'id': 7, 'total': 100, 'tags': ['vip']})").unwrap();
/// let order: Order = rynd::FromValue::from_value(&out).unwrap();
/// assert_eq!(order, Order { id: 7, total: 90.0, tags: vec!["vip".into()] });
///
/// // As a context, fields are script globals.
/// use rynd::Context;
/// order.install(&mut engine).unwrap();
/// assert_eq!(engine.eval("id + len(tags)").unwrap(), rynd::Value::Int(8));
/// ```
#[macro_export]
macro_rules! record {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $( $(#[$field_meta:meta])* $field_vis:vis $field:ident : $ty:ty ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis struct $name {
            $( $(#[$field_meta])* $field_vis $field: $ty ),*
        }

        impl $crate::FromValue for $name {
            fn from_value(value: &$crate::Value) -> $crate::RyndResult<Self> {
                let $crate::Value::Map(map) = value else {
                    return Err($crate::vm::runtime::error(format!(
                        "Expected {} map, got {}",
                        stringify!($name),
                        value.type_name()
                    )));
                };
                Ok(Self {
                    $( $field: {
                        let field = map.get(stringify!($field)).unwrap_or(&$crate::Value::Nil);
                        <$ty as $crate::FromValue>::from_value(field).map_err(|e| {
                            $crate::host::field_error(stringify!($name), stringify!($field), e)
                        })?
                    }, )*
                })
            }
        }

        impl $crate::IntoValue for $name {
            fn into_value(self) -> $crate::RyndResult<$crate::Value> {
                let mut map = ::std::collections::BTreeMap::new();
                $( map.insert(
                    stringify!($field).to_string(),
                    $crate::IntoValue::into_value(self.$field)?,
                ); )*
                Ok($crate::Value::map(map))
            }
        }

        impl $crate::Context for $name {
            fn install(&self, engine: &mut $crate::RyndEngine) -> $crate::RyndResult<()> {
                $( engine.set_global(
                    stringify!($field),
                    $crate::IntoValue::into_value(::std::clone::Clone::clone(&self.$field))?,
                ); )*
                Ok(())
            }
        }
    };
}

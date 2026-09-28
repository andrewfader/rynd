//! Typed host boundary: `Val<T>` wrappers and the `Registerable` trait.
//.
//! These types let embedders register and consume Rynd values with
//! statically-checked types instead of hand-written `args[0]` extraction.
//!
//! ```ignore
//! use rynd::{Registerable, Val, RyndEngine};
//! let mut engine = RyndEngine::new();
//! Registerable::register(&mut engine, "double", |x: i64| -> i64 { x * 2 });
//! ```
use crate::error::RyndError;
use crate::vm::value::Value;
use std::convert::TryFrom;
use std::ops::Deref;

/// A typed wrapper around a Rust value crossing the Rynd boundary.
///
/// `Val<T>` exists so embedders can name the type they are exchanging with
/// the script. It mirrors Roto's `Val<T>` and integrates with the existing
/// `From`/`TryFrom` conversions on [`Value`].
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

impl<T> Val<T>
where
    T: Into<Value> + Clone,
{
    /// Convert this typed wrapper into the dynamic [`Value`] representation
    /// that scripts receive.
    pub fn to_value(&self) -> Value {
        self.inner.clone().into()
    }
}

impl<T> TryFrom<&Value> for Val<T>
where
    for<'a> T: TryFrom<&'a Value, Error = RyndError>,
{
    type Error = RyndError;
    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        T::try_from(value).map(Self::new)
    }
}

/// Items that can be registered into a [`crate::RyndEngine`].
///
/// One blanket impl is generated per concrete argument-type list so embedders
/// can write `Registerable::register(&mut engine, "name", |x: i64| -> i64 { ... })`
/// without hand-counting arity or hand-writing `args[N]` extraction.
pub trait Registerable {
    fn register(engine: &mut crate::RyndEngine, name: &str, func: Self);
}

// One impl per concrete function-pointer type. Function pointers are
// uniquely identified by their full type signature, so distinct
// shapes never overlap. To add a new combination, append a line.
impl Registerable for fn(i64) -> i64 {
    fn register(engine: &mut crate::RyndEngine, name: &str, func: Self) {
        engine.register_closure(name, 1, move |args| {
            let arg = i64::try_from(&args[0])?;
            let result = func(arg);
            Ok(result.into())
        });
    }
}
impl Registerable for fn(f64) -> f64 {
    fn register(engine: &mut crate::RyndEngine, name: &str, func: Self) {
        engine.register_closure(name, 1, move |args| {
            let arg = f64::try_from(&args[0])?;
            let result = func(arg);
            Ok(result.into())
        });
    }
}
impl Registerable for fn(bool) -> bool {
    fn register(engine: &mut crate::RyndEngine, name: &str, func: Self) {
        engine.register_closure(name, 1, move |args| {
            let arg = bool::try_from(&args[0])?;
            let result = func(arg);
            Ok(result.into())
        });
    }
}
impl Registerable for fn(String) -> String {
    fn register(engine: &mut crate::RyndEngine, name: &str, func: Self) {
        engine.register_closure(name, 1, move |args| {
            let arg = String::try_from(&args[0])?;
            let result = func(arg);
            Ok(result.into())
        });
    }
}
impl Registerable for fn(i64) -> String {
    fn register(engine: &mut crate::RyndEngine, name: &str, func: Self) {
        engine.register_closure(name, 1, move |args| {
            let arg = i64::try_from(&args[0])?;
            let result = func(arg);
            Ok(result.into())
        });
    }
}
impl Registerable for fn(i64, i64) -> i64 {
    fn register(engine: &mut crate::RyndEngine, name: &str, func: Self) {
        engine.register_closure(name, 2, move |args| {
            let x = i64::try_from(&args[0])?;
            let y = i64::try_from(&args[1])?;
            let result = func(x, y);
            Ok(result.into())
        });
    }
}
impl Registerable for fn(i64, i64) -> String {
    fn register(engine: &mut crate::RyndEngine, name: &str, func: Self) {
        engine.register_closure(name, 2, move |args| {
            let x = i64::try_from(&args[0])?;
            let y = i64::try_from(&args[1])?;
            let result = func(x, y);
            Ok(result.into())
        });
    }
}
impl Registerable for fn(String, String) -> String {
    fn register(engine: &mut crate::RyndEngine, name: &str, func: Self) {
        engine.register_closure(name, 2, move |args| {
            let x = String::try_from(&args[0])?;
            let y = String::try_from(&args[1])?;
            let result = func(x, y);
            Ok(result.into())
        });
    }
}
//! Stage 2: typed host boundary via Val<T> and the Registerable trait.

use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn val_round_trip_for_primitives() {
    let v: rynd::Val<i64> = rynd::Val::new(42);
    let converted = v.to_value();
    assert_eq!(converted.to_string(), "42");
    let back: rynd::Val<i64> = rynd::Val::try_from(&converted).unwrap();
    assert_eq!(back.into_inner(), 42);
}

#[test]
fn val_for_string_round_trip() {
    let v: rynd::Val<String> = rynd::Val::new("hello".to_string());
    let converted = v.to_value();
    assert_eq!(converted.to_string(), "hello");
    let back: rynd::Val<String> = rynd::Val::try_from(&converted).unwrap();
    assert_eq!(back.into_inner(), "hello");
}

#[test]
fn val_for_f64_preserves_bits() {
    // Use an arbitrary value that is not a recognizable mathematical constant,
    // so clippy's approx_constant lint stays quiet.
    let v: rynd::Val<f64> = rynd::Val::new(1.234567);
    let converted = v.to_value();
    let back: rynd::Val<f64> = rynd::Val::try_from(&converted).unwrap();
    assert_eq!(back.into_inner(), 1.234567);
}

#[test]
fn val_try_from_wrong_type_returns_error() {
    // Val<bool>::try_from(&Value::Int(1)) must fail rather than silently coerce.
    let int_value = rynd::Val::<i64>::new(7).to_value();
    let result = rynd::Val::<bool>::try_from(&int_value);
    assert!(result.is_err());
}

// Registerable impls target concrete function pointers (not closures) so
// each type combination has a unique signature and Rust's coherence rules
// don't reject the impls as overlapping.

fn double(x: i64) -> i64 {
    x * 2
}

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn shout(s: String) -> String {
    format!("{}!", s)
}

#[test]
fn registerable_unary_fn_pointer() {
    let mut engine = rynd::RyndEngine::new();
    rynd::Registerable::register(&mut engine, "double", double as fn(i64) -> i64);
    let result = engine.eval("double(21)").unwrap();
    assert_eq!(result.to_string(), "42");
}

#[test]
fn registerable_binary_fn_pointer() {
    let mut engine = rynd::RyndEngine::new();
    rynd::Registerable::register(&mut engine, "add", add as fn(i64, i64) -> i64);
    let result = engine.eval("add(17, 25)").unwrap();
    assert_eq!(result.to_string(), "42");
}

#[test]
fn registerable_string_fn_pointer() {
    let mut engine = rynd::RyndEngine::new();
    rynd::Registerable::register(&mut engine, "shout", shout as fn(String) -> String);
    let result = engine.eval("shout(\"hi\")").unwrap();
    assert_eq!(result.to_string(), "hi!");
}

#[test]
fn register_typed_fn1_preserves_closure_state() {
    // Closures with captured state use the typed helper instead of the
    // Registerable trait, because closures have anonymous types.
    let mut engine = rynd::RyndEngine::new();
    let state = Rc::new(RefCell::new(0_i64));
    let state_for_closure = state.clone();
    engine.register_typed_fn1("bump", move |n: i64| -> i64 {
        let mut s = state_for_closure.borrow_mut();
        *s += n;
        *s
    });
    engine.eval("bump(10)").unwrap();
    engine.eval("bump(32)").unwrap();
    assert_eq!(*state.borrow(), 42);
}

#[test]
fn register_typed_fn2_preserves_closure_state() {
    let mut engine = rynd::RyndEngine::new();
    engine.register_typed_fn2("mul", |a: i64, b: i64| -> i64 { a * b });
    let result = engine.eval("mul(6, 7)").unwrap();
    assert_eq!(result.to_string(), "42");
}
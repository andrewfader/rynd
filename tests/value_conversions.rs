use rynd::{RyndEngine, Value};
use std::collections::BTreeMap;

#[test]
fn exact_scalar_conversions_preserve_values_and_reject_other_types() {
    for number in [i64::MIN, 0, i64::MAX] {
        let value = Value::from(number);
        assert_eq!(i64::try_from(&value).unwrap(), number);
        assert_eq!(i64::try_from(value).unwrap(), number);
    }
    for number in [-0.0_f64, f64::MIN, f64::INFINITY, f64::NAN] {
        let value = Value::from(number);
        assert_eq!(f64::try_from(&value).unwrap().to_bits(), number.to_bits());
        assert_eq!(f64::try_from(value).unwrap().to_bits(), number.to_bits());
    }
    for boolean in [false, true] {
        let value = Value::from(boolean);
        assert_eq!(bool::try_from(&value).unwrap(), boolean);
        assert_eq!(bool::try_from(value).unwrap(), boolean);
    }
    assert!(i64::try_from(Value::Float(1.0)).is_err());
    assert!(f64::try_from(Value::Int(i64::MAX)).is_err());
    assert!(bool::try_from(Value::Int(1)).is_err());
    assert!(String::try_from(Value::Nil).is_err());
    assert!(<&str>::try_from(&Value::Int(1)).is_err());
}

#[test]
fn strings_and_nested_host_collections_convert_without_source_interpolation() {
    let text = "héllo #{literal}";
    let value = Value::from(text.to_owned());
    let borrowed = <&str>::try_from(&value).unwrap();
    if let Value::String(inner) = &value {
        assert_eq!(borrowed.as_ptr(), inner.as_ptr());
    }
    assert_eq!(String::try_from(&value).unwrap(), text);
    assert_eq!(String::try_from(value.clone()).unwrap(), text);
    assert_eq!(String::try_from(value).unwrap(), text);
    assert_eq!(String::try_from(Value::from(text)).unwrap(), text);
    let mut engine = RyndEngine::new();
    engine.set_global(
        "rows",
        vec![BTreeMap::from([("amount".into(), 42_i64)])].into(),
    );
    assert_eq!(engine.eval("rows[0].amount").unwrap(), Value::Int(42));
}

#[test]
fn typed_adapters_report_errors_and_support_script_recovery() {
    let mut engine = RyndEngine::new();
    engine.register_closure("positive", 1, |args| {
        Ok(Value::from(i64::try_from(&args[0])? > 0))
    });
    assert!(bool::try_from(engine.eval("positive(42)").unwrap()).unwrap());
    let error = engine.eval("positive(true)").unwrap_err().to_string();
    assert!(error.contains("Expected int, got bool"));
    assert_eq!(
        engine
            .eval("match attempt(positive, [true]) { Err(_) => 7, _ => 0 }")
            .unwrap(),
        Value::Int(7)
    );
}

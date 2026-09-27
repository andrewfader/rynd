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

#[test]
fn test_value_type_names() {
    assert_eq!(Value::Nil.type_name(), "nil");
    assert_eq!(Value::Bool(true).type_name(), "bool");
    assert_eq!(Value::Int(42).type_name(), "int");
    assert_eq!(Value::Float(3.75).type_name(), "float");
    assert_eq!(Value::string("hi").type_name(), "string");
    assert_eq!(Value::tuple(vec![]).type_name(), "tuple");
    assert_eq!(Value::list(vec![]).type_name(), "list");
    assert_eq!(Value::map(BTreeMap::new()).type_name(), "map");
    assert_eq!(Value::variant("Custom", vec![]).type_name(), "variant");
    assert_eq!(
        Value::Function {
            name: "f".into(),
            arity: 0,
            chunk_index: 0
        }
        .type_name(),
        "function"
    );
    assert_eq!(
        Value::Closure {
            name: "c".into(),
            arity: 1,
            chunk_index: 0,
            upvalues: vec![].into()
        }
        .type_name(),
        "closure"
    );
    assert_eq!(
        Value::Native {
            name: "n".into(),
            arity: 1,
            func: |_| Ok(Value::Nil)
        }
        .type_name(),
        "native_function"
    );
    assert_eq!(
        Value::Builtin {
            name: "b".into(),
            arity: 1
        }
        .type_name(),
        "native_function"
    );
}

#[test]
fn test_value_truthiness() {
    assert!(!Value::Nil.is_truthy());
    assert!(!Value::Bool(false).is_truthy());
    assert!(Value::Bool(true).is_truthy());
    assert!(!Value::Int(0).is_truthy());
    assert!(Value::Int(1).is_truthy());
    assert!(Value::Int(-1).is_truthy());
    assert!(!Value::Float(0.0).is_truthy());
    assert!(!Value::Float(-0.0).is_truthy());
    assert!(!Value::Float(f64::NAN).is_truthy());
    assert!(Value::Float(0.1).is_truthy());
    assert!(Value::Float(-1.0).is_truthy());
    assert!(!Value::string("").is_truthy());
    assert!(Value::string("hello").is_truthy());
    assert!(!Value::list(vec![]).is_truthy());
    assert!(Value::list(vec![Value::Nil]).is_truthy());
    assert!(!Value::map(BTreeMap::new()).is_truthy());
    assert!(Value::map(BTreeMap::from([("k".into(), Value::Int(1))])).is_truthy());
    assert!(Value::tuple(vec![]).is_truthy());
    assert!(!Value::variant("None", vec![]).is_truthy());
    assert!(!Value::variant("Err", vec![Value::string("fail")]).is_truthy());
    assert!(!Value::variant("Some", vec![Value::Nil]).is_truthy());
    assert!(Value::variant("Some", vec![Value::Int(10)]).is_truthy());
    assert!(Value::variant("CustomTag", vec![]).is_truthy());
}

#[test]
fn test_value_display_and_debug() {
    assert_eq!(format!("{}", Value::Nil), "nil");
    assert_eq!(format!("{}", Value::Bool(true)), "true");
    assert_eq!(format!("{}", Value::Int(42)), "42");
    assert_eq!(format!("{}", Value::Float(1.5)), "1.5");
    assert_eq!(format!("{}", Value::string("text")), "text");
    assert_eq!(
        format!("{}", Value::tuple(vec![Value::Int(1), Value::Int(2)])),
        "(1, 2)"
    );
    assert_eq!(
        format!("{}", Value::list(vec![Value::Int(1), Value::Int(2)])),
        "[1, 2]"
    );
    assert_eq!(
        format!(
            "{}",
            Value::map(BTreeMap::from([("a".into(), Value::Int(1))]))
        ),
        "{\"a\": 1}"
    );
    assert_eq!(format!("{}", Value::variant("None", vec![])), "None");
    assert_eq!(
        format!("{}", Value::variant("Some", vec![Value::Int(1)])),
        "Some(1)"
    );
    assert_eq!(
        format!(
            "{}",
            Value::Function {
                name: "foo".into(),
                arity: 2,
                chunk_index: 0
            }
        ),
        "<fn foo/2>"
    );
    assert_eq!(
        format!(
            "{}",
            Value::Closure {
                name: "bar".into(),
                arity: 1,
                chunk_index: 0,
                upvalues: vec![].into()
            }
        ),
        "<closure bar/1>"
    );
    assert_eq!(
        format!(
            "{}",
            Value::Native {
                name: "native".into(),
                arity: 3,
                func: |_| Ok(Value::Nil)
            }
        ),
        "<native fn native/3>"
    );
    assert_eq!(
        format!(
            "{}",
            Value::Builtin {
                name: "sum".into(),
                arity: 1
            }
        ),
        "<native fn sum/1>"
    );
    assert_eq!(format!("{:?}", Value::Int(42)), "42");
}

#[test]
fn test_value_equality_and_cross_types() {
    assert_eq!(Value::Int(42), Value::Float(42.0));
    assert_eq!(Value::Float(42.0), Value::Int(42));
    assert_ne!(Value::Int(42), Value::Int(43));
    assert_ne!(Value::Float(f64::NAN), Value::Float(f64::NAN));
    assert_eq!(
        Value::variant("Some", vec![Value::Int(1)]),
        Value::variant("Some", vec![Value::Int(1)])
    );
    assert_ne!(
        Value::variant("Some", vec![Value::Int(1)]),
        Value::variant("Some", vec![Value::Int(2)])
    );
    assert_ne!(
        Value::variant("Some", vec![Value::Int(1)]),
        Value::variant("Other", vec![Value::Int(1)])
    );
    assert_ne!(Value::Int(1), Value::string("1"));
    assert_ne!(Value::Nil, Value::Bool(false));
    assert_ne!(Value::list(vec![]), Value::tuple(vec![]));
}

#[test]
fn test_from_collections() {
    let list_val = Value::from(vec![1_i64, 2, 3]);
    assert_eq!(
        list_val,
        Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );

    let map_val = Value::from(BTreeMap::from([("key".to_string(), 100_i64)]));
    assert_eq!(
        map_val,
        Value::map(BTreeMap::from([("key".into(), Value::Int(100))]))
    );

    let str_val = Value::from("static str");
    assert_eq!(str_val, Value::string("static str"));
}

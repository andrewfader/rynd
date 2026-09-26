use rynd::vm::json;
use rynd::{RyndEngine, Value};
use std::{cell::RefCell, rc::Rc};

#[test]
fn json_roundtrips_data_and_preserves_integer_precision() {
    let text = r#" {"z":null,"a":[true,false,-9223372036854775808,9223372036854775807,1.25e+2,"hé\uD83D\uDE80\n\t\r\b\f\/\\\"\u0000"]} "#;
    let value = json::parse(text).unwrap();
    let encoded = json::stringify(&value).unwrap();
    assert!(encoded.starts_with("{\"a\":"));
    assert!(encoded.contains("9223372036854775807"));
    assert_eq!(json::parse(&encoded).unwrap(), value);
    assert_eq!(
        json::parse(r#"{"x":1,"x":2}"#).unwrap().to_string(),
        "{\"x\": 2}"
    );
    for (text, expected) in [
        ("null", Value::Nil),
        ("true", Value::Bool(true)),
        ("false", Value::Bool(false)),
        ("0", Value::Int(0)),
        ("-0", Value::Int(0)),
        ("42", Value::Int(42)),
        ("1E-2", Value::Float(0.01)),
        ("-2e3", Value::Float(-2000.0)),
        ("[]", Value::list(vec![])),
        ("{}", Value::map(Default::default())),
        (r#""\u0041\u00e9\uFFFF""#, Value::string("Aé\u{ffff}")),
    ] {
        assert_eq!(json::parse(text).unwrap(), expected, "{text}");
        assert_eq!(
            json::parse(&json::stringify(&expected).unwrap()).unwrap(),
            expected
        );
    }
    // Every control character must survive serialization, including NUL.
    let controls = Value::string((0u8..32).map(char::from).collect::<String>());
    assert_eq!(
        json::parse(&json::stringify(&controls).unwrap()).unwrap(),
        controls
    );
    for value in [
        0.0,
        -0.0,
        1e100,
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::from_bits(1),
    ] {
        let text = json::stringify(&Value::Float(value)).unwrap();
        let Value::Float(decoded) = json::parse(&text).unwrap() else {
            panic!("Float type lost: {text}");
        };
        assert_eq!(decoded.to_bits(), value.to_bits());
    }
}

#[test]
fn json_rejects_malformed_lossy_and_non_data_values() {
    for text in [
        "",
        " ",
        "nil",
        "NaN",
        "Infinity",
        "+1",
        "01",
        "-01",
        "-",
        "1.",
        "1e",
        "1e+",
        "1e999",
        "9223372036854775808",
        "-9223372036854775809",
        "true false",
        "[1,]",
        "[,1]",
        "[1 2]",
        "[",
        "{",
        "{a:1}",
        "{\"a\" 1}",
        "{\"a\":1,}",
        "{\"a\":}",
        "/*x*/1",
        "[1] trailing",
        "\"unterminated",
        "\"escape\\",
        "\"line\nbreak\"",
        r#""\x""#,
        r#""\u0xx0""#,
        r#""\u0""#,
        r#""\uD800""#,
        r#""\uD800\u0041""#,
        r#""\uDC00""#,
        r#""\uD800x""#,
        r#""\uD800\x0000""#,
    ] {
        let message = json::parse(text).unwrap_err().to_string();
        assert!(
            message.contains("parse_json()") && message.contains("at byte"),
            "{text}: {message}"
        );
    }
    for value in [
        Value::Float(f64::NAN),
        Value::Float(f64::INFINITY),
        Value::tuple(vec![]),
        Value::variant("None", vec![]),
    ] {
        assert!(json::stringify(&value).is_err());
    }
    let valid = format!("{}0{}", "[".repeat(128), "]".repeat(128));
    let nested = json::parse(&valid).unwrap();
    assert_eq!(json::stringify(&nested).unwrap(), valid);
    assert!(json::parse(&format!("[{valid}]")).is_err());
    assert!(json::stringify(&Value::list(vec![nested])).is_err());
}

#[test]
fn collection_results_cover_empty_inputs_ordering_and_immutability() {
    for (source, expected) in [
        (
            r#"let xs = [3,1,2]; [sort(xs), xs]"#,
            "[[1, 2, 3], [3, 1, 2]]",
        ),
        (
            r#"sort([9007199254740993, 9007199254740992.0, -1, 0.5])"#,
            "[-1, 0.5, 9007199254740992, 9007199254740993]",
        ),
        (r#"sort(["z","a","é"])"#, "[a, z, é]"),
        (
            r#"[sort([]), sort([1]), sort_by([], \x -> x)]"#,
            "[[], [1], []]",
        ),
        (r#"[3,1,2,4] |> sort_by(\x -> x % 2)"#, "[2, 4, 3, 1]"),
        (
            r#"[1,2,3] |> group_by(\x -> x % 2 == 0 ? "even" : "odd")"#,
            "{\"even\": [2], \"odd\": [1, 3]}",
        ),
        (r#"group_by([], \x -> "")"#, "{}"),
        (
            r#"let m = {"b":2,"a":1}; [keys(m), values(m), entries(m) |> to_map()]"#,
            "[[a, b], [1, 2], {\"a\": 1, \"b\": 2}]",
        ),
        (
            r#"[take([1,2], 1), skip([1,2],1), take([1],99), skip([1],99), take([1],0), skip([1],0)]"#,
            "[[1], [2], [1], [], [], [1]]",
        ),
        (
            r#"[enumerate(["a","b"]), zip([1,2], [3]), zip([], [])]"#,
            "[[(0, a), (1, b)], [(1, 3)], []]",
        ),
        (
            r#"[any([], \x -> true), all([], \x -> false), find([], \x -> true)]"#,
            "[false, true, None]",
        ),
        (
            r#"[any([0,0], \x -> x), all([1,2], \x -> x), find([0,1], \x -> x == 0), find([1], \x -> false)]"#,
            "[false, true, Some(0), None]",
        ),
        (
            r#"[0,1,2] |> filter_map(\x -> x % 2 == 0 ? Some(x) : None)"#,
            "[0, 2]",
        ),
        (
            r#"[false, nil] |> filter_map(\x -> Some(x))"#,
            "[false, nil]",
        ),
        (r#"[1,2] |> flat_map(\x -> [x,x*10])"#, "[1, 10, 2, 20]"),
        (
            r#"[flat_map([], \x -> []), filter_map([], \x -> None)]"#,
            "[[], []]",
        ),
    ] {
        assert_eq!(
            RyndEngine::new().eval(source).unwrap().to_string(),
            expected,
            "{source}"
        );
    }
    for source in [
        "sort(1)",
        "sort([true])",
        "sort([1,\"a\"])",
        "sort([0.0/0.0])",
        "take([], -1)",
        "skip([], 1.0)",
        "keys([])",
        "values(0)",
        "entries(nil)",
        "zip([],1)",
        "filter_map([1], abs)",
        "flat_map([1], abs)",
        "group_by([1], abs)",
        "sort_by([], 1)",
        "all([], \\x,y -> true)",
        "find([1], \\x -> 1 / 0)",
        "sort_by([1], \\x -> 1/0)",
        "parse_json(1)",
        "attempt(abs, 1)",
        "sort()",
        "to_json(abs)",
    ] {
        assert!(RyndEngine::new().eval(source).is_err(), "{source}");
    }
}

#[test]
fn callbacks_are_evaluated_once_and_searches_short_circuit() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let captured = calls.clone();
    let mut engine = RyndEngine::new();
    engine.register_closure("key", 1, move |args| {
        captured.borrow_mut().push(args[0].clone());
        Ok(args[0].clone())
    });
    engine.eval("sort_by([3,1,2], key)").unwrap();
    assert_eq!(
        *calls.borrow(),
        vec![Value::Int(3), Value::Int(1), Value::Int(2)]
    );
    for (source, expected_calls) in [
        ("any([0,1,2], key)", 2),
        ("all([1,0,2], key)", 2),
        ("find([1,2,3], key)", 1),
    ] {
        calls.borrow_mut().clear();
        engine.eval(source).unwrap();
        assert_eq!(calls.borrow().len(), expected_calls, "{source}");
    }
}

#[test]
fn attempt_restores_nested_frames_operands_and_preserves_effects() {
    let mut engine = RyndEngine::new();
    engine.enable_output_capture();
    let result = engine
        .eval(
            r#"
        fn bad(x) { println(x); 100 + (1 / 0) }
        fn recover(x) {
            let local = 7
            10 + match attempt(bad, [x]) { Err(message) => local, _ => 0 }
        }
        [1,2,3] |> map(recover)
    "#,
        )
        .unwrap();
    assert_eq!(result.to_string(), "[17, 17, 17]");
    assert_eq!(engine.take_output(), "1\n2\n3\n");
    assert_eq!(
        engine.call("recover", &[Value::Int(4)]).unwrap(),
        Value::Int(17)
    );
    assert_eq!(engine.eval("40 + 2").unwrap(), Value::Int(42));
    for source in [
        "attempt(abs, [])",
        "attempt(1, [])",
        "attempt(parse_int, [\"bad\"])",
        "attempt(map, [[1], bad])",
    ] {
        assert!(
            matches!(engine.eval(source).unwrap(), Value::Variant { name, .. } if name == "Err")
        );
    }
    assert_eq!(
        engine.eval("attempt(abs, [-2])").unwrap(),
        Value::variant("Ok", vec![Value::Int(2)])
    );
    engine.register_fn("failure", 0, |_| {
        Err(rynd::vm::runtime::error("host failure"))
    });
    assert!(
        engine
            .eval("attempt(failure, [])")
            .unwrap()
            .to_string()
            .contains("host failure")
    );
    engine.eval("fn recur() { recur() }").unwrap();
    assert!(
        engine
            .eval("attempt(recur, [])")
            .unwrap()
            .to_string()
            .contains("depth limit")
    );
    assert_eq!(engine.eval("2+3").unwrap(), Value::Int(5));
}

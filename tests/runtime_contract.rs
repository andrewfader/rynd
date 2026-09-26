use rynd::vm::runtime::{NativeRuntime, Runtime};
use rynd::{RyndEngine, Value};
use std::rc::Rc;

#[test]
fn builtin_contract_and_error_paths() {
    for (source, expected) in [
        (
            "[sum([1, 2.5, 3]), head([]), tail([]), tail([1,2]), concat([1], [2])]",
            "[6.5, nil, [], [2], [1, 2]]",
        ),
        (
            "[len([1]), len((1,2)), len({}), len(\"hé\"), abs(-2.5)]",
            "[1, 2, 0, 2, 2.5]",
        ),
        (
            "[min(3, 2.0), max(3.0, 2), min(1, 1), max(2, 2)]",
            "[2, 3, 1, 2]",
        ),
        ("[8.0 / 2, 8 / 2.0, 8.5 - 2, 2 * 2.5, 4.0 % 2]", "error"),
        (
            "[8.0 / 2, 8 / 2.0, 8.5 - 2, 2 * 2.5, 2.5 + 0.5]",
            "[4, 4, 6.5, 5, 3]",
        ),
        (
            "[\"x\" * 3, \"x\" * -1, 2 + \"x\", \"x\" + 2]",
            "[xxx, , 2x, x2]",
        ),
        (
            "[Some(1).tag, Some(1).name, Some(1).value, Err(2)?.value, Some(1).missing]",
            "[Some, Some, 1, 2, nil]",
        ),
        (
            "[Some(1) == Some(1), Err(2) == Ok(2), (1,2) == (1,2), {\"x\":1} == {\"x\":1}]",
            "[true, false, true, true]",
        ),
        (
            "[nil ?: 1, None ?: 2, Err(0) ?: 3, Some(0) ?: 4, [] ?: 5, {} ?: 6, () ?: 7]",
            "[1, 2, 3, 4, 5, 6, ()]",
        ),
        ("to_map([(\"x\", 1), [\"y\", 2]])", "{\"x\": 1, \"y\": 2}"),
        (
            "[\"a\" < \"b\", 2.5 < 3, 2 > 1.5, 1.5 >= 1.5, 2 <= 2, 0.0 / 0.0 > 1]",
            "[true, true, true, true, true, false]",
        ),
        (
            "[1e3, 2.5e-2, -9223372036854775808, 9223372036854775807..=9223372036854775807]",
            "[1000, 0.025, -9223372036854775808, [9223372036854775807]]",
        ),
        (
            "let f = \\x -> x; let g = f; [f == g, f == (\\x -> x)]",
            "[true, false]",
        ),
        ("fn nothing() { return; } nothing()", "nil"),
        ("fn make(x) { \\y -> x + y } make(\n1\n)(2)", "3"),
    ] {
        let result = RyndEngine::new().eval(source);
        if expected == "error" {
            assert!(result.is_err(), "{source}");
        } else {
            assert_eq!(result.unwrap().to_string(), expected, "{source}");
        }
    }
    for source in [
        "len(1)",
        "sum(1)",
        "sum([true])",
        "head(1)",
        "tail(nil)",
        "push(1,2)",
        "concat([],1)",
        "abs(true)",
        "min(true,1)",
        "max(1,nil)",
        "to_map([1])",
        "map(1, abs)",
        "filter(1, abs)",
        "reduce(1, 0, abs)",
        "1 % 0",
        "9223372036854775807 * 2",
        "-(-9223372036854775808)",
        "(-9223372036854775808) % -1",
        "true < false",
        "-true",
        "abs()",
        "1()",
        "[1][true]",
        "range(false,1)",
        "-9223372036854775808..=9223372036854775807",
    ] {
        assert!(RyndEngine::new().eval(source).is_err(), "{source}");
    }
}

#[test]
fn host_callbacks_capture_output_and_reset_script_state() {
    let mut engine = RyndEngine::default();
    engine.register_fn("host", 1, |args| Ok(args[0].clone()));
    engine.enable_output_capture();
    engine
        .eval("let x = 1; print(\" a\"); println(host(2))")
        .unwrap();
    assert_eq!(engine.captured_output(), " a2\n");
    engine.reset();
    assert!(engine.eval("x").is_err());
    assert_eq!(engine.eval("host(3)").unwrap(), Value::Int(3));
}

#[test]
fn native_runtime_supports_embedding_and_call_validation() {
    let mut rt = NativeRuntime::default();
    rt.set_global("answer", Value::Int(42));
    assert_eq!(rt.get_global("answer").unwrap(), Value::Int(42));
    assert!(rt.get_global("missing").is_err());
    let native = Value::Native {
        name: "host".into(),
        arity: 1,
        func: |args| Ok(args[0].clone()),
    };
    assert_eq!(
        rt.call(native.clone(), vec![Value::Int(5)]).unwrap(),
        Value::Int(5)
    );
    assert!(rt.call(native, vec![]).is_err());
    let function = Value::Compiled {
        name: "identity".into(),
        arity: 1,
        func: Rc::new(|_, args, _| Ok(args[0].clone())),
    };
    assert_eq!(
        rt.call(function.clone(), vec![Value::Int(3)]).unwrap(),
        Value::Int(3)
    );
    assert_eq!(format!("{function}"), "<closure identity/1>");
    assert!(function == function.clone());
    let map = rt.get_global("map").unwrap();
    assert_eq!(
        rt.call(map, vec![Value::list(vec![Value::Int(2)]), function])
            .unwrap(),
        Value::list(vec![Value::Int(2)])
    );
    let recursive = Value::Compiled {
        name: "recursive".into(),
        arity: 0,
        func: Rc::new(|rt, _, this| rt.call(this.clone(), vec![])),
    };
    assert!(
        rt.call(recursive, vec![])
            .unwrap_err()
            .to_string()
            .contains("depth limit")
    );
    assert!(rt.call(Value::Nil, vec![]).is_err());
    let bytecode = Value::Function {
        name: "test".into(),
        arity: 0,
        chunk_index: 0,
    };
    assert!(rt.call(bytecode, vec![]).is_err());
}

#[test]
fn malformed_sources_return_errors_without_panicking() {
    for source in [
        "let",
        "fn",
        "match",
        "[",
        "(",
        "{",
        "let =",
        "fn f( {",
        "\\x x",
        "|x x",
        "[x for in xs]",
        "Some(",
        "if true",
        "let [a,a] = [1,2]",
        "fn f(x,x) { x }",
        "return 1",
        "let mut x = 1",
        "match 1 { x => }",
        "@",
        "?",
        "\"unterminated",
        "\"escape\\",
        "1e",
        "1e+",
        "9223372036854775808",
        "99999999999999999999999999",
    ] {
        assert!(RyndEngine::new().eval(source).is_err(), "{source}");
        assert!(rynd::transpile(source).is_err(), "{source}");
    }
    assert!(
        rynd::syntax::parser::Parser::new(vec![])
            .parse()
            .unwrap()
            .statements
            .is_empty()
    );
    let tokens = [
        "let", "x", "fn", "(", ")", "{", "}", "[", "]", ",", ":", "=", "if", "else", "match",
        "Some", "for", "in", "1", "+", "-", ";", "\\", "->", "=>",
    ];
    let mut seed = 19u64;
    for _ in 0..1000 {
        let mut source = String::new();
        for _ in 0..12 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            source.push_str(tokens[(seed >> 32) as usize % tokens.len()]);
            source.push(' ');
        }
        let _ = RyndEngine::new().eval(&source);
    }
}

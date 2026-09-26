use rynd::RyndEngine;

fn expect(source: &str, expected: &str) {
    let actual = RyndEngine::new().eval(source).unwrap();
    assert_eq!(actual.to_string(), expected, "{source}");
}

#[test]
fn block_values_preserve_locals_and_operand_stack() {
    expect("fn f() { let x = 10 x + 1 } f()", "11");
    expect("1 + { let x = 10; x + 1 }", "12");
    expect("fn f(a) { let (x, y) = (a, 2); x + y } f(10)", "12");
}

#[test]
fn closures_capture_nested_environments_and_survive_calls() {
    expect(
        r"fn make(x) { \y -> x + y } let add = make(10); add(2)",
        "12",
    );
    expect(
        r"fn make(x) { \y -> \z -> x + y + z } let a = make(1); let b = a(2); b(3)",
        "6",
    );
    expect(
        r"fn f(offset) { [x + offset for x in [1, 2]] } f(10)",
        "[11, 12]",
    );
    expect(
        "fn outer() { fn fact(n) { if n <= 1 { 1 } else { n * fact(n - 1) } } fact(5) } outer()",
        "120",
    );
}

#[test]
fn patterns_check_structure_bind_payloads_and_keep_scope() {
    expect(
        "match Some((1, [2, 3])) { Some((a, [b, c])) => a + b + c, _ => 0 }",
        "6",
    );
    expect("match [1, 2] { [9, 9] => 1, _ => 2 }", "2");
    expect("match (1, 2) { (9, 9) => 1, _ => 2 }", "2");
    expect(
        "let t = 99; let result = match 1 { t if false => 0, t => t }; t",
        "99",
    );
    expect("let Some([x, y]) = Some([2, 3]); x + y", "5");
    assert!(RyndEngine::new().eval("let [x, y] = [1]; x").is_err());
    assert!(RyndEngine::new().eval("let 1 = 2").is_err());
}

#[test]
fn engine_preserves_functions_and_recovers_from_errors() {
    let mut engine = RyndEngine::new();
    engine.eval("fn double(x) { x * 2 }").unwrap();
    assert_eq!(engine.eval("double(21)").unwrap().to_string(), "42");
    assert!(engine.eval("double(1 / 0)").is_err());
    assert_eq!(engine.eval("double(3)").unwrap().to_string(), "6");
    engine
        .eval(r"fn make(x) { \y -> x + y } let add = make(10)")
        .unwrap();
    assert_eq!(engine.eval("add(5)").unwrap().to_string(), "15");
}

#[test]
fn maps_comprehensions_ranges_and_numeric_precision() {
    expect(
        r#"{to_string(x): x * x for x in 1..=4 if x % 2 == 0}"#,
        r#"{"2": 4, "4": 16}"#,
    );
    expect(r#"{"x": 1, "x": 2}["x"]"#, "2");
    expect("{}", "{}");
    expect("[3.14159, 1 == 1.0, 2 > 1.0]", "[3.14159, true, true]");
    expect("match -2 { -2 => 7, _ => 0 }", "7");
}

#[test]
fn integer_errors_are_results_not_panics() {
    for code in [
        "9223372036854775807 + 1",
        "(-9223372036854775807 - 1) / -1",
        "abs(-9223372036854775807 - 1)",
    ] {
        assert!(RyndEngine::new().eval(code).is_err(), "{code}");
    }
}

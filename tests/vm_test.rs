use rynd::{RyndEngine, Value};

#[test]
fn test_vm_basic_arithmetic() {
    let mut engine = RyndEngine::new();
    let res = engine.eval("2 + 3 * 4").expect("Eval failed");
    assert_eq!(res, Value::Int(14));

    let res2 = engine.eval("(2 + 3) * 4").expect("Eval failed");
    assert_eq!(res2, Value::Int(20));
}

#[test]
fn test_vm_functions_and_scope() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let factor = 3
        fn scale(x) {
            x * factor
        }
        scale(7)
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::Int(21));
}

#[test]
fn test_vm_recursive_fibonacci() {
    let mut engine = RyndEngine::new();
    let code = r#"
        fn fib(n) {
            if n <= 1 {
                n
            } else {
                fib(n - 1) + fib(n - 2)
            }
        }
        fib(10)
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::Int(55));
}

#[test]
fn test_vm_pipeline_transformation() {
    let mut engine = RyndEngine::new();
    let code = r#"
        [1, 2, 3, 4, 5]
        |> filter(\x -> x > 2)
        |> map(\x -> x * 10)
        |> sum()
    "#;
    let res = engine.eval(code).expect("Eval failed");
    // [3, 4, 5] * 10 = [30, 40, 50], sum = 120
    assert_eq!(res, Value::Int(120));
}

#[test]
fn test_vm_comprehensions() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let numbers = [1, 2, 3, 4, 5, 6]
        let evens_doubled = [x * 2 for x in numbers if x % 2 == 0]
        sum(evens_doubled)
    "#;
    let res = engine.eval(code).expect("Eval failed");
    // evens: [2, 4, 6], doubled: [4, 8, 12], sum = 24
    assert_eq!(res, Value::Int(24));
}

#[test]
fn test_vm_elvis_and_safe_nav() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let fallback = nil ?: "default_val"
        let active = "active_val" ?: "default_val"
        [fallback, active]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::string("default_val"),
            Value::string("active_val")
        ])
    );
}

#[test]
fn test_vm_match_expression() {
    let mut engine = RyndEngine::new();
    let code = r#"
        fn classify(val) {
            match val {
                0 => "zero",
                1 => "one",
                _ => "many"
            }
        }
        [classify(0), classify(1), classify(42)]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::string("zero"),
            Value::string("one"),
            Value::string("many")
        ])
    );
}

#[test]
fn test_vm_maps_and_indexing() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let user = {"name": "Ada Lovelace", "role": "Pioneer"}
        user["name"] + " - " + user["role"]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::string("Ada Lovelace - Pioneer"));
}

#[test]
fn test_vm_negative_and_out_of_bounds_indexing() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let list = [10, 20, 30]
        let tup = (100, 200)
        let str = "hello"
        let map = {"a": 1}

        [
            list[-1], list[-2], list[99], list[-99],
            tup[-1], tup[99],
            str[-1], str[-2], str[99],
            map["missing"]
        ]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::Int(30),
            Value::Int(20),
            Value::Nil,
            Value::Nil,
            Value::Int(200),
            Value::Nil,
            Value::string("o"),
            Value::string("l"),
            Value::Nil,
            Value::Nil,
        ])
    );

    // Invalid target indexing error
    assert!(engine.eval("42[0]").is_err());
}

#[test]
fn test_vm_string_multiplication_and_concatenation() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let repeated = "abc" * 3
        let zero_rep = "abc" * 0
        let neg_rep = "abc" * -2
        let concat1 = "hello " + 42
        let concat2 = 42 + " items"
        [repeated, zero_rep, neg_rep, concat1, concat2]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::string("abcabcabc"),
            Value::string(""),
            Value::string(""),
            Value::string("hello 42"),
            Value::string("42 items"),
        ])
    );
}

#[test]
fn test_vm_variant_field_access() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let some_val = Some(42)
        let none_val = None
        let tag1 = some_val.tag
        let name1 = some_val.name
        let val1 = some_val.value
        let tag2 = none_val.tag
        let val2 = none_val.value
        let safe_scalar = 42?.missing
        [tag1, name1, val1, tag2, val2, safe_scalar]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::string("Some"),
            Value::string("Some"),
            Value::Int(42),
            Value::string("None"),
            Value::Nil,
            Value::Nil,
        ])
    );

    // Direct field access on scalar is error
    assert!(engine.eval("42.missing").is_err());
}

#[test]
fn test_vm_modulo_and_arithmetic_errors() {
    let mut engine = RyndEngine::new();
    assert_eq!(engine.eval("10 % 3").unwrap(), Value::Int(1));
    assert!(
        engine
            .eval("10 % 0")
            .unwrap_err()
            .to_string()
            .contains("Modulo by zero")
    );
    assert!(
        engine
            .eval("10 / 0")
            .unwrap_err()
            .to_string()
            .contains("Division by zero")
    );
    assert_eq!(engine.eval("-3.75").unwrap(), Value::Float(-3.75));
}

#[test]
fn test_vm_inclusive_and_exclusive_ranges() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let r1 = 1..5
        let r2 = 1..=5
        let r3 = 5..5
        let r4 = 5..=5
        [r1, r2, r3, r4]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::list(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4)
            ]),
            Value::list(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
                Value::Int(5)
            ]),
            Value::list(vec![]),
            Value::list(vec![Value::Int(5)]),
        ])
    );

    // Non-integer range error
    assert!(engine.eval("1.0..5").is_err());
}

#[test]
fn test_vm_runtime_builtins() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let s = "foo:bar:baz"
        let parts = split(s, ":")
        let joined = join(parts, "-")
        let trimmed = trim("  hello  ")
        let parsed_i = parse_int(" 123 ")
        let parsed_f = parse_float(" 3.125 ")
        let h = head([1, 2, 3])
        let t = tail([1, 2, 3])
        let p = push([1, 2], 3)
        let c = concat([1, 2], [3, 4])
        let m = to_map([["a", 10], ["b", 20]])
        let a = abs(-42)
        let mn = min(10, 20)
        let mx = max(10, 20)
        let l_str = len("hello 🦀")
        let l_list = len([1, 2, 3, 4])
        let cont = contains("haystack", "stack") and contains([1, 2, 3], 2)
        [joined, trimmed, parsed_i, parsed_f, h, t, p, c, m["a"], a, mn, mx, l_str, l_list, cont]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::string("foo-bar-baz"),
            Value::string("hello"),
            Value::Int(123),
            Value::Float(3.125),
            Value::Int(1),
            Value::list(vec![Value::Int(2), Value::Int(3)]),
            Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
            Value::list(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4)
            ]),
            Value::Int(10),
            Value::Int(42),
            Value::Int(10),
            Value::Int(20),
            Value::Int(7), // "hello 🦀": 6 ascii + 1 crab emoji = 7 chars
            Value::Int(4),
            Value::Bool(true),
        ])
    );
}

#[test]
fn test_vm_recursive_closure_get_self() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let fact = \n -> if n <= 1 { 1 } else { n * fact(n - 1) }
        fact(5)
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::Int(120));
}

#[test]
fn test_vm_output_capture() {
    let mut engine = RyndEngine::new();
    engine.enable_output_capture();
    engine.eval("print('hello '); println('world')").unwrap();
    assert_eq!(engine.take_output(), "hello world\n");
}

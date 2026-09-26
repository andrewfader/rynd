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

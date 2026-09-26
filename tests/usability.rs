use rynd::{RyndEngine, Value};
use std::{cell::RefCell, rc::Rc};

#[test]
fn embedded_rules_accept_host_data_and_recover_after_failed_calls() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    let mut engine = RyndEngine::new();
    engine.enable_output_capture();
    engine.set_global("limit", Value::Int(20));
    engine.register_closure("record", 1, move |args| {
        recorded.borrow_mut().push(args[0].clone());
        Ok(args[0].clone())
    });
    engine
        .eval("fn accept(n) { record(n >= limit) } fn bad(n) { 1 / n }")
        .unwrap();
    assert_eq!(
        engine.call("accept", &[Value::Int(25)]).unwrap(),
        Value::Bool(true)
    );
    engine.set_global("limit", Value::Int(30));
    assert_eq!(
        engine.call("accept", &[Value::Int(25)]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        *events.borrow(),
        vec![Value::Bool(true), Value::Bool(false)]
    );
    assert!(engine.call("bad", &[Value::Int(0)]).is_err());
    assert!(engine.call("accept", &[]).is_err());
    assert!(engine.call("missing", &[]).is_err());
    assert_eq!(engine.call("bad", &[Value::Int(1)]).unwrap(), Value::Int(1));
    assert_eq!(engine.get_global("limit").unwrap(), Value::Int(30));
    engine.eval("print(\"first\")").unwrap();
    assert_eq!(engine.take_output(), "first");
    assert_eq!(engine.take_output(), "");
    // Registration survives script shadowing and reset, as does capture mode.
    engine.register_fn("identity", 1, |args| Ok(args[0].clone()));
    engine.eval("let record = 1; let identity = 2").unwrap();
    engine.reset();
    assert!(engine.get_global("limit").is_err());
    assert_eq!(
        engine.call("record", &[Value::Int(42)]).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        engine.call("identity", &[Value::Int(3)]).unwrap(),
        Value::Int(3)
    );
    engine.eval("println(\"second\")").unwrap();
    assert_eq!(engine.take_output(), "second\n");
}

#[test]
fn check_does_not_execute_and_reports_compile_errors() {
    assert!(rynd::check("println(1 / 0); read_text(\"missing\")").is_ok());
    assert!(rynd::check("let =").is_err());
    assert!(rynd::check("return 1").is_err());
    assert!(rynd::check("fn f(x, x) { x }").is_err());
}

#[test]
fn text_helpers_reject_bad_input_and_retain_unicode() {
    let mut engine = RyndEngine::new();
    assert_eq!(
        engine
            .eval(r#"" α\r\nβ\n" |> lines() |> map(trim) |> join("|")"#)
            .unwrap(),
        Value::string("α|β")
    );
    assert_eq!(engine.eval(r#"[parse_int(" -9223372036854775808 "), parse_float(" 1.25e2 "), contains("café", "fé"), contains([1], 1.0), contains({"a":1}, "a")]"#).unwrap().to_string(), "[-9223372036854775808, 125, true, true, true]");
    for source in [
        "lines(1)",
        "trim(nil)",
        "split(\"x\", 1)",
        "join([1], \",\")",
        "join(1, \",\")",
        "parse_int(\"9223372036854775808\")",
        "parse_int(\"x\")",
        "parse_float(\"NaN\")",
        "parse_float(\"1e9999\")",
        "parse_float(\"oops\")",
        "contains(1, 2)",
        "contains(\"x\", 1)",
        "read_text(1)",
        "read_stdin(1)",
        "read_text(\"/nonexistent/rynd/input\")",
    ] {
        assert!(engine.eval(source).is_err(), "{source}");
    }
}

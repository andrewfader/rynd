//! Stage 7: filtermap verb + Verdict.

#[test]
fn verdict_accept_and_reject_are_distinct() {
    let accept = rynd::Verdict::Accept;
    let reject = rynd::Verdict::Reject;
    assert_ne!(accept, reject);
    assert_eq!(accept, rynd::Verdict::Accept);
}

#[test]
fn verdict_display_compact() {
    assert_eq!(format!("{}", rynd::Verdict::Accept), "accept");
    assert_eq!(format!("{}", rynd::Verdict::Reject), "reject");
}

#[test]
fn verdict_from_truthy_value_is_accept() {
    let v = rynd::Verdict::from_value(&rynd::Value::Int(1));
    assert_eq!(v, rynd::Verdict::Accept);
    let v = rynd::Verdict::from_value(&rynd::Value::Bool(true));
    assert_eq!(v, rynd::Verdict::Accept);
}

#[test]
fn verdict_from_falsy_value_is_reject() {
    let v = rynd::Verdict::from_value(&rynd::Value::Int(0));
    assert_eq!(v, rynd::Verdict::Reject);
    let v = rynd::Verdict::from_value(&rynd::Value::Nil);
    assert_eq!(v, rynd::Verdict::Reject);
    let v = rynd::Verdict::from_value(&rynd::Value::Bool(false));
    assert_eq!(v, rynd::Verdict::Reject);
}

#[test]
fn filter_builtin_keeps_truthy_list_items() {
    let mut engine = rynd::RyndEngine::new();
    let src = "filter([1, 0, 2, 0, 3], |n| { n != 0 })";
    let result = engine.eval(src).unwrap();
    assert_eq!(result.to_string(), "[1, 2, 3]");
}
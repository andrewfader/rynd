//! Filter-map verdicts: scripts accept or reject input and may transform it.
use rynd::{RyndEngine, Value, Verdict};

const ROUTES: &str = r##"
fn route(prefix, len) {
    if prefix == "0.0.0.0" { return Reject("default route") }
    if len > 24 { Reject("too specific") } else { Accept("#{prefix}/#{len}") }
}
"##;

#[test]
fn filtermap_splits_verdict_and_payload() {
    let mut engine = RyndEngine::new();
    engine.eval(ROUTES).unwrap();
    let call = |engine: &mut RyndEngine, prefix: &str, len: i64| {
        engine
            .filtermap("route", &[Value::from(prefix), Value::Int(len)])
            .unwrap()
    };
    assert_eq!(
        call(&mut engine, "10.0.0.0", 8),
        (Verdict::Accept, Value::from("10.0.0.0/8"))
    );
    assert_eq!(
        call(&mut engine, "0.0.0.0", 0),
        (Verdict::Reject, Value::from("default route"))
    );
    assert_eq!(
        call(&mut engine, "10.1.2.0", 28),
        (Verdict::Reject, Value::from("too specific"))
    );
}

#[test]
fn plain_results_are_judged_by_truthiness() {
    let mut engine = RyndEngine::new();
    engine
        .eval("fn positive(x) { x > 0 }\nfn echo(x) { x }")
        .unwrap();
    assert_eq!(
        engine.filtermap("positive", &[Value::Int(3)]).unwrap(),
        (Verdict::Accept, Value::Bool(true))
    );
    assert_eq!(
        engine.filtermap("echo", &[Value::Nil]).unwrap(),
        (Verdict::Reject, Value::Nil)
    );
    assert!(engine.filtermap("missing", &[]).is_err());
    for (value, verdict) in [
        (Value::Int(1), Verdict::Accept),
        (Value::Bool(true), Verdict::Accept),
        (Value::Int(0), Verdict::Reject),
        (Value::Nil, Verdict::Reject),
        (Value::Bool(false), Verdict::Reject),
    ] {
        assert_eq!(Verdict::from_value(&value), verdict);
    }
}

#[test]
fn verdicts_are_ordinary_variants_in_scripts() {
    let mut engine = RyndEngine::new();
    engine.eval(ROUTES).unwrap();
    let kept = engine
        .eval(r#"[("10.0.0.0", 8), ("0.0.0.0", 0), ("192.168.1.0", 24), ("1.2.3.4", 32)] |> filter_map(\(p, l) -> route(p, l))"#)
        .unwrap();
    assert_eq!(kept.to_string(), "[10.0.0.0/8, 192.168.1.0/24]");
    // Reject is falsey, so verdict functions double as predicates.
    assert_eq!(
        engine
            .eval("[8, 30] |> filter(\\l -> route(\"10.0.0.0\", l))")
            .unwrap()
            .to_string(),
        "[8]"
    );
    let described = engine
        .eval(
            r#"match route("0.0.0.0", 0) { Accept(p) => "ok #{p}", Reject(why) => "no: #{why}" }"#,
        )
        .unwrap();
    assert_eq!(described, Value::from("no: default route"));
    assert_eq!(
        Verdict::Accept.to_value(Value::Int(1)),
        engine.eval("Accept(1)").unwrap()
    );
    assert_eq!(format!("{}", Verdict::Reject), "Reject");
}

#[test]
fn filter_map_rejects_non_verdict_callbacks() {
    let mut engine = RyndEngine::new();
    let error = engine
        .eval("filter_map([1], \\x -> x)")
        .unwrap_err()
        .to_string();
    assert!(error.contains("Accept(value), or Reject(value)"), "{error}");
}

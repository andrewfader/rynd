//! Typed host boundary: registration, conversions, records, contexts, libraries.
use rynd::{Context, FromValue, IntoValue, Library, RyndEngine, RyndResult, Val, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

fn double(x: i64) -> i64 {
    x * 2
}

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn shout(s: String) -> String {
    format!("{s}!")
}

#[test]
fn functions_and_closures_of_every_arity_register_directly() {
    let mut engine = RyndEngine::new();
    engine.register("answer", || 42_i64);
    engine.register("double", double);
    engine.register("add", add);
    engine.register(
        "sum8",
        |a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64| {
            a + b + c + d + e + f + g + h
        },
    );
    assert_eq!(engine.eval("answer()").unwrap(), Value::Int(42));
    assert_eq!(engine.eval("double(21)").unwrap(), Value::Int(42));
    assert_eq!(engine.eval("add(17, 25)").unwrap(), Value::Int(42));
    assert_eq!(
        engine.eval("sum8(1, 2, 3, 4, 5, 6, 7, 8)").unwrap(),
        Value::Int(36)
    );
    let error = engine.eval("add(1)").unwrap_err().to_string();
    assert!(error.contains("Expected 2 arguments, found 1"), "{error}");
}

#[test]
fn registerable_trait_accepts_fn_items_pointers_and_closures() {
    let mut engine = RyndEngine::new();
    rynd::Registerable::register(&mut engine, "double", double as fn(i64) -> i64);
    rynd::Registerable::register(&mut engine, "shout", shout);
    rynd::Registerable::register(&mut engine, "triple", |x: i64| x * 3);
    assert_eq!(engine.eval("double(21)").unwrap(), Value::Int(42));
    assert_eq!(engine.eval("shout('hi')").unwrap(), Value::from("hi!"));
    assert_eq!(engine.eval("triple(14)").unwrap(), Value::Int(42));
}

#[test]
fn closures_keep_host_state_and_typed_helpers_delegate() {
    let mut engine = RyndEngine::new();
    let total = Rc::new(RefCell::new(0_i64));
    let sink = total.clone();
    engine.register_typed_fn1("bump", move |n: i64| {
        *sink.borrow_mut() += n;
        *sink.borrow()
    });
    engine.register_typed_fn2("mul", |a: i64, b: i64| a * b);
    engine.eval("bump(10)").unwrap();
    engine.eval("bump(mul(4, 8))").unwrap();
    assert_eq!(*total.borrow(), 42);
}

#[test]
fn argument_errors_name_the_expected_type_and_host_errors_propagate() {
    let mut engine = RyndEngine::new();
    engine.register("double", double);
    engine.register("port", |text: String| -> RyndResult<u16> {
        text.parse()
            .map_err(|_| rynd::vm::runtime::error(format!("bad port '{text}'")))
    });
    let error = engine.eval("double('x')").unwrap_err().to_string();
    assert!(error.contains("Expected int, got string"), "{error}");
    assert_eq!(engine.eval("port('443')").unwrap(), Value::Int(443));
    let error = engine.eval("port('https')").unwrap_err().to_string();
    assert!(error.contains("bad port 'https'"), "{error}");
    // Script recovery works on host errors.
    assert_eq!(
        engine.eval("attempt(port, ['x'])").unwrap().to_string(),
        "Err(Runtime error: bad port 'x')"
    );
}

#[test]
fn conversions_cover_collections_options_tuples_and_integer_ranges() {
    let mut engine = RyndEngine::new();
    engine.register("evens", |xs: Vec<i64>| {
        xs.into_iter().filter(|x| x % 2 == 0).collect::<Vec<_>>()
    });
    engine.register("lookup", |m: BTreeMap<String, i64>, k: String| {
        m.get(&k).copied()
    });
    engine.register("swap", |pair: (String, i64)| (pair.1, pair.0));
    engine.register("or_zero", |x: Option<i64>| x.unwrap_or(0));
    engine.register("byte", |x: u8| x);
    engine.register("half", |x: f64| x / 2.0);
    engine.register("count", |m: HashMap<String, Value>| m.len());
    engine.register("log", |_: String| {});

    assert_eq!(
        engine.eval("evens([1, 2, 3, 4])").unwrap().to_string(),
        "[2, 4]"
    );
    assert_eq!(
        engine.eval("lookup({'a': 1}, 'a')").unwrap().to_string(),
        "Some(1)"
    );
    assert_eq!(
        engine.eval("lookup({'a': 1}, 'b')").unwrap().to_string(),
        "None"
    );
    assert_eq!(engine.eval("swap(('x', 1))").unwrap().to_string(), "(1, x)");
    assert_eq!(engine.eval("or_zero(nil)").unwrap(), Value::Int(0));
    assert_eq!(engine.eval("or_zero(None)").unwrap(), Value::Int(0));
    assert_eq!(engine.eval("or_zero(Some(5))").unwrap(), Value::Int(5));
    assert_eq!(engine.eval("byte(255)").unwrap(), Value::Int(255));
    assert!(
        engine
            .eval("byte(256)")
            .unwrap_err()
            .to_string()
            .contains("out of range for u8")
    );
    // Ints widen to floats at the host boundary.
    assert_eq!(engine.eval("half(3)").unwrap(), Value::Float(1.5));
    assert_eq!(
        engine.eval("count({'a': 1, 'b': 2})").unwrap(),
        Value::Int(2)
    );
    assert_eq!(engine.eval("log('x')").unwrap(), Value::Nil);

    assert!(u64::MAX.into_value().is_err());
    assert_eq!(usize::from_value(&Value::Int(3)).unwrap(), 3);
    assert!(usize::from_value(&Value::Int(-1)).is_err());
}

#[test]
fn val_wraps_typed_values_both_ways() {
    let v = Val::new(42_i64);
    let value = v.to_value().unwrap();
    assert_eq!(value, Value::Int(42));
    let back = Val::<i64>::try_from(&value).unwrap();
    assert_eq!(*back, 42);
    assert!(Val::<bool>::try_from(&value).is_err());
    let text = Val::new("hello".to_string());
    assert_eq!(text.to_value().unwrap().to_string(), "hello");
    assert_eq!(
        Val::<f64>::try_from(&Value::Float(1.25))
            .unwrap()
            .into_inner(),
        1.25
    );

    let mut engine = RyndEngine::new();
    engine.register("inc", |x: Val<i64>| Val::new(*x + 1));
    assert_eq!(engine.eval("inc(1)").unwrap(), Value::Int(2));
}

rynd::record! {
    #[derive(Debug, Clone, PartialEq)]
    pub struct Line {
        pub sku: String,
        pub qty: i64,
    }
}

rynd::record! {
    /// Nested records, optional fields, and float totals.
    #[derive(Debug, Clone, PartialEq)]
    struct Order {
        id: i64,
        lines: Vec<Line>,
        note: Option<String>,
        total: f64,
    }
}

#[test]
fn records_round_trip_through_scripts() {
    let mut engine = RyndEngine::new();
    engine.register("fulfil", |mut order: Order| {
        order.total *= 0.5;
        order.note = Some(format!("{} lines", order.lines.len()));
        order
    });
    let out = engine
        .eval("fulfil({'id': 7, 'lines': [{'sku': 'a', 'qty': 2}], 'total': 10})")
        .unwrap();
    assert_eq!(
        out.to_string(),
        "{\"id\": 7, \"lines\": [{\"qty\": 2, \"sku\": a}], \"note\": Some(1 lines), \"total\": 5}"
    );
    let order = Order::from_value(&out).unwrap();
    assert_eq!(
        order,
        Order {
            id: 7,
            lines: vec![Line {
                sku: "a".into(),
                qty: 2
            }],
            note: Some("1 lines".into()),
            total: 5.0,
        }
    );
    let error = engine
        .eval("fulfil({'id': 7, 'lines': [{'sku': 1, 'qty': 2}], 'total': 1})")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Order.lines: Line.sku: Expected string"),
        "{error}"
    );
    let error = Order::from_value(&Value::Int(1)).unwrap_err().to_string();
    assert!(error.contains("Expected Order map, got int"), "{error}");
}

#[test]
fn records_install_as_context_globals() {
    let line = Line {
        sku: "widget".into(),
        qty: 3,
    };
    let mut engine = RyndEngine::new();
    engine.install(&line).unwrap();
    assert_eq!(
        engine.eval("upper(sku) + ' x' + to_string(qty)").unwrap(),
        Value::from("WIDGET x3")
    );
    // Re-installing replaces the previous context.
    Line {
        sku: "gear".into(),
        qty: 1,
    }
    .install(&mut engine)
    .unwrap();
    assert_eq!(engine.eval("sku").unwrap(), Value::from("gear"));
}

mod pricing {
    pub fn cents(dollars: i64) -> i64 {
        dollars * 100
    }
}

#[test]
fn register_macro_uses_rust_names_for_engines_and_libraries() {
    let mut engine = RyndEngine::new();
    rynd::register!(engine, double, add, pricing::cents);
    assert_eq!(
        engine.eval("add(double(1), cents(1))").unwrap(),
        Value::Int(102)
    );

    let mut lib = Library::new();
    rynd::register!(lib, shout);
    assert_eq!(lib.names(), vec!["shout"]);
}

#[test]
fn libraries_install_functions_and_constants_and_survive_reset() {
    let mut lib = Library::new();
    lib.register("greet", |s: String| format!("hello {s}"));
    lib.constant("GREETING_LIMIT", 3_i64).unwrap();
    let mut extra = Library::new();
    extra.register("GREETING_LIMIT", || 5_i64);
    extra.constant("VERSION", "1.0").unwrap();
    assert_eq!(lib.names(), vec!["GREETING_LIMIT", "greet"]);
    assert!(lib.contains("greet"));

    let mut engine = RyndEngine::with_library(&lib);
    assert_eq!(
        engine.eval("greet('Ada')").unwrap(),
        Value::from("hello Ada")
    );
    assert_eq!(engine.eval("GREETING_LIMIT").unwrap(), Value::Int(3));
    engine.reset();
    // Functions are host callbacks and survive reset; constants are script globals.
    assert_eq!(engine.eval("greet('Bo')").unwrap(), Value::from("hello Bo"));
    assert!(engine.eval("GREETING_LIMIT").is_err());

    lib.extend(&extra);
    assert_eq!(lib.names(), vec!["GREETING_LIMIT", "VERSION", "greet"]);
    let mut engine = RyndEngine::with_library(&lib);
    assert_eq!(engine.eval("GREETING_LIMIT()").unwrap(), Value::Int(5));
    assert_eq!(engine.eval("VERSION").unwrap(), Value::from("1.0"));
}

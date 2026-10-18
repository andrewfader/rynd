use rynd::{Limits, RyndEngine, RyndError, Value};
use std::time::{Duration, Instant};

fn limited(limits: Limits) -> RyndEngine {
    let mut engine = RyndEngine::new();
    engine.set_limits(limits);
    engine
}

fn limit_message(result: Result<Value, RyndError>) -> String {
    let error = result.expect_err("expected the run to hit a limit");
    assert!(error.is_limit(), "not a limit error: {error}");
    error.to_string()
}

/// Run `f` on a thread with a small (2 MiB) stack, as an embedding host might.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn runaway_tail_recursion_hits_the_operation_limit() {
    let mut engine = limited(Limits {
        operations: Some(100_000),
        ..Limits::none()
    });
    let message = limit_message(engine.eval("fn spin(n) { spin(n + 1) } spin(0)"));
    assert!(message.contains("operations"), "{message}");
}

#[test]
fn time_limit_stops_a_loop_without_an_operation_limit() {
    let mut engine = limited(Limits {
        time: Some(Duration::from_millis(50)),
        ..Limits::none()
    });
    let start = Instant::now();
    let message = limit_message(engine.eval("fn spin(n) { spin(n + 1) } spin(0)"));
    assert!(message.contains("time"), "{message}");
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn builtin_callbacks_count_toward_the_budget() {
    let mut engine = limited(Limits {
        operations: Some(50_000),
        ..Limits::none()
    });
    limit_message(engine.eval(r"range(0, 100000) |> map(\x -> x + 1) |> len()"));
}

#[test]
fn doubling_a_string_stops_at_the_string_limit() {
    let mut engine = limited(Limits {
        string_bytes: Some(1 << 20),
        ..Limits::none()
    });
    let message = limit_message(engine.eval(
        r#"fn grow(s, n) { if n == 0 { len(s) } else { grow(s + s, n - 1) } } grow("x", 64)"#,
    ));
    assert!(message.contains("string length"), "{message}");
    limit_message(engine.eval(r#""ab" * 10000000"#));
}

#[test]
fn huge_ranges_and_list_growth_stop_before_allocating() {
    let mut engine = limited(Limits {
        collection_len: Some(10_000),
        ..Limits::none()
    });
    let message = limit_message(engine.eval("range(0, 1000000000000)"));
    assert!(message.contains("collection length"), "{message}");
    limit_message(engine.eval(
        "fn grow(l, n) { if n == 0 { len(l) } else { grow(concat(l, l), n - 1) } } grow([1], 64)",
    ));
}

#[test]
fn shared_structure_cannot_expand_without_bound() {
    // `many` is small in memory (1000 references to one list) but expands
    // to a billion elements when flattened, printed, or encoded.
    let setup = r"let big = range(0, 1000000)
        let many = range(0, 1000) |> map(\_ -> big)";
    let mut engine = limited(Limits::sandbox());
    for expr in [
        "flatten(many)",
        "to_string(many)",
        "to_json(many)",
        "len(\"\" + many)",
    ] {
        let start = Instant::now();
        limit_message(engine.eval(&format!("{setup}\n{expr}")));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{expr} took too long"
        );
    }
    limit_message(engine.eval("windows(range(0, 1000000), 500000)"));
}

#[test]
fn quadratic_builtins_are_charged_by_their_work() {
    let mut engine = limited(Limits {
        operations: Some(1_000_000),
        ..Limits::none()
    });
    limit_message(engine.eval("uniq(range(0, 100000))"));
    limit_message(engine.eval(
        "fn build(l, n) { if n == 0 { len(l) } else { build(push(l, n), n - 1) } } build([], 100000)",
    ));
}

#[test]
fn allocation_limit_counts_everything_a_run_creates() {
    let mut engine = limited(Limits {
        allocation: Some(1 << 20),
        ..Limits::none()
    });
    let message = limit_message(
        engine.eval(r#"range(0, 10000) |> map(\i -> "item " + to_string(i) + "!" * 200) |> len()"#),
    );
    assert!(message.contains("allocation"), "{message}");
}

#[test]
fn scripts_cannot_catch_limit_errors() {
    let mut engine = limited(Limits {
        operations: Some(10_000),
        ..Limits::none()
    });
    let message = limit_message(engine.eval(
        r"fn spin(n) { spin(n + 1) }
          let caught = attempt(\ -> spin(0), [])
          fn again(r) { again(r) }
          again(caught)",
    ));
    assert!(message.contains("operations"), "{message}");
}

#[test]
fn every_run_gets_a_fresh_budget_and_the_engine_stays_usable() {
    let mut engine = limited(Limits {
        operations: Some(20_000),
        ..Limits::none()
    });
    limit_message(engine.eval("fn spin(n) { spin(n + 1) } spin(0)"));
    for _ in 0..5 {
        assert_eq!(
            engine
                .eval("fn count(n) { if n == 0 { 0 } else { 1 + count(n - 1) } } count(100)")
                .unwrap(),
            Value::Int(100)
        );
    }
    engine.eval("fn twice(x) { x * 2 }").unwrap();
    assert_eq!(
        engine.call("twice", &[Value::Int(21)]).unwrap(),
        Value::Int(42)
    );
    engine.set_limits(Limits::none());
    assert_eq!(
        engine
            .eval(
                "fn sum(n, acc) { if n == 0 { acc } else { sum(n - 1, acc + n) } } sum(100000, 0)"
            )
            .unwrap(),
        Value::Int(5000050000)
    );
}

#[test]
fn host_calls_into_script_functions_are_limited_too() {
    let mut engine = limited(Limits {
        operations: Some(10_000),
        ..Limits::none()
    });
    engine.eval("fn spin(n) { spin(n + 1) }").unwrap();
    assert!(
        engine
            .call("spin", &[Value::Int(0)])
            .unwrap_err()
            .is_limit()
    );
}

#[test]
fn another_thread_can_interrupt_a_run() {
    let mut engine = RyndEngine::new();
    let interrupt = engine.interrupt_handle();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        interrupt.interrupt();
    });
    let message = limit_message(engine.eval("fn spin(n) { spin(n + 1) } spin(0)"));
    assert!(message.contains("interrupted"), "{message}");
    stopper.join().unwrap();
    engine.interrupt_handle().reset();
    assert_eq!(engine.eval("1 + 1").unwrap(), Value::Int(2));
}

#[test]
fn call_depth_and_source_size_limits() {
    let mut engine = limited(Limits {
        call_depth: Some(50),
        source_bytes: Some(200),
        ..Limits::none()
    });
    let error = engine
        .eval("fn deep(n) { if n == 0 { 0 } else { 1 + deep(n - 1) } } deep(100)")
        .unwrap_err();
    assert!(
        error.to_string().contains("Call depth limit exceeded (50)"),
        "{error}"
    );
    assert_eq!(
        engine
            .eval("fn deep(n) { if n == 0 { 0 } else { 1 + deep(n - 1) } } deep(40)")
            .unwrap(),
        Value::Int(40)
    );
    let error = engine
        .eval(&format!("1 + {}", "1 + ".repeat(100) + "1"))
        .unwrap_err();
    assert!(
        error.is_limit() && error.to_string().contains("source size"),
        "{error}"
    );
}

#[test]
fn fiber_count_is_limited() {
    let mut engine = limited(Limits {
        fibers: Some(10),
        ..Limits::none()
    });
    let message = limit_message(engine.eval(r"range(0, 100) |> map(\i -> spawn(\ -> i)) |> len()"));
    assert!(message.contains("fibers"), "{message}");
}

#[test]
fn sandboxed_engines_run_ordinary_scripts() {
    let mut engine = RyndEngine::sandboxed();
    assert_eq!(
        engine
            .eval(r#"range(1, 1001) |> filter(\x -> x % 3 == 0) |> map(\x -> to_string(x)) |> join(",") |> len()"#)
            .unwrap(),
        Value::Int(1295)
    );
    let fib = "fn fib(n) { if n <= 1 { n } else { fib(n - 1) + fib(n - 2) } } fib(20)";
    assert_eq!(engine.eval(fib).unwrap(), Value::Int(6765));
    assert!(
        engine
            .eval("fn spin(n) { spin(n + 1) } spin(0)")
            .unwrap_err()
            .is_limit()
    );
    engine.reset();
    assert!(
        engine
            .eval("fn spin(n) { spin(n + 1) } spin(0)")
            .unwrap_err()
            .is_limit()
    );
}

#[test]
fn deeply_nested_source_is_a_parse_error_not_a_crash() {
    on_small_stack(|| {
        let mut engine = RyndEngine::new();
        for source in [
            format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000)),
            format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000)),
            format!("{}true", "!".repeat(100_000)),
            format!("1{}", " + 1".repeat(100_000)),
            format!("let {}x{} = 1", "[".repeat(100_000), "]".repeat(100_000)),
        ] {
            let error = engine.eval(&source).unwrap_err();
            assert!(error.to_string().contains("nested too deeply"), "{error}");
        }
        assert_eq!(engine.eval("((1 + 2) * [3][0])").unwrap(), Value::Int(9));
    });
}

#[test]
fn deeply_nested_values_drop_print_and_compare_on_a_small_stack() {
    on_small_stack(|| {
        let mut engine = RyndEngine::new();
        let nest = "fn nest(x, n) { if n == 0 { x } else { nest([x], n - 1) } }";
        assert_eq!(
            engine
                .eval(&format!("{nest} len(nest(0, 1000000))"))
                .unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            engine
                .eval(&format!("{nest} nest(0, 100000) == nest(0, 100000)"))
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .eval(&format!("{nest} nest(0, 100000) == nest(1, 100000)"))
                .unwrap(),
            Value::Bool(false)
        );
        let text = engine
            .eval(&format!("{nest} to_string(nest(0, 100000))"))
            .unwrap();
        assert!(text.to_string().contains("..."));
        let value = engine.eval(&format!("{nest} nest(0, 1000000)")).unwrap();
        drop(value);
    });
}

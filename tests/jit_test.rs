//! Native JIT (feature `jit`): differential tests against the bytecode VM.
#![cfg(feature = "jit")]

use rynd::{RyndEngine, RyndResult, Value};

fn plain(source: &str) -> RyndResult<Value> {
    RyndEngine::new().eval(source)
}

fn jit_engine() -> RyndEngine {
    let mut engine = RyndEngine::new();
    assert!(engine.enable_jit(), "host must support the Cranelift JIT");
    engine
}

/// Evaluate `source` on the VM and with the JIT, assert identical results,
/// and return the JIT engine's compiled function names.
fn differential(source: &str) -> (RyndResult<Value>, Vec<String>) {
    let expected = plain(source);
    let mut engine = jit_engine();
    let actual = engine.eval(source);
    assert_eq!(actual, expected, "JIT and VM disagree on:\n{source}");
    (actual, engine.jit_functions())
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

const FIB: &str = "fn fib(n) { if n <= 1 { n } else { fib(n - 1) + fib(n - 2) } }\n";

#[test]
fn fibonacci_is_compiled_and_matches_vm() {
    let (result, compiled) = differential(&format!("{FIB}[fib(0), fib(1), fib(10), fib(20)]"));
    assert_eq!(
        result.unwrap(),
        Value::list(vec![
            Value::Int(0),
            Value::Int(1),
            Value::Int(55),
            Value::Int(6765)
        ])
    );
    assert_eq!(compiled, names(&["fib"]));
}

#[test]
fn mutual_recursion_is_native() {
    let source = "
        fn is_even(n) { if n == 0 { true } else { is_odd(n - 1) } }
        fn is_odd(n) { if n == 0 { false } else { is_even(n - 1) } }
        [is_even(10), is_odd(10), is_even(7), is_odd(7), is_even(0)]
    ";
    let (result, compiled) = differential(source);
    assert_eq!(
        result.unwrap(),
        Value::list(vec![
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(true)
        ])
    );
    assert_eq!(compiled, names(&["is_even", "is_odd"]));
}

#[test]
fn gcd_factorial_locals_and_early_return() {
    let source = "
        fn gcd(a, b) { if b == 0 { a } else { gcd(b, a % b) } }
        fn fact(n) { if n <= 1 { 1 } else { n * fact(n - 1) } }
        fn clamp(x, lo, hi) {
            if x < lo { return lo }
            if x > hi { return hi }
            x
        }
        fn poly(x) {
            let sq = x * x
            let cube = sq * x
            let x = cube - sq
            x + 1
        }
        fn sign(n) { if n < 0 { -1 } else { if n == 0 { 0 } else { 1 } } }
        [gcd(1071, 462), gcd(17, 5), fact(20), clamp(-5, 0, 10), clamp(50, 0, 10),
         clamp(7, 0, 10), poly(3), sign(-9), sign(0), sign(4), 17 / 5, -17 / 5, -17 % 5]
    ";
    let (_, compiled) = differential(source);
    assert_eq!(compiled, names(&["clamp", "fact", "gcd", "poly", "sign"]));
}

#[test]
fn predicates_logic_and_pipelines() {
    let source = "
        fn in_range(x, lo, hi) { x >= lo and x <= hi }
        fn outside(x, lo, hi) { not in_range(x, lo, hi) }
        fn either(a, b) { a > 10 or b > 10 }
        fn same(a, b) { (a > 0) == (b > 0) }
        fn truthy(n) { not not n }
        fn add(a, b) { a + b }
        fn double(x) { x * 2 }
        fn chain(x) { x |> add(3) |> double }
        [in_range(5, 1, 10), in_range(0, 1, 10), outside(0, 1, 10), either(1, 11),
         either(1, 2), same(3, 4), same(-3, 4), truthy(0), truthy(7), chain(4)]
    ";
    let (result, compiled) = differential(source);
    let list = match result.unwrap() {
        Value::List(items) => items,
        other => panic!("expected list, got {other}"),
    };
    assert_eq!(list[9], Value::Int(14));
    assert_eq!(
        compiled,
        names(&[
            "add", "chain", "double", "either", "in_range", "outside", "same", "truthy"
        ])
    );
}

#[test]
fn ineligible_functions_stay_on_the_vm() {
    let source = r#"
        let limit = 10
        fn greet(n) { "hi " + to_string(n) }
        fn halve(x) { x / 2.0 }
        fn uses_global(x) { x + limit }
        fn listy(n) { [n, n + 1] }
        fn no_else(n) { if n > 0 { 1 } }
        fn calls_ineligible(n) { uses_global(n) + 1 }
        fn ok(n) { n + 1 }
        [greet(3), halve(3), uses_global(1), listy(1), no_else(0), calls_ineligible(2), ok(1)]
    "#;
    let (result, compiled) = differential(source);
    assert!(result.is_ok());
    assert_eq!(compiled, names(&["ok"]));
}

#[test]
fn arithmetic_errors_match_the_vm() {
    for call in [
        "add(9223372036854775807, 1)",
        "sub(-9223372036854775807, 2)",
        "mul(4611686018427387904, 2)",
        "div(1, 0)",
        "rem(1, 0)",
        "div(-9223372036854775807 - 1, -1)",
        "rem(-9223372036854775807 - 1, -1)",
        "neg(-9223372036854775807 - 1)",
    ] {
        let source = format!(
            "
            fn add(a, b) {{ a + b }}
            fn sub(a, b) {{ a - b }}
            fn mul(a, b) {{ a * b }}
            fn div(a, b) {{ a / b }}
            fn rem(a, b) {{ a % b }}
            fn neg(a) {{ -a }}
            fn outer(a) {{ a + 1 }}
            outer({call})
            "
        );
        let (result, compiled) = differential(&source);
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("Integer overflow")
                || message.contains("Division by zero")
                || message.contains("Modulo by zero"),
            "{call}: {message}"
        );
        assert_eq!(compiled.len(), 7, "{compiled:?}");
    }
}

#[test]
fn depth_limit_matches_the_vm() {
    let source = "
        fn down(n) { if n == 0 { 0 } else { 1 + down(n - 1) } }
        fn forever(n) { forever(n + 1) }
        down(255)
    ";
    let (result, compiled) = differential(source);
    assert_eq!(result.unwrap(), Value::Int(255));
    assert_eq!(compiled, names(&["down", "forever"]));

    let over = differential("fn down(n) { if n == 0 { 0 } else { 1 + down(n - 1) } }\ndown(256)").0;
    assert!(
        over.unwrap_err()
            .to_string()
            .contains("Call depth limit exceeded (256)")
    );

    let forever = differential("fn forever(n) { forever(n + 1) }\nforever(0)").0;
    assert!(
        forever
            .unwrap_err()
            .to_string()
            .contains("Call depth limit exceeded (256)")
    );
}

#[test]
fn non_int_arguments_fall_back_to_bytecode() {
    let mut engine = jit_engine();
    engine.eval(FIB).unwrap();
    assert_eq!(engine.jit_functions(), names(&["fib"]));
    let float = engine.eval("fib(2.5)").unwrap();
    assert_eq!(float, plain(&format!("{FIB}fib(2.5)")).unwrap());
    let err = engine.eval(r#"fib("x")"#).unwrap_err();
    assert_eq!(err, plain(&format!("{FIB}fib(\"x\")")).unwrap_err());
}

#[test]
fn engine_call_uses_native_code() {
    let mut engine = jit_engine();
    engine.eval(FIB).unwrap();
    assert!(matches!(
        engine.get_global("fib").unwrap(),
        Value::Compiled { .. }
    ));
    assert_eq!(
        engine.call("fib", &[Value::Int(25)]).unwrap(),
        Value::Int(75025)
    );
    assert_eq!(
        engine.call("fib", &[Value::Float(1.0)]).unwrap(),
        Value::Float(1.0)
    );
    let err = engine.call("fib", &[]).unwrap_err().to_string();
    assert!(err.contains("Expected 1 arguments, found 0"), "{err}");
}

#[test]
fn rebinding_a_callee_falls_back_to_the_vm() {
    let source = "
        fn helper(n) { n + 1 }
        fn user(n) { helper(n) * 2 }
    ";
    let mut engine = jit_engine();
    engine.eval(source).unwrap();
    assert_eq!(engine.jit_functions(), names(&["helper", "user"]));
    assert_eq!(engine.eval("user(1)").unwrap(), Value::Int(4));
    // `user` resolves `helper` through globals; a rebinding must be honoured.
    engine.eval("fn helper(n) { n + 100 }").unwrap();
    assert_eq!(engine.eval("user(1)").unwrap(), Value::Int(202));
}

#[test]
fn jit_is_opt_in() {
    let mut engine = RyndEngine::new();
    assert!(!engine.jit_enabled());
    engine.eval(FIB).unwrap();
    assert!(engine.jit_functions().is_empty());
    assert!(matches!(
        engine.get_global("fib").unwrap(),
        Value::Closure { .. }
    ));
    // Enabling later only affects programs compiled afterwards.
    assert!(engine.enable_jit());
    engine.eval("fn sq(x) { x * x }").unwrap();
    assert_eq!(engine.jit_functions(), names(&["sq"]));
    assert_eq!(engine.eval("sq(12) + fib(10)").unwrap(), Value::Int(199));
}

#[test]
fn depth_limit_spans_bytecode_and_native_frames() {
    // `wrap` stays on the VM (it calls a builtin); `down` is native.
    let program = "
        fn down(n) { if n == 0 { 0 } else { 1 + down(n - 1) } }
        fn wrap(s, n) { if n == 0 { down(200) + len(s) } else { wrap(s, n - 1) } }
    ";
    let (fits, compiled) = differential(&format!("{program}wrap(\"x\", 50)"));
    assert_eq!(fits.unwrap(), Value::Int(201));
    assert_eq!(compiled, names(&["down"]));
    let (over, _) = differential(&format!("{program}wrap(\"x\", 100)"));
    assert!(
        over.unwrap_err()
            .to_string()
            .contains("Call depth limit exceeded (256)")
    );
}

#[test]
fn cli_jit_flag_enables_native_compilation() {
    // The flag layers enable_jit() over the host setup, which still runs.
    let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let setup = |engine: &mut RyndEngine| engine.register("seed", || 9_i64);
    rynd::cli::run(
        args(&["eval", "--jit", "fn sq(n) { n * n } sq(seed())"]),
        &setup,
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rynd"))
        .args([
            "eval",
            "--jit",
            "fn fib(n) { if n <= 1 { n } else { fib(n - 1) + fib(n - 2) } } fib(25)",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "75025");
    // The same setup chaining the CLI uses: native code is really installed.
    let mut engine = RyndEngine::new();
    setup(&mut engine);
    assert!(engine.enable_jit());
    engine.eval("fn sq(n) { n * n }").unwrap();
    assert_eq!(engine.jit_functions(), names(&["sq"]));
}

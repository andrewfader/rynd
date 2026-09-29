//! Verifies the safety knobs documented in `SECURITY.md`:
//! the call-depth limit and the helper that exposes it.

#[test]
fn safety_call_depth_constant_exists() {
    // Bounds are enforced as compile-time assertions next to the constant.
    const _: () = assert!(rynd::vm::safety::MAX_CALL_DEPTH >= 64);
    const _: () = assert!(rynd::vm::safety::MAX_CALL_DEPTH <= 4096);
}

#[test]
fn call_depth_limit_returns_runtime_error() {
    let mut engine = rynd::RyndEngine::new();
    // Recurse deeper than MAX_CALL_DEPTH.
    let depth = rynd::vm::safety::MAX_CALL_DEPTH + 16;
    let source =
        format!("fn deep(n) {{ if n == 0 {{ 0 }} else {{ deep(n - 1) + 1 }} }} deep({depth})");
    let result = engine.eval(&source);
    assert!(
        result.is_err(),
        "expected recursion beyond the limit to be a runtime error, got Ok"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("depth") || msg.contains("recursion"),
        "expected a depth/recursion error message, got: {msg}"
    );
}
#[test]
fn sandboxed_engines_cannot_reach_the_host() {
    let mut engine = rynd::RyndEngine::sandboxed();
    for name in rynd::vm::safety::HOST_ACCESS_BUILTINS {
        let error = engine.eval(name).unwrap_err().to_string();
        assert!(error.contains("Undefined variable"), "{name}: {error}");
    }
    // Pure builtins and host functions stay available, including after reset.
    engine.register("host_now", || 42_i64);
    engine.reset();
    assert!(engine.eval("run_process").is_err());
    assert_eq!(
        engine
            .eval("[3, 1] |> sort() |> map(\\x -> x + host_now())")
            .unwrap()
            .to_string(),
        "[43, 45]"
    );
    let mut open = rynd::RyndEngine::new();
    assert!(open.eval("cwd").is_ok());
    assert!(open.remove_global("cwd").is_some());
    assert!(open.eval("cwd()").is_err());
}

#[test]
fn every_host_access_builtin_is_a_real_builtin() {
    let globals = rynd::RyndEngine::new().globals();
    for name in rynd::vm::safety::HOST_ACCESS_BUILTINS {
        assert!(globals.contains_key(*name), "{name} is not a builtin");
    }
}

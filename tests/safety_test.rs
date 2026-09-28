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
    let source = format!("fn deep(n) {{ if n == 0 {{ 0 }} else {{ deep(n - 1) + 1 }} }} deep({depth})");
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
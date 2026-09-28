//! Verifies that every engine module is reachable from the public API.
//! All functionality is unconditionally available — no feature flags gate it.

#[test]
fn tcp_module_available() {
    let _builtins: &[(&str, usize)] = rynd::vm::sockets::BUILTINS;
}

#[test]
fn json_module_available() {
    // JSON parsing/serialization is core; verify the public parse function is reachable.
    let parsed = rynd::vm::json::parse("[1, 2, 3]").expect("parse ok");
    assert_eq!(parsed.to_string(), "[1, 2, 3]");
}

#[test]
fn concurrency_module_available() {
    let _builtins: &[(&str, usize)] = rynd::vm::concurrency::BUILTINS;
}

#[test]
fn transpiler_available() {
    let _t = rynd::RustTranspiler::new();
}

#[test]
fn project_module_available() {
    // Reachability check via an actual call into the module.
    let tmp = std::env::temp_dir().join("rynd-feature-flag-test");
    let _ = rynd::project::new(&tmp, false);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn benchmarks_module_available() {
    let _f: fn() -> Result<(), String> = rynd::benchmarks::run;
}

#[test]
fn debugger_module_available() {
    use std::io::BufReader;
    let session: rynd::debugger::DebugSession<BufReader<std::io::Stdin>, std::io::Stdout> =
        rynd::debugger::DebugSession::new(
            BufReader::new(std::io::stdin()),
            std::io::stdout(),
            true,
        );
    drop(session);
}

#[test]
fn core_engine_still_compiles() {
    let mut engine = rynd::RyndEngine::new();
    let result = engine.eval("1 + 2").unwrap();
    assert_eq!(result.to_string(), "3");
}

#[test]
fn source_tree_module_available() {
    let mut tree = rynd::SourceTree::new();
    tree.add("main.rynd", "let x = 1");
    assert_eq!(tree.len(), 1);
}

#[test]
fn verdict_module_available() {
    let _accept = rynd::Verdict::Accept;
    let _reject = rynd::Verdict::Reject;
}

#[test]
fn host_registerable_available() {
    fn double(x: i64) -> i64 {
        x * 2
    }
    let mut engine = rynd::RyndEngine::new();
    rynd::Registerable::register(&mut engine, "double", double as fn(i64) -> i64);
    let result = engine.eval("double(21)").unwrap();
    assert_eq!(result.to_string(), "42");
}

#[test]
fn package_library_runtime_available() {
    let mut lib = rynd::Library::new();
    lib.register("noop", |x: i64| -> i64 { x });
    assert!(lib.names().contains(&"noop".to_string()));

    let mut rt = rynd::ScriptRuntime::new();
    let pkg = rynd::Package::compile(&mut rt.engine, "fn go() { 1 } go()").unwrap();
    let result = pkg.run(&mut rt.engine).unwrap();
    assert_eq!(result.to_string(), "1");
}

#[test]
fn tools_module_available() {
    assert!(rynd::has_main_function("fn main() { 1 }"));
    let ast = rynd::tools::print_ast("1 + 2");
    assert!(!ast.is_empty());
}
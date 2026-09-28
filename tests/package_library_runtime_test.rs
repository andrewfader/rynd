//! Stage 4: Package + Library + Runtime + hot-reload.

#[test]
fn package_holds_compiled_script() {
    let mut engine = rynd::RyndEngine::new();
    let pkg = rynd::Package::compile(&mut engine, "fn answer() { 42 } answer()").unwrap();
    assert!(
        pkg.entry().is_some(),
        "package should expose a compiled entry"
    );
    let result = pkg.run(&mut engine).unwrap();
    assert_eq!(result.to_string(), "42");
}

#[test]
fn hot_reload_replaces_compilation_in_same_session() {
    let mut engine = rynd::RyndEngine::new();
    let mut pkg = rynd::Package::compile(&mut engine, "fn go() { 1 } go()").unwrap();
    let first = pkg.run(&mut engine).unwrap();
    assert_eq!(first.to_string(), "1");
    pkg.reload(&mut engine, "fn go() { 99 } go()").unwrap();
    let result = pkg.run(&mut engine).unwrap();
    assert_eq!(result.to_string(), "99");
}

#[test]
fn library_collects_registered_functions() {
    let mut lib = rynd::Library::new();
    lib.register("greet", |s: String| -> String { format!("hello {s}") });
    let functions = lib.names();
    assert!(functions.contains(&"greet".to_string()));
}

#[test]
fn runtime_provides_engine_for_invoking_package() {
    let mut rt = rynd::ScriptRuntime::new();
    let pkg = rynd::Package::compile(&mut rt.engine, "fn v() { 7 } v()").unwrap();
    let result = pkg.run(&mut rt.engine).unwrap();
    assert_eq!(result.to_string(), "7");
}
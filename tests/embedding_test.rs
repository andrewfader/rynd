//! Embedding surface: packages, runtimes, in-memory source trees, the CLI
//! as a library, and inspection helpers.
use rynd::{Library, Package, RyndEngine, ScriptRuntime, SourceTree, Value};

#[test]
fn package_runs_and_hot_reloads_in_place() {
    let mut engine = RyndEngine::new();
    let mut pkg = Package::compile(&mut engine, "fn go() { 1 } go()").unwrap();
    assert_eq!(pkg.run(&mut engine).unwrap(), Value::Int(1));
    assert_eq!(
        pkg.reload(&mut engine, "fn go() { 99 } go()").unwrap(),
        Value::Int(99)
    );
    assert_eq!(pkg.run(&mut engine).unwrap(), Value::Int(99));
    assert_eq!(engine.call("go", &[]).unwrap(), Value::Int(99));
    assert_eq!(pkg.source(), "fn go() { 99 } go()");
}

#[test]
fn package_reload_keeps_other_scripts_and_old_closures_valid() {
    let mut engine = RyndEngine::new();
    let mut rules =
        Package::compile(&mut engine, "fn rate() { 5 }\nlet keep = \\x -> x + rate()").unwrap();
    rules.run(&mut engine).unwrap();
    // Compiled after the package: reload must not invalidate it.
    let other = engine.compile("fn other() { 7 } other()").unwrap();
    let old_closure = engine.get_global("keep").unwrap();

    rules
        .reload(
            &mut engine,
            "fn rate() { 10 }\nlet fresh = \\x -> x * rate()",
        )
        .unwrap();
    assert_eq!(engine.run(&other).unwrap(), Value::Int(7));
    // The old closure still runs its original code, now calling the new rate().
    assert_eq!(engine.eval("keep(1)").unwrap(), Value::Int(11));
    assert_eq!(
        engine.call("fresh", &[Value::Int(2)]).unwrap(),
        Value::Int(20)
    );
    assert!(matches!(old_closure, Value::Closure { .. }));
}

#[test]
fn failed_reload_keeps_the_previous_version() {
    let mut engine = RyndEngine::new();
    let mut pkg = Package::compile(&mut engine, "fn v() { 1 }").unwrap();
    pkg.run(&mut engine).unwrap();
    assert!(pkg.reload(&mut engine, "fn v( {").is_err());
    assert!(pkg.reload(&mut engine, "fn v() { 2 }\n1 / 0").is_err());
    assert_eq!(pkg.source(), "fn v() { 1 }");
    // The runtime failure rolled back the half-applied redefinition.
    assert_eq!(engine.call("v", &[]).unwrap(), Value::Int(1));
}

#[test]
fn package_from_another_engine_is_rejected() {
    let mut first = RyndEngine::new();
    let pkg = Package::compile(&mut first, "1").unwrap();
    let mut second = RyndEngine::new();
    assert!(pkg.run(&mut second).is_err());
    assert_eq!(pkg.script().entry(), 0);
}

#[test]
fn script_runtime_binds_a_library_and_reinstalls_it_on_reset() {
    let mut lib = Library::new();
    lib.register("fee", |cents: i64| cents / 50);
    lib.constant("CURRENCY", "USD").unwrap();
    let mut rt = ScriptRuntime::with_library(lib);
    let mut pkg = rt.load("fn total(cents) { cents + fee(cents) }").unwrap();
    assert_eq!(
        rt.call("total", &[Value::Int(1000)]).unwrap(),
        Value::Int(1020)
    );
    pkg.reload(&mut rt.engine, "fn total(cents) { cents }")
        .unwrap();
    assert_eq!(
        rt.call("total", &[Value::Int(1000)]).unwrap(),
        Value::Int(1000)
    );
    rt.reset();
    assert!(rt.call("total", &[Value::Int(1)]).is_err());
    assert_eq!(
        rt.eval("CURRENCY + to_string(fee(100))").unwrap(),
        Value::from("USD2")
    );
    assert!(rt.library().contains("fee"));
    let mut plain = ScriptRuntime::default();
    assert_eq!(plain.eval("1 + 1").unwrap(), Value::Int(2));
}

#[test]
fn source_tree_resolves_imports_without_the_filesystem() {
    let mut tree = SourceTree::new();
    tree.add(
        "lib/math.rynd",
        "pub fn square(x) { x * x }\nfn hidden() { 0 }",
    );
    tree.add(
        "lib/stats.rynd",
        "import \"math.rynd\" as m\npub fn sum_squares(xs) { xs |> map(m.square) |> sum() }",
    );
    tree.add(
        "main.rynd",
        "import \"./lib/stats.rynd\" as stats\nimport \"lib/../lib/math.rynd\" as math\npub let answer = stats.sum_squares([1, 2, 3]) + math.square(2)\nanswer",
    );
    assert_eq!(tree.len(), 3);
    assert!(tree.contains("./lib/math.rynd"));
    assert!(
        tree.get("lib/x/../math.rynd")
            .unwrap()
            .starts_with("pub fn square")
    );

    let bundle = tree.compile("main.rynd").unwrap();
    assert_eq!(bundle.exports, vec!["answer"]);
    // Each module is loaded once even when imported twice via different spellings.
    assert_eq!(bundle.files.len(), 3);

    let mut engine = RyndEngine::new();
    assert_eq!(
        engine.eval_tree(&tree, "main.rynd").unwrap(),
        Value::Int(18)
    );
    // Non-exported names stay private.
    let mut tree2 = tree.clone();
    tree2.add("bad.rynd", "import \"lib/math.rynd\" as m\nm.hidden()");
    assert!(engine.eval_tree(&tree2, "bad.rynd").is_err());
}

#[test]
fn source_tree_reports_missing_files_cycles_and_syntax_errors() {
    let mut tree = SourceTree::new();
    assert!(tree.compile("missing.rynd").is_err());

    tree.add("main.rynd", "import \"nope.rynd\" as n\n1");
    let error = tree.compile("main.rynd").unwrap_err().to_string();
    assert!(
        error.contains("nope.rynd: not found in source tree"),
        "{error}"
    );

    tree.add("main.rynd", "import \"a.rynd\" as a\n1");
    tree.add("a.rynd", "import \"main.rynd\" as m\npub let x = 1");
    let error = tree.compile("main.rynd").unwrap_err().to_string();
    assert!(error.contains("Cyclic module imports"), "{error}");

    tree.add("a.rynd", "fn broken( ");
    let error = tree.compile("main.rynd").unwrap_err().to_string();
    assert!(error.contains("a.rynd"), "errors name the file: {error}");

    assert_eq!(tree.remove("a.rynd").as_deref(), Some("fn broken( "));
    assert_eq!(tree.paths().count(), 1);
    assert!(!tree.is_empty());
}

#[test]
fn has_main_function_is_a_cheap_syntactic_gate() {
    assert!(rynd::has_main_function("fn main() { 1 + 1 }"));
    assert!(rynd::has_main_function("let x = 1\nfn main() { x }"));
    assert!(!rynd::has_main_function("fn helper() { 1 }"));
    assert!(!rynd::has_main_function("let main = 1"));
    assert!(!rynd::has_main_function("fn main("));
    assert!(!rynd::has_main_function(""));
}

#[test]
fn tools_render_ast_and_bytecode() {
    let ast = rynd::tools::print_ast("1 + 2");
    assert!(ast.contains("Add"), "{ast}");
    assert!(rynd::tools::print_ast("fn (").starts_with("parse error"));
    let dump = rynd::tools::print_bytecode("1 + 2");
    assert!(
        dump.contains("chunk 0000") && dump.contains("Binary(Add)"),
        "{dump}"
    );
    assert!(rynd::tools::print_bytecode("fn (").is_empty());
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cli_runs_as_a_library_with_host_setup() {
    let setup = |engine: &mut RyndEngine| engine.register("double", |x: i64| x * 2);
    rynd::cli::run(args(&["eval", "double(21)"]), &setup).unwrap();
    rynd::cli::run(args(&["--version"]), &setup).unwrap();
    // Without the setup hook the host function does not exist.
    let error = rynd::cli::run(args(&["eval", "double(21)"]), &|_| {}).unwrap_err();
    assert!(error.contains("Undefined variable 'double'"), "{error}");
    let error = rynd::cli::run(args(&["frobnicate"]), &setup).unwrap_err();
    assert!(error.contains("Unknown command"), "{error}");
}

#[test]
fn installed_binary_still_reports_its_version() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rynd"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("rynd {}", env!("CARGO_PKG_VERSION"))
    );
}

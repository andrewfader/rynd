use rynd::{RyndEngine, Value};
use std::{fs, path::PathBuf, process::Command};

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let dir = PathBuf::from("target").join(format!("modules-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn modules_isolate_names_share_imports_and_compile_natively() {
    let f = Fixture::new("graph");
    f.file(
        "numbers.rynd",
        r#"
        println("loaded once")
        let offset = 10
        pub fn add(x) { x + offset }
        pub fn even(n) { n == 0 ? true : odd(n - 1) }
        fn odd(n) { n == 0 ? false : even(n - 1) }
        pub fn scoped(offset) { [offset + x for x in [1,2]] }
    "#,
    );
    f.file(
        "other.rynd",
        "import \"numbers.rynd\" as numbers; pub fn twice(x) { numbers.add(x) * 2 }",
    );
    let main=f.file("main.rynd", r#"
        import "numbers.rynd" as numbers
        import "./numbers.rynd" as again
        import "other.rynd" as other
        let offset = 100
        println([numbers.add(1), again.add(2), other.twice(3), numbers.even(8), numbers.scoped(50), numbers?.offset])
    "#);
    let mut vm = RyndEngine::new();
    vm.enable_output_capture();
    vm.eval_file(&main).unwrap();
    let expected = "loaded once\n[11, 12, 26, true, [51, 52], nil]\n";
    assert_eq!(vm.take_output(), expected);
    let source = f.file("native.rs", &rynd::transpile_file(&main).unwrap());
    let exe = f.0.join("native");
    let out = Command::new("rustc")
        .args(["-O"])
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let native = Command::new(exe).output().unwrap();
    assert!(native.status.success());
    assert_eq!(native.stdout, expected.as_bytes());
}

#[test]
fn module_errors_include_source_and_reject_cycles() {
    let f = Fixture::new("errors");
    let a = f.file("a.rynd", "import \"b.rynd\" as b");
    f.file("b.rynd", "import \"a.rynd\" as a");
    assert!(
        rynd::check_file(&a)
            .unwrap_err()
            .to_string()
            .contains("Cyclic module")
    );
    f.file("b.rynd", "pub fn fail() { 1 / 0 }");
    f.file("a.rynd", "import \"b.rynd\" as b; b.fail()");
    let error = RyndEngine::new().eval_file(&a).unwrap_err().to_string();
    assert!(error.contains("b.rynd:1:"), "{error}");
    f.file("b.rynd", "pub let =");
    assert!(
        rynd::check_file(&a)
            .unwrap_err()
            .to_string()
            .contains("b.rynd:1:")
    );
    f.file("b.rynd", "pub let x = 1; pub let x = 2");
    assert!(
        rynd::check_file(&a)
            .unwrap_err()
            .to_string()
            .contains("Duplicate export")
    );
    f.file("a.rynd", "import \"missing.rynd\" as no");
    assert!(rynd::check_file(&a).is_err());
}

#[test]
fn compiled_entry_points_are_reusable_and_session_bound() {
    let mut vm = RyndEngine::new();
    let compiled = vm.compile("input |> map(\\x -> x * 2) |> sum()").unwrap();
    for n in 0..50 {
        vm.set_global("input", Value::list(vec![Value::Int(n), Value::Int(1)]));
        assert_eq!(vm.run(&compiled).unwrap(), Value::Int(2 * n + 2));
    }
    assert!(RyndEngine::new().run(&compiled).is_err());
    vm.reset();
    assert!(vm.run(&compiled).is_err());
    vm.eval("fn depth(n) { n == 0 ? 0 : depth(n - 1) }")
        .unwrap();
    assert_eq!(vm.call("depth", &[Value::Int(255)]).unwrap(), Value::Int(0));
    assert!(
        vm.call("depth", &[Value::Int(256)])
            .unwrap_err()
            .to_string()
            .contains("Call depth limit")
    );
    assert_eq!(vm.call("depth", &[Value::Int(1)]).unwrap(), Value::Int(0));
}

#[test]
fn interpolation_and_visibility_errors_are_real_errors() {
    for source in [
        r##""#{}""##,
        r##""#{1;2}""##,
        r##""#{1"##,
        "pub 1",
        "fn f() { pub let x = 1 }",
        "import 1 as foo",
    ] {
        assert!(rynd::check(source).is_err(), "{source}");
    }
}

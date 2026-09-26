use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Workspace(PathBuf);
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn successful(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rynd"))
}
fn cargo(path: &Path, action: &str) -> Command {
    let mut c = Command::new("cargo");
    c.arg(action)
        .arg("--offline")
        .arg("--manifest-path")
        .arg(path.join("Cargo.toml"));
    c
}

#[test]
fn cargo_application_is_portable_rebuilds_imports_and_calls_rust_crates() {
    let w = Workspace(
        PathBuf::from("target").join(format!("rynd-app-workflow-{}", std::process::id())),
    );
    fs::create_dir_all(&w.0).unwrap();
    let original = w.0.join("greeting-app");
    successful(cli().arg("new").arg(&original));
    assert!(
        !cli()
            .arg("new")
            .arg(&original)
            .output()
            .unwrap()
            .status
            .success()
    );
    let moved = w.0.join("moved");
    fs::rename(original, &moved).unwrap();
    // Portable after moving; no absolute compiler dependency or original checkout.
    let out = successful(
        cli()
            .arg("run")
            .arg(&moved)
            .args(["--offline", "--", "Ada"]),
    );
    assert_eq!(out.stdout, b"Hello, Ada!\n");
    successful(cli().arg("test").arg(&moved).arg("--offline"));
    successful(cli().arg("check").arg(&moved).arg("--offline"));
    // A real local Rust dependency supplies native functionality to Rynd.
    let dep = w.0.join("salutation");
    fs::create_dir_all(dep.join("src")).unwrap();
    fs::write(
        dep.join("Cargo.toml"),
        "[package]\nname=\"salutation\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    fs::write(
        dep.join("src/lib.rs"),
        "pub fn greet(name: &str) -> String { format!(\"Welcome, {name}\") }",
    )
    .unwrap();
    let manifest = moved.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).unwrap().replace(
        "[dependencies]",
        "[dependencies]\nsalutation = { path = \"../salutation\" }",
    );
    fs::write(&manifest, text).unwrap();
    let native = moved.join("src/native.rs");
    let text = fs::read_to_string(&native)
        .unwrap()
        .replace("format!(\"Hello, {name}\")", "salutation::greet(name)");
    fs::write(native, text).unwrap();
    // Only the imported Rynd file changes: Cargo must notice and regenerate Rust.
    let module = moved.join("src/greetings.rynd");
    let text = fs::read_to_string(&module).unwrap().replace("}!", "}!!");
    fs::write(&module, text).unwrap();
    let out = successful(cargo(&moved, "run").args(["--", "Grace"]));
    assert_eq!(out.stdout, b"Welcome, Grace!!\n");
    successful(
        cli()
            .arg("build")
            .arg(&moved)
            .args(["--offline", "--release"]),
    );
    // A source error fails the Cargo build instead of silently using stale code.
    fs::write(module, "pub fn broken( {").unwrap();
    let failure = cargo(&moved, "build").output().unwrap();
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("greetings.rynd"));
}

#[test]
fn generated_library_can_be_consumed_by_an_independent_rust_crate() {
    let w = Workspace(
        PathBuf::from("target").join(format!("rynd-lib-workflow-{}", std::process::id())),
    );
    fs::create_dir_all(&w.0).unwrap();
    let library = w.0.join("rules");
    successful(cli().arg("new").arg(&library).arg("--lib"));
    assert!(!library.join("src/main.rs").exists());
    successful(cargo(&library, "test").arg("--release"));
    let consumer = w.0.join("consumer");
    fs::create_dir_all(consumer.join("src")).unwrap();
    fs::write(consumer.join("Cargo.toml"),"[package]\nname=\"consumer\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[dependencies]\nrules={path=\"../rules\"}\n").unwrap();
    fs::write(
        consumer.join("src/main.rs"),
        r#"fn main() {
        let mut app=rules::Application::new(vec![]).unwrap();
        println!("{}",app.call("greet", &[rules::Value::string("Rust")]).unwrap());
    }"#,
    )
    .unwrap();
    assert_eq!(
        successful(&mut cargo(&consumer, "run")).stdout,
        b"Hello, Rust!\n"
    );
}

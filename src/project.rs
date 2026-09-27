//! Cargo projects with Rynd sources, Rust extension points and a pinned toolchain.
use crate::{RyndError, RyndResult};
use std::{fs, path::Path};

pub fn new(path: impl AsRef<Path>, library: bool) -> RyndResult<()> {
    let path = path.as_ref();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| fail("Project path needs a name"))?;
    if !name.starts_with(|c: char| c.is_ascii_alphabetic())
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(fail(
            "Project name must start with a letter and contain only letters, digits, '-' or '_'",
        ));
    }
    let crate_name = name.replace('-', "_");
    if matches!(crate_name.as_str(), "crate" | "self" | "super" | "Self") {
        return Err(fail(
            "Project name cannot be a reserved Rust keyword ('crate', 'self', 'super')",
        ));
    }
    if path.exists() {
        return Err(fail(
            "Project destination already exists; no files were changed",
        ));
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(fail)?;
    }
    fs::create_dir(path).map_err(fail)?;
    write(
        path,
        "Cargo.toml",
        &format!(
            r#"[package]
name = {name:?}
version = "0.1.0"
edition = "2024"

[dependencies]
rynd = {{ path = ".rynd/compiler" }}

[build-dependencies]
rynd = {{ path = ".rynd/compiler" }}
"#
        ),
    )?;
    write(path, ".gitignore", "/target\n")?;
    write(
        path,
        "build.rs",
        r#"fn main() {
    rynd::build::cargo_module("src/main.rynd", "rynd.rs")
        .expect("Rynd compilation failed");
}
"#,
    )?;
    write(path, "src/lib.rs", LIBRARY)?;
    write(path, "src/native.rs", NATIVE)?;
    write(
        path,
        "src/greetings.rynd",
        r##"# Private helpers stay inside this module. Rust interop is an ordinary call.
fn normalize(name) { trim(name) }
pub fn greet(name) {
    let clean = normalize(name)
    let label = clean == "" ? "world" : clean
    "#{rust_greeting(label)}!"
}
"##,
    )?;
    let mut entry = String::from(
        "import \"greetings.rynd\" as greetings\n\npub fn greet(name) { greetings.greet(name) }\n\npub let language = \"Rynd\"\n",
    );
    if !library {
        entry.push_str("\npub fn main() { println(greet(args[0] ?: \"world\")) }\n");
        write(
            path,
            "src/main.rs",
            &format!(
                r#"fn main() {{
    let result = r#{crate_name}::Application::new(std::env::args().skip(1).collect())
        .and_then(|mut app| app.call("main", &[]));
    if let Err(error) = result {{
        eprintln!("{{error}}");
        std::process::exit(1);
    }}
}}
"#
            ),
        )?;
    }
    write(path, "src/main.rynd", &entry)?;
    write(
        path,
        "README.md",
        r#"# Rynd + Rust project

Edit `src/main.rynd` and its imported modules. Cargo recompiles Rynd to direct
Rust when those files change. `pub` declarations are the library's exports.
`src/native.rs` registers Rust functions callable from Rynd; add Rust crates
under `[dependencies]` in Cargo.toml to use the Rust ecosystem there.

- `cargo build --offline --release`: compile the application/library.
- `cargo test --offline`: test calls from Rust to Rynd and back.
- `cargo run --offline -- Ada`: run a binary project.
- `Application::new(args)?.call("greet", &[Value::string("Ada")])`: call from Rust.

The generated code executes native Rust with the shared dynamic `rynd::Value`
ABI. Use `Value::from` and checked `TryFrom` conversions in Rust adapters.

`.rynd/compiler` is a pinned, vendored snapshot of the Rynd compiler/runtime,
so this project builds offline and remains portable after moving it. Check it
into source control. The snapshot uses Rust's standard library. Cargo's lockfile pins dependencies
you add. Update the snapshot deliberately when upgrading Rynd.
"#,
    )?;
    write(
        path,
        ".rynd/compiler/Cargo.toml",
        &format!(
            "[package]\nname = \"rynd\"\nversion = {:?}\nedition = \"2024\"\nautobins = false\n[lib]\npath = \"src/lib.rs\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )?;
    for (relative, source) in TOOLCHAIN {
        write(path, &format!(".rynd/compiler/{relative}"), source)?;
    }
    Ok(())
}

fn fail(error: impl std::fmt::Display) -> RyndError {
    RyndError::IoError(error.to_string())
}
fn write(root: &Path, path: &str, text: &str) -> RyndResult<()> {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(fail)?;
    }
    fs::write(path, text).map_err(fail)
}

const LIBRARY: &str = r#"pub use rynd::{RyndResult, Value};
use rynd::{NativeRuntime, Runtime};
mod native;
#[allow(dead_code, unused_imports, unused_variables, unused_mut, unreachable_code)]
mod generated { include!(concat!(env!("OUT_DIR"), "/rynd.rs")); }

pub struct Application { runtime: NativeRuntime }
impl Application {
    pub fn new(args: Vec<String>) -> RyndResult<Self> {
        let mut runtime = NativeRuntime::default();
        runtime.set_global("args", Value::list(args.into_iter().map(Value::string).collect()));
        native::register(&mut runtime);
        generated::init(&mut runtime)?;
        Ok(Self { runtime })
    }
    pub fn get(&self, export: &str) -> RyndResult<Value> {
        if !generated::EXPORTS.contains(&export) {
            return Err(rynd::vm::runtime::error(format!("Unknown public export '{export}'")));
        }
        self.runtime.get_global(export)
    }
    pub fn call(&mut self, export: &str, args: &[Value]) -> RyndResult<Value> {
        let function = self.get(export)?;
        self.runtime.call_ref(&function, args)
    }
    pub fn exports() -> &'static [&'static str] { generated::EXPORTS }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rust_calls_rynd_which_calls_rust() {
        let mut app = Application::new(vec![]).unwrap();
        assert_eq!(app.call("greet", &[Value::string("Ada")]).unwrap(), Value::string("Hello, Ada!"));
        assert_eq!(app.get("language").unwrap(), Value::string("Rynd"));
        assert!(app.call("private", &[]).is_err());
    }
}
"#;

const NATIVE: &str = r#"use rynd::{NativeRuntime, Value};

// Add ordinary Rust crate dependencies to Cargo.toml and expose their operations
// here. Values cross the boundary explicitly; errors propagate back to Rynd.
pub fn register(runtime: &mut NativeRuntime) {
    runtime.register_fn("rust_greeting", 1, |args| match &args[0] {
        Value::String(name) => Ok(Value::string(format!("Hello, {name}"))),
        _ => Err(rynd::vm::runtime::error("rust_greeting expects a string")),
    });
}
"#;

// Embedded so an installed CLI can create portable projects without depending
// on the original checkout or downloading an unpublished registry package.
const TOOLCHAIN: &[(&str, &str)] = &[
    ("src/benchmark_stats.rs", include_str!("benchmark_stats.rs")),
    ("src/benchmarks.rs", include_str!("benchmarks.rs")),
    ("src/build.rs", include_str!("build.rs")),
    ("src/debugger.rs", include_str!("debugger.rs")),
    ("src/error.rs", include_str!("error.rs")),
    ("src/lib.rs", include_str!("lib.rs")),
    ("src/main.rs", include_str!("main.rs")),
    ("src/modules.rs", include_str!("modules.rs")),
    ("src/project.rs", include_str!("project.rs")),
    ("src/syntax/captures.rs", include_str!("syntax/captures.rs")),
    ("src/syntax/ast.rs", include_str!("syntax/ast.rs")),
    ("src/syntax/lexer.rs", include_str!("syntax/lexer.rs")),
    ("src/syntax/mod.rs", include_str!("syntax/mod.rs")),
    ("src/syntax/parser.rs", include_str!("syntax/parser.rs")),
    ("src/syntax/token.rs", include_str!("syntax/token.rs")),
    ("src/transpiler/mod.rs", include_str!("transpiler/mod.rs")),
    (
        "src/transpiler/rust_codegen.rs",
        include_str!("transpiler/rust_codegen.rs"),
    ),
    ("src/vm/compiler.rs", include_str!("vm/compiler.rs")),
    ("src/vm/machine.rs", include_str!("vm/machine.rs")),
    ("src/vm/mod.rs", include_str!("vm/mod.rs")),
    ("src/vm/opcode.rs", include_str!("vm/opcode.rs")),
    ("src/vm/runtime.rs", include_str!("vm/runtime.rs")),
    ("src/vm/scripting.rs", include_str!("vm/scripting.rs")),
    ("src/vm/json.rs", include_str!("vm/json.rs")),
    ("src/vm/collections.rs", include_str!("vm/collections.rs")),
    ("src/vm/sockets.rs", include_str!("vm/sockets.rs")),
    ("src/vm/concurrency.rs", include_str!("vm/concurrency.rs")),
    ("src/vm/value.rs", include_str!("vm/value.rs")),
];

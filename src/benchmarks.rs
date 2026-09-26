//! Reproducible, fallible benchmark entry point shared by the CLI and Cargo.
use crate::syntax::{lexer::Lexer, parser::Parser};
use crate::vm::{compiler::Compiler, machine::Machine};
use crate::{RyndEngine, Value, transpile};
use std::{fs, hint::black_box, process::Command};
#[path = "benchmark_stats.rs"]
mod stats;

pub const PIPELINE: &str =
    r"range(1, 10001) |> filter(\x -> x % 2 == 0) |> map(\x -> x * 2) |> sum()";
const FIB: &str = "fn fib(n) { if n <= 1 { n } else { fib(n - 1) + fib(n - 2) } } fib(18)";

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn check(value: Value, expected: i64) -> Result<(), String> {
    if value == Value::Int(expected) {
        black_box(value);
        Ok(())
    } else {
        Err(format!(
            "Incorrect benchmark result: expected {expected}, got {value}"
        ))
    }
}

pub fn run() -> Result<(), String> {
    println!(
        "Rynd benchmarks: {} build; warmup=3 batches; samples=20",
        if cfg!(debug_assertions) {
            "debug (use --release for performance measurements)"
        } else {
            "release"
        }
    );
    println!(
        "95% intervals describe batch means on this host; they are not cross-machine guarantees."
    );
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let base = std::env::var_os("RYND_BENCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("target/bench_run"));
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let scratch = Scratch(base.join(format!("rynd-bench-{}-{unique}", std::process::id())));
    fs::create_dir(&scratch.0).map_err(|e| e.to_string())?;
    // Compile first: failures must stop the suite before any success message.
    let mut source = transpile(PIPELINE).map_err(|e| e.to_string())?;
    let main = source
        .rfind("\nfn main() {")
        .ok_or("Generated program has no main entry point")?;
    source.truncate(main);
    source.push_str("\nmod stats {\n");
    source.push_str(include_str!("benchmark_stats.rs"));
    source.push_str("\n}\n");
    source.push_str(
        r#"
fn main() {
    let mut rt=runtime::NativeRuntime::default();
    let mut action=|| -> Result<(),String> {
        let value=rynd_program(&mut rt).map_err(|e|e.to_string())?;
        if value != Value::Int(50010000) { return Err(format!("Wrong native result: {value}")); }
        std::hint::black_box(value); Ok(())
    };
    let result=if std::env::args().any(|a|a == "--once") {
        action().map(|_| "50010000".to_string())
    } else { stats::measure(&mut action,5) };
    match result { Ok(s)=>println!("{s}"), Err(e)=>{eprintln!("{e}");std::process::exit(1);} }
}
"#,
    );
    let rs = scratch.0.join("pipeline.rs");
    let bin = scratch.0.join("pipeline");
    fs::write(&rs, source).map_err(|e| e.to_string())?;
    let start = std::time::Instant::now();
    let compile = Command::new("rustc")
        .arg("-O")
        .arg(&rs)
        .arg("-o")
        .arg(&bin)
        .output()
        .map_err(|e| format!("Cannot run rustc: {e}"))?;
    if !compile.status.success() {
        return Err(format!(
            "Native compilation failed:\n{}",
            String::from_utf8_lossy(&compile.stderr)
        ));
    }
    println!(
        "Native compilation (one observation): {:.2} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    println!(
        "Lexer + parser: {}",
        stats::measure(
            || {
                let tokens = Lexer::new(black_box(PIPELINE))
                    .tokenize()
                    .map_err(|e| e.to_string())?;
                black_box(Parser::new(tokens).parse().map_err(|e| e.to_string())?);
                Ok(())
            },
            200
        )?
    );
    for (name, source, expected) in [
        ("fib(18)", FIB, 2584),
        ("pipeline 10,000 items", PIPELINE, 50010000),
    ] {
        println!(
            "VM {name}, including engine/frontend/compiler: {}",
            stats::measure(
                || {
                    check(
                        RyndEngine::new()
                            .eval(black_box(source))
                            .map_err(|e| e.to_string())?,
                        expected,
                    )
                },
                2
            )?
        );
        let tokens = Lexer::new(source).tokenize().map_err(|e| e.to_string())?;
        let program = Parser::new(tokens).parse().map_err(|e| e.to_string())?;
        let chunks = Compiler::new()
            .compile(&program)
            .map_err(|e| e.to_string())?;
        let mut vm = Machine::new(chunks);
        println!(
            "VM {name}, precompiled bytecode: {}",
            stats::measure(|| check(vm.run().map_err(|e| e.to_string())?, expected), 2)?
        );
    }
    // Reuse identical inputs and compiled scripts to isolate collection work.
    for (name, source, expected) in [
        (
            "filter + head (first match)",
            r"head(filter(items, \x -> x == 1))",
            Value::Int(1),
        ),
        (
            "find (first match)",
            r"find(items, \x -> x == 1)",
            Value::variant("Some", vec![Value::Int(1)]),
        ),
        (
            "filter + map + sum",
            r"items |> filter(\x -> x % 2 == 0) |> map(\x -> x * 2) |> sum()",
            Value::Int(50010000),
        ),
        (
            "filter_map + sum",
            r"items |> filter_map(\x -> x % 2 == 0 ? Some(x * 2) : None) |> sum()",
            Value::Int(50010000),
        ),
    ] {
        let mut engine = RyndEngine::new();
        engine.set_global("items", Value::list((1..10001).map(Value::Int).collect()));
        let script = engine.compile(source).map_err(|e| e.to_string())?;
        println!(
            "VM {name}, precompiled, input reused: {}",
            stats::measure(
                || {
                    let value = engine.run(&script).map_err(|e| e.to_string())?;
                    if value != expected {
                        return Err(format!("Wrong {name} result: {value}"));
                    }
                    black_box(value);
                    Ok(())
                },
                if name.starts_with("find") { 200 } else { 2 }
            )?
        );
    }
    let native = Command::new(&bin).output().map_err(|e| e.to_string())?;
    if !native.status.success() {
        return Err(format!(
            "Native benchmark failed: {}",
            String::from_utf8_lossy(&native.stderr)
        ));
    }
    println!(
        "Native pipeline, in-process, excluding startup: {}",
        String::from_utf8(native.stdout)
            .map_err(|e| e.to_string())?
            .trim()
    );
    println!(
        "Native pipeline, including process startup: {}",
        stats::measure(
            || {
                let output = Command::new(&bin)
                    .arg("--once")
                    .output()
                    .map_err(|e| e.to_string())?;
                if !output.status.success() || output.stdout != b"50010000\n" {
                    return Err("Native process failed or returned incorrect output".into());
                }
                Ok(())
            },
            1
        )?
    );
    println!(
        "Handwritten typed Rust pipeline, in-process: {}",
        stats::measure(
            || {
                let result: i64 = (1..black_box(10001_i64))
                    .filter(|x| x % 2 == 0)
                    .map(|x| x * 2)
                    .sum();
                check(Value::Int(black_box(result)), 50010000)
            },
            200
        )?
    );
    println!("The handwritten baseline uses iterators; Rynd materializes dynamic collections.");
    println!("Benchmark suite passed successfully.");
    Ok(())
}

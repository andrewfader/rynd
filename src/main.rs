use rynd::syntax::{lexer::Lexer, parser::Parser};
use rynd::{RyndEngine, RyndError, Value, transpile};
use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

fn print_usage() {
    println!(
        r#"Rynd — a functional language layer for Rust applications and libraries

USAGE:
    rynd new <directory> [--lib]       Create a portable Cargo application/library
    rynd build <file.rynd> [-o binary] Build a standalone native executable
    rynd build <project> [Cargo options] Build a Cargo project
    rynd test <project> [Cargo options] Test a Cargo project
    rynd run <file.rynd|-> [-- args...] Execute a script; '-' reads source from stdin
    rynd eval <expression> [-- args...] Evaluate an expression
    rynd check <file.rynd|->           Check syntax and compilation without execution
    rynd compile <file.rynd|-> [-o out.rs] Generate standalone Rust
    rynd repl                         Interactive, multiline REPL
    rynd bench                        Run performance benchmarks
    rynd --version                    Print version
    rynd --help                       Print this help

Scripts receive a list of strings in 'args'. Use read_stdin() for piped data,
read_text(path) for UTF-8 files, and -- to separate options from script arguments."#
    );
}

fn source(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|e| e.to_string())?;
        Ok(source)
    } else {
        fs::read_to_string(path).map_err(|e| format!("File Error reading '{path}': {e}"))
    }
}

fn execute() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        print_usage();
        return Ok(());
    };
    if matches!(command, "build" | "run" | "test" | "check")
        && args.get(1).is_some_and(|path| Path::new(path).is_dir())
    {
        let manifest = Path::new(&args[1]).join("Cargo.toml");
        if !manifest.is_file() {
            return Err(format!("{} has no Cargo.toml", args[1]));
        }
        let status = Command::new("cargo")
            .arg(command)
            .arg("--manifest-path")
            .arg(manifest)
            .args(&args[2..])
            .status()
            .map_err(|e| format!("Cannot run Cargo: {e}"))?;
        if !status.success() {
            return Err(format!("Cargo {command} failed ({status})"));
        }
        return Ok(());
    }
    match command {
        "new" => {
            let (path, library) = match &args[1..] {
                [path] => (path, false),
                [path, flag] if flag == "--lib" => (path, true),
                _ => return Err("Usage: rynd new <directory> [--lib]".into()),
            };
            rynd::project::new(path, library).map_err(|e| e.to_string())?;
            println!(
                "Created Rynd {} at {path}",
                if library { "library" } else { "application" }
            );
        }
        "run" | "eval" => {
            let input = args.get(1).ok_or_else(|| {
                format!(
                    "rynd {command} requires {}",
                    if command == "run" {
                        "a file path"
                    } else {
                        "an expression"
                    }
                )
            })?;
            let program = if command == "run" && input == "-" {
                source(input)?
            } else {
                input.clone()
            };
            let extra = &args[2..];
            // For eval, require a separator so accidental extra expressions are not ignored.
            if command == "eval" && !extra.is_empty() && extra[0] != "--" {
                return Err("Use -- before script arguments to rynd eval".into());
            }
            let extra = if extra.first().is_some_and(|s| s == "--") {
                &extra[1..]
            } else {
                extra
            };
            let mut engine = RyndEngine::new();
            engine.set_global(
                "args",
                Value::list(extra.iter().cloned().map(Value::string).collect()),
            );
            let value = if command == "run" && input != "-" {
                engine.eval_file(input)
            } else {
                engine.eval(&program)
            }
            .map_err(|e| format!("Execution Error: {e}"))?;
            if command == "eval" || value != Value::Nil {
                println!("{value}");
            }
        }
        "check" | "compile" | "build" => {
            let path = args
                .get(1)
                .ok_or_else(|| format!("rynd {command} requires a file path"))?;
            let output = match &args[2..] {
                [] => None,
                [flag, path] if command != "check" && flag == "-o" => Some(path),
                _ => {
                    return Err(format!(
                        "Invalid {command} options; compile/build accept -o <path>"
                    ));
                }
            };
            let bundle = if path == "-" {
                None
            } else {
                Some(rynd::modules::load(path).map_err(|e| e.to_string())?)
            };
            let raw = if path == "-" {
                Some(source(path)?)
            } else {
                None
            };
            if command == "check" {
                if let Some(bundle) = &bundle {
                    rynd::vm::compiler::Compiler::new()
                        .compile(&bundle.program)
                        .map_err(|e| e.to_string())?;
                } else {
                    rynd::check(raw.as_ref().unwrap()).map_err(|e| e.to_string())?;
                }
                println!("{path}: OK");
            } else {
                let rust = if let Some(bundle) = &bundle {
                    rynd::RustTranspiler::new().transpile(&bundle.program)
                } else {
                    transpile(raw.as_ref().unwrap())
                }
                .map_err(|e| e.to_string())?;
                let default = if Path::new(path).extension().is_some() {
                    Path::new(path).with_extension(env::consts::EXE_EXTENSION)
                } else {
                    PathBuf::from(format!("{path}.bin"))
                };
                let output = output
                    .map(PathBuf::from)
                    .or_else(|| (command == "build" && path != "-").then_some(default));
                if command == "build" && output.is_none() {
                    return Err("rynd build - requires -o <binary>".into());
                }
                if let Some(target) = output {
                    if let Some(bundle) = &bundle {
                        for input in &bundle.files {
                            rynd::build::distinct_output(input, &target)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    if command == "build" {
                        rynd::build::native(&rust, &target).map_err(|e| e.to_string())?;
                    } else {
                        fs::write(&target, rust)
                            .map_err(|e| format!("Failed to write '{}': {e}", target.display()))?;
                    }
                    eprintln!("Generated {}", target.display());
                } else {
                    print!("{rust}");
                }
            }
        }

        "repl" | "bench" | "--version" | "-V" | "--help" | "-h" | "help" => {
            if args.len() != 1 {
                return Err(format!("{command} does not accept arguments"));
            }
            match command {
                "repl" => run_repl()?,
                "bench" => rynd::benchmarks::run().map_err(|e| format!("Benchmark failed: {e}"))?,
                "--version" | "-V" => println!("rynd {}", env!("CARGO_PKG_VERSION")),
                _ => print_usage(),
            }
        }
        _ => return Err(format!("Unknown command: '{command}'. Use rynd --help.")),
    }
    Ok(())
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

// Use the language lexer/parser so delimiters in strings and comments don't
// affect continuation, and a trailing operator can continue on the next line.
fn needs_more(source: &str) -> bool {
    let tokens = match Lexer::new(source).tokenize() {
        Ok(tokens) => tokens,
        Err(RyndError::LexError { message, .. }) => {
            return message.starts_with("Unterminated string");
        }
        Err(_) => return false,
    };
    let eof = tokens.last().unwrap().span.clone();
    matches!(Parser::new(tokens).parse(), Err(RyndError::ParseError { span, .. }) if span == eof)
}

fn run_repl() -> Result<(), String> {
    let stdin = io::stdin();
    let interactive = stdin.is_terminal();
    if interactive {
        println!(
            "Rynd {} — use :help for commands",
            env!("CARGO_PKG_VERSION")
        );
    }
    let mut engine = RyndEngine::new();
    let mut reader = stdin.lock();
    let mut pending = String::new();
    loop {
        if interactive {
            print!(
                "{}",
                if pending.is_empty() {
                    "rynd> "
                } else {
                    "....> "
                }
            );
            io::stdout().flush().map_err(|e| e.to_string())?;
        }
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            if !pending.trim().is_empty() {
                return Err("Incomplete input at end of REPL session".into());
            }
            break;
        }
        let trimmed = line.trim();
        // Commands work even when abandoning an incomplete expression.
        if matches!(trimmed, ":quit" | "exit" | "quit") {
            break;
        }
        if trimmed == ":reset" {
            engine.reset();
            pending.clear();
            continue;
        }
        if trimmed == ":cancel" {
            pending.clear();
            continue;
        }
        if trimmed == ":help" {
            println!(
                ":quit — exit; :reset — clear session; :cancel — discard incomplete input\nMultiline functions, lists, strings and expressions are accepted."
            );
            continue;
        }
        if pending.is_empty() && trimmed.is_empty() {
            continue;
        }
        pending.push_str(&line);
        if needs_more(&pending) {
            continue;
        }
        match engine.eval(&pending) {
            Ok(value) => println!("=> {value}"),
            Err(error) => eprintln!("Error: {error}"),
        }
        pending.clear();
    }
    Ok(())
}

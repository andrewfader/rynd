use rynd::syntax::needs_more;
use rynd::{RyndEngine, Value, transpile};
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
    rynd lines <expr> [files...]      Transform/filter input one line at a time
    rynd -p <expr> [files...]         Alias for lines; -n suppresses result printing
    rynd debug <file.rynd> [-- args...] Interactive source debugger
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
        "lines" | "-p" | "-n" => {
            let expression = args
                .get(1)
                .ok_or("Usage: rynd lines <expression> [files...]")?;
            let mut engine = RyndEngine::new();
            let script = engine.compile(expression).map_err(|e| e.to_string())?;
            let files = args[2..]
                .strip_prefix(&["--".to_string()])
                .unwrap_or(&args[2..]);
            engine.set_global("args", Value::from(files.to_vec()));
            let mut record = 0_i64;
            let mut process = |reader: &mut dyn BufRead, file: &str| -> Result<(), String> {
                for (number, line) in reader.lines().enumerate() {
                    let line = line.map_err(|e| format!("{file}:{}: {e}", number + 1))?;
                    record = record.checked_add(1).ok_or("Record count overflow")?;
                    engine.set_global("line", Value::string(line));
                    engine.set_global(
                        "line_number",
                        Value::Int(i64::try_from(number + 1).map_err(|e| e.to_string())?),
                    );
                    engine.set_global("record_number", Value::Int(record));
                    engine.set_global("file", Value::string(file));
                    let value = engine
                        .run(&script)
                        .map_err(|e| format!("{file}:{}: {e}", number + 1))?;
                    if command != "-n" && value != Value::Nil {
                        println!("{value}");
                    }
                }
                Ok(())
            };
            if files.is_empty() {
                process(&mut io::stdin().lock(), "-")?;
            } else {
                for file in files {
                    if file == "-" {
                        process(&mut io::stdin().lock(), file)?;
                    } else {
                        let input = fs::File::open(file).map_err(|e| format!("{file}: {e}"))?;
                        process(&mut io::BufReader::new(input), file)?;
                    }
                }
            }
        }
        "debug" => {
            let path = args
                .get(1)
                .ok_or("Usage: rynd debug <file.rynd> [-- args...]")?;
            if args.len() > 2 && args[2] != "--" {
                return Err("Use -- before debugger script arguments".into());
            }
            let bundle = rynd::modules::load(path).map_err(|e| e.to_string())?;
            let (chunks, symbols) = rynd::vm::compiler::Compiler::new()
                .compile_debug(&bundle.program)
                .map_err(|e| e.to_string())?;
            let mut machine = rynd::Machine::new(chunks);
            machine.debug_symbols = symbols;
            machine.globals.insert(
                "args".into(),
                Value::from(args.get(3..).unwrap_or_default().to_vec()),
            );
            let interactive = io::stdin().is_terminal();
            machine.debugger = Some(Box::new(rynd::debugger::DebugSession::new(
                io::BufReader::new(io::stdin()),
                io::stdout(),
                interactive,
            )));
            match machine.run() {
                Ok(value) => println!("Finished: {value}"),
                Err(error) => return Err(format!("Debug execution: {error}")),
            }
        }
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
    let mut history: Vec<String> = Vec::new();
    let mut loaded: Option<String> = None;
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
                ":quit; :reset; :cancel; :vars [filter]; :complete prefix; :history; :! N\n:load FILE; :reload; :save FILE; :type EXPR; :time EXPR; :bench N EXPR\n:bench runs the full benchmark suite. _ holds the last result. Multiline input is accepted."
            );
            continue;
        }
        if pending.is_empty() && trimmed.starts_with(':') {
            let (command, rest) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
            let rest = rest.trim();
            match command {
                ":vars" | ":complete" => {
                    for (name, value) in engine.globals() {
                        if (command == ":complete" && name.starts_with(rest))
                            || (command == ":vars" && name.contains(rest))
                        {
                            if command == ":complete" {
                                println!("{name}");
                            } else {
                                println!("{name}: {} = {value}", value.type_name());
                            }
                        }
                    }
                    continue;
                }
                ":history" => {
                    for (index, source) in history.iter().enumerate() {
                        println!("{}: {}", index + 1, source.trim_end());
                    }
                    continue;
                }
                ":save" => {
                    if let Err(error) = fs::write(rest, history.join("\n")) {
                        eprintln!("Error: {error}");
                    }
                    continue;
                }
                ":load" | ":reload" => {
                    let path = if command == ":reload" {
                        loaded.as_deref().unwrap_or("")
                    } else {
                        rest
                    };
                    if path.is_empty() {
                        eprintln!("Error: :load requires a file before :reload");
                        continue;
                    }
                    match engine.eval_file(path) {
                        Ok(value) => {
                            println!("=> {value}");
                            engine.set_global("_", value);
                            loaded = Some(path.into());
                        }
                        Err(error) => eprintln!("Error: {error}"),
                    }
                    continue;
                }
                ":type" => line = format!("type_of({rest})"),
                ":time" => line = format!("benchmark(\\ -> ({rest}), 1)"),
                ":bench" if rest.is_empty() => {
                    if let Err(error) = rynd::benchmarks::run() {
                        eprintln!("Error: {error}");
                    }
                    continue;
                }
                ":bench" => {
                    if let Some((n, expression)) = rest.split_once(' ') {
                        if n.parse::<u64>().is_ok_and(|n| n > 0) {
                            line = format!("benchmark(\\ -> ({expression}), {n})");
                        } else {
                            eprintln!("Error: iterations must be positive");
                            continue;
                        }
                    } else {
                        eprintln!("Usage: :bench N EXPR");
                        continue;
                    }
                }
                ":!" => {
                    if let Some(source) = rest
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| n.checked_sub(1))
                        .and_then(|n| history.get(n))
                    {
                        line = source.clone();
                    } else {
                        eprintln!("Error: no such history entry");
                        continue;
                    }
                }
                _ => {
                    eprintln!("Unknown REPL command: {command}");
                    continue;
                }
            }
        }
        if pending.is_empty() && line.trim().is_empty() {
            continue;
        }
        pending.push_str(&line);
        if needs_more(&pending) {
            continue;
        }
        history.push(pending.clone());
        match engine.eval(&pending) {
            Ok(value) => {
                println!("=> {value}");
                engine.set_global("_", value);
            }
            Err(error) => eprintln!("Error: {error}"),
        }
        pending.clear();
    }
    Ok(())
}

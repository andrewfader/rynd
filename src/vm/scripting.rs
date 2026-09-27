//! Text, files, process execution, and measurement shared by both targets.
use super::{
    runtime::{Runtime, error},
    value::Value,
};
use crate::error::RyndResult;
use std::{
    collections::BTreeMap,
    convert::TryFrom,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Instant,
};

pub const BUILTINS: &[(&str, usize)] = &[
    ("words", 1),
    ("grep", 2),
    ("replace", 3),
    ("lower", 1),
    ("upper", 1),
    ("starts_with", 2),
    ("ends_with", 2),
    ("read_lines", 1),
    ("cat", 1),
    ("write_text", 2),
    ("append_text", 2),
    ("read_bytes", 1),
    ("write_bytes", 2),
    ("cwd", 0),
    ("env", 1),
    ("path_join", 1),
    ("list_dir", 1),
    ("file_info", 1),
    ("exists", 1),
    ("mkdir_all", 1),
    ("sleep_ms", 1),
    ("run_process", 3),
    ("parse_json_lines", 1),
    ("to_json_lines", 1),
    ("type_of", 1),
    ("benchmark", 2),
    ("assert", 2),
];
fn text(value: &Value) -> RyndResult<&str> {
    <&str>::try_from(value)
}
fn list(value: &Value) -> RyndResult<&[Value]> {
    match value {
        Value::List(items) => Ok(items),
        _ => Err(error("Expected list")),
    }
}
fn io(err: impl std::fmt::Display) -> crate::error::RyndError {
    error(err.to_string())
}
fn record<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::map(
        IntoIterator::into_iter(pairs)
            .map(|(k, v)| (k.into(), v))
            .collect::<BTreeMap<_, _>>(),
    )
}
fn bytes(data: &[u8]) -> Value {
    Value::list(data.iter().map(|b| Value::Int(*b as i64)).collect())
}

pub fn call(rt: &mut dyn Runtime, name: &str, args: &[Value]) -> RyndResult<Value> {
    match name {
        "type_of" => Ok(Value::string(args[0].type_name())),
        "assert" => {
            let message = text(&args[1])?;
            if !args[0].is_truthy() {
                return Err(error(format!("Assertion failed: {message}")));
            }
            Ok(Value::Nil)
        }
        "words" => Ok(Value::list(
            text(&args[0])?
                .split_whitespace()
                .map(Value::string)
                .collect(),
        )),
        "grep" => {
            let needle = text(&args[1])?;
            Ok(Value::list(
                text(&args[0])?
                    .lines()
                    .filter(|line| line.contains(needle))
                    .map(Value::string)
                    .collect(),
            ))
        }
        "replace" => Ok(Value::string(
            text(&args[0])?.replace(text(&args[1])?, text(&args[2])?),
        )),
        "lower" => Ok(Value::string(text(&args[0])?.to_lowercase())),
        "upper" => Ok(Value::string(text(&args[0])?.to_uppercase())),
        "starts_with" => Ok(Value::Bool(text(&args[0])?.starts_with(text(&args[1])?))),
        "ends_with" => Ok(Value::Bool(text(&args[0])?.ends_with(text(&args[1])?))),
        "read_lines" => Ok(Value::list(
            std::fs::read_to_string(text(&args[0])?)
                .map_err(io)?
                .lines()
                .map(Value::string)
                .collect(),
        )),
        "cat" => {
            let mut out = String::new();
            for path in list(&args[0])? {
                out.push_str(&std::fs::read_to_string(text(path)?).map_err(io)?);
            }
            Ok(Value::string(out))
        }
        "write_text" | "append_text" => {
            let path = text(&args[0])?;
            let content = text(&args[1])?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .append(name == "append_text")
                .truncate(name == "write_text")
                .open(path)
                .map_err(io)?;
            file.write_all(content.as_bytes()).map_err(io)?;
            Ok(Value::Nil)
        }
        "read_bytes" => Ok(bytes(&std::fs::read(text(&args[0])?).map_err(io)?)),
        "write_bytes" => {
            let data = list(&args[1])?
                .iter()
                .map(|v| {
                    u8::try_from(i64::try_from(v)?).map_err(|_| error("Byte must be in 0..255"))
                })
                .collect::<RyndResult<Vec<_>>>()?;
            std::fs::write(text(&args[0])?, data).map_err(io)?;
            Ok(Value::Nil)
        }
        "cwd" => Ok(Value::string(
            std::env::current_dir().map_err(io)?.to_string_lossy(),
        )),
        "env" => match std::env::var(text(&args[0])?) {
            Ok(value) => Ok(Value::string(value)),
            Err(std::env::VarError::NotPresent) => Ok(Value::Nil),
            Err(err) => Err(io(err)),
        },
        "path_join" => {
            let mut path = PathBuf::new();
            for part in list(&args[0])? {
                path.push(text(part)?);
            }
            Ok(Value::string(path.to_string_lossy()))
        }
        "list_dir" => {
            let mut paths = std::fs::read_dir(text(&args[0])?)
                .map_err(io)?
                .map(|entry| entry.map(|e| e.path().to_string_lossy().into_owned()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(io)?;
            paths.sort();
            Ok(Value::from(paths))
        }
        "exists" => Ok(Value::Bool(
            std::path::Path::new(text(&args[0])?)
                .try_exists()
                .map_err(io)?,
        )),
        "mkdir_all" => {
            std::fs::create_dir_all(text(&args[0])?).map_err(io)?;
            Ok(Value::Nil)
        }
        "file_info" => {
            let info = std::fs::metadata(text(&args[0])?).map_err(io)?;
            Ok(record([
                ("size", Value::Int(i64::try_from(info.len()).map_err(io)?)),
                ("is_file", Value::Bool(info.is_file())),
                ("is_dir", Value::Bool(info.is_dir())),
                ("readonly", Value::Bool(info.permissions().readonly())),
            ]))
        }
        "sleep_ms" => {
            let ms = u64::try_from(i64::try_from(&args[0])?)
                .map_err(|_| error("Sleep duration must be nonnegative"))?;
            std::thread::sleep(std::time::Duration::from_millis(ms));
            Ok(Value::Nil)
        }
        "run_process" => {
            let program = text(&args[0])?;
            let arguments = list(&args[1])?
                .iter()
                .map(text)
                .collect::<RyndResult<Vec<_>>>()?;
            let input = text(&args[2])?.as_bytes().to_vec();
            let mut child = Command::new(program)
                .args(arguments)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(io)?;
            let mut stdin = child.stdin.take().unwrap();
            // Drain output while feeding stdin, so large bidirectional pipes cannot deadlock.
            let writer = std::thread::spawn(move || stdin.write_all(&input));
            let output = child.wait_with_output().map_err(io)?;
            match writer
                .join()
                .map_err(|_| error("Process input writer failed"))?
            {
                Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => return Err(io(e)),
                _ => {}
            }
            Ok(record([
                (
                    "status",
                    output
                        .status
                        .code()
                        .map(|n| Value::Int(n as i64))
                        .unwrap_or(Value::Nil),
                ),
                ("success", Value::Bool(output.status.success())),
                (
                    "stdout",
                    Value::string(String::from_utf8_lossy(&output.stdout)),
                ),
                (
                    "stderr",
                    Value::string(String::from_utf8_lossy(&output.stderr)),
                ),
                ("stdout_bytes", bytes(&output.stdout)),
                ("stderr_bytes", bytes(&output.stderr)),
            ]))
        }
        "parse_json_lines" => {
            let mut out = Vec::new();
            for (index, line) in text(&args[0])?.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                out.push(
                    super::json::parse(line)
                        .map_err(|e| error(format!("JSON line {}: {e}", index + 1)))?,
                );
            }
            Ok(Value::list(out))
        }
        "to_json_lines" => {
            let mut out = String::new();
            for value in list(&args[0])? {
                out.push_str(&super::json::stringify(value)?);
                out.push('\n');
            }
            Ok(Value::string(out))
        }
        "benchmark" => {
            super::runtime::check_arity(&args[0], 0)?;
            let n = i64::try_from(&args[1])?;
            if n <= 0 {
                return Err(error("Benchmark iterations must be positive"));
            }
            let mut result = Value::Nil;
            let mut total = 0.0_f64;
            let mut min = f64::INFINITY;
            let mut max = 0.0_f64;
            for _ in 0..n {
                let start = Instant::now();
                result = rt.call_ref(&args[0], &[])?;
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                total += elapsed;
                min = min.min(elapsed);
                max = max.max(elapsed);
            }
            Ok(record([
                ("iterations", Value::Int(n)),
                ("total_ms", Value::Float(total)),
                ("mean_ms", Value::Float(total / n as f64)),
                ("min_ms", Value::Float(min)),
                ("max_ms", Value::Float(max)),
                ("result", result),
            ]))
        }
        _ => Err(error(format!("Unknown builtin '{name}'"))),
    }
}

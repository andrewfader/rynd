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
    ("now", 0),
    ("iso_time", 1),
    ("parse_time", 1),
    ("slugify", 1),
];

/// Milliseconds since the Unix epoch (negative before 1970).
pub fn unix_millis() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 3339 UTC with milliseconds: `2026-10-17T09:41:00.327Z`.
pub fn iso_time(millis: i64) -> String {
    let (days, ms) = (millis.div_euclid(86_400_000), millis.rem_euclid(86_400_000));
    let (y, m, d) = civil_from_days(days);
    let secs = ms / 1000;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
        ms % 1000
    )
}

/// Parse RFC 3339 (`Z` or `±HH:MM` offsets, optional fraction) or a bare
/// `YYYY-MM-DD` date into Unix milliseconds.
pub fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    let num = |s: &str| -> Option<i64> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let (date, rest) = match text.find(['T', 't', ' ']) {
        Some(i) => (&text[..i], Some(&text[i + 1..])),
        None => (text, None),
    };
    let mut fields = date.split('-');
    let (year, month, day) = (
        num(fields.next()?)?,
        num(fields.next()?)? as u32,
        num(fields.next()?)? as u32,
    );
    if fields.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if civil_from_days(days) != (year, month, day) {
        return None;
    }
    let mut millis = days * 86_400_000;
    if let Some(rest) = rest {
        let (clock, offset) = if let Some(clock) = rest.strip_suffix(['Z', 'z']) {
            (clock, 0)
        } else {
            let i = rest.rfind(['+', '-'])?;
            let (h, m) = rest[i + 1..].split_once(':')?;
            let minutes = num(h)? * 60 + num(m)?;
            (
                &rest[..i],
                if &rest[i..=i] == "+" {
                    minutes
                } else {
                    -minutes
                },
            )
        };
        let (hms, fraction) = clock.split_once('.').unwrap_or((clock, ""));
        let mut parts = hms.split(':');
        let (h, m, s) = (
            num(parts.next()?)?,
            num(parts.next()?)?,
            num(parts.next()?)?,
        );
        if parts.next().is_some() || h > 23 || m > 59 || s > 60 {
            return None;
        }
        let ms = if fraction.is_empty() {
            0
        } else {
            num(fraction)?;
            format!("{fraction:0<3}")[..3].parse::<i64>().ok()?
        };
        millis += ((h * 60 + m) * 60 + s) * 1000 + ms - offset * 60_000;
    }
    Some(millis)
}

/// Lowercase ASCII words joined by hyphens: `"Hello, Wörld!"` → `"hello-w-rld"`.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}
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
            super::concurrency::sleep(std::time::Duration::from_millis(ms))?;
            Ok(Value::Nil)
        }
        "now" => Ok(Value::Int(unix_millis())),
        "iso_time" => Ok(Value::string(iso_time(i64::try_from(&args[0])?))),
        "parse_time" => {
            let source = text(&args[0])?;
            parse_time(source)
                .map(Value::Int)
                .ok_or_else(|| error(format!("parse_time(): not an RFC 3339 time: {source:?}")))
        }
        "slugify" => Ok(Value::string(slugify(text(&args[0])?))),
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
            let output = super::concurrency::blocking(|| child.wait_with_output()).map_err(io)?;
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

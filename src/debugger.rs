//! Source debugger for Meso, with a scriptable command stream for editor/test use.
use crate::{
    RyndResult, Span, Value,
    syntax::{lexer::Lexer, parser::Parser},
    vm::{compiler::Compiler, machine::Machine, runtime::error},
};
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
    path::Path,
};

pub trait DebugHook {
    fn before_instruction(&mut self, machine: &mut Machine) -> RyndResult<()>;
    fn on_error(&mut self, _machine: &mut Machine, _error: &crate::RyndError) -> RyndResult<()> {
        Ok(())
    }
}

fn io(error_value: impl std::fmt::Display) -> crate::RyndError {
    error(error_value.to_string())
}

impl Machine {
    /// Frame zero is the innermost frame; captures and locals use lexical names.
    pub fn debug_bindings(&self, selected: usize) -> RyndResult<BTreeMap<String, Value>> {
        let index = self
            .frames
            .len()
            .checked_sub(selected + 1)
            .ok_or_else(|| error("Invalid frame"))?;
        let frame = &self.frames[index];
        let symbols = self.debug_symbols.get(frame.chunk_index);
        let ip = if selected == 0 {
            frame.ip
        } else {
            frame.ip.saturating_sub(1)
        };
        let mut values = BTreeMap::new();
        if let Value::Closure { name, upvalues, .. } = &frame.callee {
            values.insert(
                name.rsplit("::").next().unwrap_or(name).to_string(),
                frame.callee.clone(),
            );
            for (name, value) in symbols
                .into_iter()
                .flat_map(|s| &s.captures)
                .zip(upvalues.iter())
            {
                values.insert(name.clone(), value.clone());
            }
        }
        if let Some(names) = symbols.and_then(|s| s.locals.get(ip)) {
            for (name, value) in names.iter().zip(&frame.locals) {
                values.insert(name.clone(), value.clone());
            }
        }
        Ok(values)
    }
    pub fn debug_span(&self, selected: usize) -> RyndResult<Span> {
        let index = self
            .frames
            .len()
            .checked_sub(selected + 1)
            .ok_or_else(|| error("Invalid frame"))?;
        let frame = &self.frames[index];
        let ip = if selected == 0 {
            frame.ip
        } else {
            frame.ip.saturating_sub(1)
        };
        self.chunks[frame.chunk_index]
            .spans
            .get(ip)
            .cloned()
            .ok_or_else(|| error("Missing source location"))
    }
    /// Globals visible by their source names in the selected module.
    pub fn debug_globals(&self, selected: usize) -> RyndResult<BTreeMap<String, Value>> {
        let mut values: BTreeMap<_, _> = self
            .globals
            .iter()
            .filter(|(name, _)| !name.starts_with("@module:"))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        if let Some(file) = self.debug_span(selected)?.file {
            let prefix = format!("@module:{file}::");
            for (name, value) in &self.globals {
                if let Some(name) = name.strip_prefix(&prefix) {
                    values.insert(name.into(), value.clone());
                }
            }
        }
        Ok(values)
    }

    /// Evaluate against a snapshot. Script bindings are isolated; host effects are real.
    pub fn debug_eval(&self, selected: usize, source: &str) -> (RyndResult<Value>, String) {
        let mut output = String::new();
        let result = (|| {
            let program = Parser::new(Lexer::new(source).tokenize()?).parse()?;
            let mut machine = Machine::new(self.chunks.clone());
            machine.globals = self.globals.clone();
            machine.globals.extend(self.debug_globals(selected)?);
            machine.globals.extend(self.debug_bindings(selected)?);
            let entry = machine.chunks.len();
            machine
                .chunks
                .extend(Compiler::with_offset(entry).compile(&program)?);
            machine.enable_output_capture();
            let result = machine.run_from(entry);
            output = machine.get_captured_output().concat();
            result
        })();
        (result, output)
    }
}

#[derive(Clone)]
struct Breakpoint {
    file: Option<String>,
    line: usize,
    condition: Option<String>,
}
enum Mode {
    Step,
    Continue,
    Next(usize),
    Finish(usize),
}

pub struct DebugSession<R, W> {
    input: R,
    output: W,
    interactive: bool,
    breaks: Vec<Option<Breakpoint>>,
    mode: Mode,
    previous: Option<(usize, usize, usize, usize)>,
    selected: usize,
    history: Vec<String>,
    last_resume: String,
    handling_error: bool,
    terminated: bool,
}
impl<R: BufRead, W: Write> DebugSession<R, W> {
    pub fn new(input: R, output: W, interactive: bool) -> Self {
        Self {
            input,
            output,
            interactive,
            breaks: vec![],
            mode: Mode::Step,
            previous: None,
            selected: 0,
            history: vec![],
            last_resume: "next".into(),
            handling_error: false,
            terminated: false,
        }
    }
    fn say(&mut self, text: impl std::fmt::Display) -> RyndResult<()> {
        writeln!(self.output, "{text}").map_err(io)
    }
    fn evaluate(&mut self, machine: &Machine, expression: &str) -> RyndResult<()> {
        let original = expression;
        let mut expression = expression.to_owned();
        while crate::syntax::needs_more(&expression) {
            if self.interactive {
                write!(self.output, "... ").map_err(io)?;
                self.output.flush().map_err(io)?;
            }
            let mut line = String::new();
            if self.input.read_line(&mut line).map_err(io)? == 0 {
                self.terminated = true;
                return Err(error("Incomplete debugger expression"));
            }
            if line.trim() == ":cancel" {
                self.history.pop();
                return self.say("Cancelled");
            }
            expression.push('\n');
            expression.push_str(&line);
        }
        if expression != original
            && let Some(last) = self.history.last_mut()
        {
            *last = format!("p {expression}");
        }
        let (result, output) = machine.debug_eval(self.selected, &expression);
        write!(self.output, "{output}").map_err(io)?;
        match result {
            Ok(value) => self.say(format!("=> {value}")),
            Err(err) => self.say(format!("Error: {err}")),
        }
    }
    fn commands(&mut self, machine: &Machine) -> RyndResult<()> {
        loop {
            if self.interactive {
                write!(self.output, "(rynd:dbg) ").map_err(io)?;
                self.output.flush().map_err(io)?;
            }
            let mut line = String::new();
            if self.input.read_line(&mut line).map_err(io)? == 0 {
                self.terminated = true;
                return Err(error("Debugger input closed"));
            }
            let mut line = if line.trim().is_empty() {
                self.last_resume.clone()
            } else {
                line.trim().to_owned()
            };
            if let Some(index) = line.strip_prefix("! ").or_else(|| line.strip_prefix(":! ")) {
                if let Some(entry) = index
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|n| self.history.get(n))
                {
                    line = entry.clone();
                } else {
                    self.say("No such history entry")?;
                    continue;
                }
            }
            self.history.push(line.clone());
            let (cmd, rest) = line.split_once(' ').unwrap_or((&line, ""));
            let rest = rest.trim();
            let depth = machine.frames.len();
            match cmd.trim_start_matches(':') {
                "c" | "continue" => { self.mode = Mode::Continue; self.last_resume = "continue".into(); return Ok(()); }
                "s" | "step" => { self.mode = Mode::Step; self.last_resume = "step".into(); return Ok(()); }
                "n" | "next" => { self.mode = Mode::Next(depth); self.last_resume = "next".into(); return Ok(()); }
                "finish" | "out" => { self.mode = Mode::Finish(depth); self.last_resume = "finish".into(); return Ok(()); }
                "q" | "quit" => { self.terminated = true; return Err(error("Debugger stopped")); },
                "h" | "help" => self.say("break [file:]line [if expr]; delete ID; breaks; continue/c; step/s; next/n; finish\nlocals; globals; bt; frame N; list; p EXPR; type EXPR; time EXPR; bench N EXPR; history; ! N; complete PREFIX; save FILE; quit\nMultiline expressions accept :cancel. Expressions use the selected frame snapshot. Calls can perform I/O. Empty input repeats stepping.")?,
                "b" | "break" => {
                    let (location, condition) = rest.split_once(" if ").map(|(a,b)| (a, Some(b.to_owned()))).unwrap_or((rest, None));
                    let (file, line) = location.rsplit_once(':').map(|(a,b)| (Some(a.to_owned()), b)).unwrap_or((machine.debug_span(self.selected)?.file.map(|s| s.to_string()), location));
                    match line.parse::<usize>() {
                        Ok(line) if line > 0 => {
                            let file = if let Some(file) = file {
                                let path = Path::new(&file);
                                let current = machine.debug_span(self.selected)?.file;
                                let relative = current.as_ref().and_then(|name| Path::new(name.as_ref()).parent()).map(|parent| parent.join(path));
                                let resolved = path.canonicalize().or_else(|err| relative.map(|p| p.canonicalize()).unwrap_or(Err(err)));
                                match resolved.and_then(|path| std::fs::read_to_string(&path).map(|source| (path, source))) {
                                    Ok((path, source)) if line <= source.lines().count() => Some(path.to_string_lossy().into_owned()),
                                    Ok(_) => { self.say("Breakpoint line is beyond the end of the file")?; continue; }
                                    Err(err) => { self.say(format!("Error: {err}"))?; continue; }
                                }
                            } else { None };
                            self.breaks.push(Some(Breakpoint { file, line, condition }));
                            self.say(format!("Breakpoint {} at {location}", self.breaks.len()))?;
                        }
                        _ => self.say("Usage: break [file:]line [if expression]")?,
                    }
                }
                "delete" => match rest.parse::<usize>().ok().and_then(|id| id.checked_sub(1)).and_then(|id| self.breaks.get_mut(id)) {
                    Some(slot) if slot.is_some() => { *slot = None; self.say("Breakpoint deleted")?; }
                    _ => self.say("Unknown breakpoint")?,
                },
                "breaks" => {
                    for (index, bp) in self.breaks.clone().into_iter().enumerate() {
                        if let Some(bp) = bp { self.say(format!("{}: {}:{}{}", index+1, bp.file.unwrap_or_default(), bp.line,
                            bp.condition.map(|c| format!(" if {c}")).unwrap_or_default()))?; }
                    }
                }
                "locals" => for (name, value) in machine.debug_bindings(self.selected)? { self.say(format!("{name} = {value}"))?; },
                "complete" => {
                    let mut names: std::collections::BTreeSet<String> = machine.debug_globals(self.selected)?.into_keys().collect();
                    names.extend(machine.debug_bindings(self.selected)?.into_keys());
                    for name in names.into_iter().filter(|name| name.starts_with(rest)) { self.say(name)?; }
                }
                "save" => {
                    if let Err(err) = std::fs::write(rest, self.history.join("\n")) { self.say(format!("Error: {err}"))?; }
                }
                "globals" => {
                    let values = machine.debug_globals(self.selected)?;
                    for (name, value) in values.into_iter().filter(|(name,_)| name.contains(rest)) { self.say(format!("{name} = {value}"))?; }
                }
                "bt" | "where" => {
                    for (index, frame) in machine.frames.iter().rev().enumerate() {
                        self.say(format!("#{index} {} at {}", frame.callee, machine.debug_span(index)?))?;
                    }
                }
                "frame" => match rest.parse::<usize>() {
                    Ok(index) if index < depth => { self.selected = index; self.say(format!("Frame #{index} at {}", machine.debug_span(index)?))?; }
                    _ => self.say("Invalid frame")?,
                },
                "l" | "list" => {
                    let span = machine.debug_span(self.selected)?;
                    if let Some(file) = &span.file {
                        match std::fs::read_to_string(file.as_ref()) {
                            Ok(source) => for (index, line) in source.lines().enumerate().filter(|(i,_)| (i+1).abs_diff(span.line) <= 3) {
                                self.say(format!("{} {:4} {line}", if index+1 == span.line { ">" } else { " " }, index+1))?;
                            },
                            Err(err) => self.say(format!("Error: {err}"))?,
                        }
                    } else { self.say("Source has no file")?; }
                }
                "p" | "eval" => self.evaluate(machine, rest)?,
                "type" => self.evaluate(machine, &format!("type_of({rest})"))?,
                "time" => self.evaluate(machine, &format!("benchmark(\\ -> ({rest}), 1)"))?,
                "bench" => {
                    if let Some((count, expression)) = rest.split_once(' ') {
                        match count.parse::<u64>() {
                            Ok(n) if n > 0 => self.evaluate(machine, &format!("benchmark(\\ -> ({expression}), {n})"))?,
                            _ => self.say("Benchmark iterations must be positive")?,
                        }
                    } else { self.say("Usage: bench N EXPR")?; }
                }
                "history" => for (index, command) in self.history.clone().iter().enumerate() { self.say(format!("{}: {command}", index+1))?; },
                _ => self.evaluate(machine, &line)?,
            }
        }
    }
}
impl<R: BufRead, W: Write> DebugHook for DebugSession<R, W> {
    fn before_instruction(&mut self, machine: &mut Machine) -> RyndResult<()> {
        if self.terminated {
            return Err(error("Debugger stopped"));
        }
        self.handling_error = false;
        let frame = machine
            .frames
            .last()
            .ok_or_else(|| error("Missing debug frame"))?;
        use crate::vm::opcode::OpCode;
        if matches!(
            machine.chunks[frame.chunk_index].code.get(frame.ip),
            Some(
                OpCode::Return
                    | OpCode::Halt
                    | OpCode::Pop
                    | OpCode::TruncateLocals(_)
                    | OpCode::DefineLocal(_)
                    | OpCode::DefineGlobal(_)
                    | OpCode::BindPattern(_)
                    | OpCode::Jump(_)
            )
        ) {
            return Ok(());
        }
        let span = machine.debug_span(0)?;
        let depth = machine.frames.len();
        let key = (depth, frame.chunk_index, span.line, frame.ip);
        // Stop once per source-line visit, including re-entry through a call/loop.
        let previous = self.previous.replace(key);
        if previous
            .is_some_and(|old| old.0 == key.0 && old.1 == key.1 && old.2 == key.2 && key.3 > old.3)
        {
            return Ok(());
        }
        let mut hit = None;
        for (index, breakpoint) in self.breaks.iter().enumerate() {
            let Some(bp) = breakpoint else {
                continue;
            };
            if bp.line != span.line {
                continue;
            }
            if let Some(file) = &bp.file {
                let matches = span.file.as_ref().is_some_and(|current| {
                    current.as_ref() == file
                        || Path::new(file).canonicalize().ok().as_deref()
                            == Some(Path::new(current.as_ref()))
                });
                if !matches {
                    continue;
                }
            }
            if let Some(condition) = &bp.condition {
                let (result, output) = machine.debug_eval(0, condition);
                write!(self.output, "{output}").map_err(io)?;
                match result {
                    Ok(value) if value.is_truthy() => {}
                    Ok(_) => continue,
                    Err(err) => {
                        self.say(format!("Breakpoint condition error: {err}"))?;
                    }
                }
            }
            hit = Some(index + 1);
            break;
        }
        let stop = hit.is_some()
            || match self.mode {
                Mode::Step => true,
                Mode::Continue => false,
                Mode::Next(start) => depth <= start,
                Mode::Finish(start) => depth < start,
            };
        if stop {
            self.selected = 0;
            self.say(format!(
                "Stopped at {span}{}",
                hit.map(|id| format!(" (breakpoint {id})"))
                    .unwrap_or_default()
            ))?;
            self.commands(machine)?;
        }
        Ok(())
    }
    fn on_error(&mut self, machine: &mut Machine, error: &crate::RyndError) -> RyndResult<()> {
        if self.handling_error {
            return Ok(());
        }
        self.handling_error = true;
        self.selected = 0;
        self.say(format!("Exception: {error}"))?;
        self.commands(machine)
    }
}

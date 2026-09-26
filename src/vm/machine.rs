use super::opcode::{Chunk, OpCode};
use super::runtime::{self, Runtime, error};
use super::value::Value;
use crate::error::{RyndError, RyndResult, Span};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct CallFrame {
    pub chunk_index: usize,
    pub ip: usize,
    pub stack_offset: usize,
    pub locals: Vec<Value>,
    pub callee: Value,
}

pub struct Machine {
    pub chunks: Vec<Chunk>,
    pub stack: Vec<Value>,
    pub frames: Vec<CallFrame>,
    pub globals: HashMap<String, Value>,
    pub output_buffer: Option<Vec<String>>,
    local_pool: Vec<Vec<Value>>,
}
impl Machine {
    pub fn new(chunks: Vec<Chunk>) -> Self {
        Self {
            chunks,
            stack: Vec::with_capacity(256),
            frames: Vec::with_capacity(64),
            globals: runtime::globals(),
            output_buffer: None,
            local_pool: Vec::new(),
        }
    }
    pub fn enable_output_capture(&mut self) {
        self.output_buffer = Some(Vec::new());
    }
    pub fn get_captured_output(&self) -> Vec<String> {
        self.output_buffer.clone().unwrap_or_default()
    }
    pub fn register_fn(&mut self, name: &str, arity: usize, func: super::value::NativeFunction) {
        self.globals.insert(
            name.into(),
            Value::Native {
                name: name.into(),
                arity,
                func,
            },
        );
    }
    pub fn run(&mut self) -> RyndResult<Value> {
        if self.chunks.is_empty() {
            return Ok(Value::Nil);
        }
        self.run_from(0)
    }
    pub fn run_from(&mut self, chunk_index: usize) -> RyndResult<Value> {
        self.stack.clear();
        self.frames.clear();
        self.frames.push(CallFrame {
            chunk_index,
            ip: 0,
            stack_offset: 0,
            locals: self.local_pool.pop().unwrap_or_default(),
            callee: Value::Nil,
        });
        let result = self.run_until_depth(0);
        // Failed executions must not poison the next REPL/embedding evaluation.
        self.stack.clear();
        self.frames.clear();
        result
    }
    pub fn run_until_depth(&mut self, depth: usize) -> RyndResult<Value> {
        while self.frames.len() > depth {
            let frame = self.frames.last_mut().unwrap();
            let chunk = self
                .chunks
                .get(frame.chunk_index)
                .ok_or_else(|| error("Invalid function chunk"))?;
            let op = chunk
                .code
                .get(frame.ip)
                .cloned()
                .ok_or_else(|| error("Instruction pointer out of bounds"))?;
            let index = frame.chunk_index;
            let ip = frame.ip;
            frame.ip += 1;
            if let Err(error) = self.step(op) {
                let chunk = &self.chunks[index];
                let span = chunk
                    .spans
                    .get(ip)
                    .cloned()
                    .unwrap_or_else(|| Span::new(*chunk.lines.get(ip).unwrap_or(&1), 1));
                return Err(error.at(span));
            }
        }
        self.pop()
    }
    fn step(&mut self, op: OpCode) -> RyndResult<()> {
        match op {
            OpCode::Constant(i) => {
                let chunk = self.frames.last().unwrap().chunk_index;
                self.stack.push(
                    self.chunks[chunk]
                        .constants
                        .get(i)
                        .cloned()
                        .ok_or_else(|| error("Invalid constant index"))?,
                );
            }
            OpCode::Nil => self.stack.push(Value::Nil),
            OpCode::Pop => {
                self.pop()?;
            }
            OpCode::Binary(op) => {
                let b = self.pop()?;
                let a = self.pop()?;
                self.stack.push(runtime::binary(&op, a, b)?);
            }
            OpCode::Unary(op) => {
                let a = self.pop()?;
                self.stack.push(runtime::unary(&op, a)?);
            }
            OpCode::MakeList(n) => {
                let items = self.take(n)?;
                self.stack.push(Value::list(items));
            }
            OpCode::MakeTuple(n) => {
                let items = self.take(n)?;
                self.stack.push(Value::tuple(items));
            }
            OpCode::MakeMap(n) => {
                let values =
                    self.take(n.checked_mul(2).ok_or_else(|| error("Map size overflow"))?)?;
                let mut map = BTreeMap::new();
                for pair in values.chunks_exact(2) {
                    map.insert(runtime::map_key(&pair[0]), pair[1].clone());
                }
                self.stack.push(Value::map(map));
            }
            OpCode::MakeClosure {
                name,
                arity,
                chunk_index,
                captures,
            } => {
                let upvalues = Rc::new(self.take(captures)?);
                self.stack.push(Value::Closure {
                    name,
                    arity,
                    chunk_index,
                    upvalues,
                });
            }
            OpCode::BindPattern(p) => {
                let value = self.pop()?;
                self.stack.extend(
                    runtime::match_pattern(&p, &value)
                        .ok_or_else(|| error("Pattern does not match value"))?,
                );
            }
            OpCode::MatchPattern { pattern, fail } => {
                let value = self.pop()?;
                if let Some(bindings) = runtime::match_pattern(&pattern, &value) {
                    self.stack.extend(bindings);
                } else {
                    self.frames.last_mut().unwrap().ip = fail;
                }
            }
            OpCode::Range(inclusive) => {
                let end = self.pop()?;
                let start = self.pop()?;
                self.stack
                    .push(runtime::make_range(&start, &end, inclusive)?);
            }
            OpCode::GetIndex => {
                let index = self.pop()?;
                let target = self.pop()?;
                self.stack.push(runtime::index(&target, &index)?);
            }
            OpCode::GetField(name) => {
                let target = self.pop()?;
                self.stack.push(runtime::field(&target, &name, false)?);
            }
            OpCode::SafeGetField(name) => {
                let target = self.pop()?;
                self.stack.push(runtime::field(&target, &name, true)?);
            }
            OpCode::DefineGlobal(name) => {
                let value = self.pop()?;
                self.globals.insert(name.to_string(), value);
            }
            OpCode::GetGlobal(name) => self.stack.push(self.get_global(&name)?),
            OpCode::DefineLocal(i) => {
                let value = self.pop()?;
                let locals = &mut self.frames.last_mut().unwrap().locals;
                if locals.len() <= i {
                    locals.resize(i + 1, Value::Nil);
                }
                locals[i] = value;
            }
            OpCode::GetLocal(i) => self.stack.push(
                self.frames
                    .last()
                    .unwrap()
                    .locals
                    .get(i)
                    .cloned()
                    .ok_or_else(|| error("Invalid local index"))?,
            ),
            OpCode::TruncateLocals(n) => self.frames.last_mut().unwrap().locals.truncate(n),
            OpCode::GetSelf => self.stack.push(self.frames.last().unwrap().callee.clone()),
            OpCode::GetUpvalue(i) => match &self.frames.last().unwrap().callee {
                Value::Closure { upvalues, .. } => self.stack.push(
                    upvalues
                        .get(i)
                        .cloned()
                        .ok_or_else(|| error("Invalid capture index"))?,
                ),
                _ => return Err(error("Function has no captured environment")),
            },
            OpCode::Jump(offset) => self.frames.last_mut().unwrap().ip = offset,
            OpCode::JumpIfFalse(offset) => {
                if !self.peek()?.is_truthy() {
                    self.frames.last_mut().unwrap().ip = offset;
                }
            }
            OpCode::JumpIfTrue(offset) | OpCode::ElvisJump(offset) => {
                if self.peek()?.is_truthy() {
                    self.frames.last_mut().unwrap().ip = offset;
                }
            }
            OpCode::Call(n) => {
                let start = self
                    .stack
                    .len()
                    .checked_sub(n)
                    .ok_or_else(|| error("Stack underflow"))?;
                let mut args = self.local_pool.pop().unwrap_or_default();
                args.extend(self.stack.drain(start..));
                let callee = self.pop()?;
                runtime::check_arity(&callee, args.len())?;
                if matches!(callee, Value::Closure { .. } | Value::Function { .. }) {
                    self.push_frame(callee, args)?;
                } else {
                    let value = self.call_ref(&callee, &args)?;
                    args.clear();
                    self.local_pool.push(args);
                    self.stack.push(value);
                }
            }
            OpCode::Return | OpCode::Halt => {
                let value = self.pop()?;
                let mut frame = self.frames.pop().unwrap();
                self.stack.truncate(frame.stack_offset);
                self.stack.push(value);
                frame.locals.clear();
                self.local_pool.push(frame.locals);
            }
        }
        Ok(())
    }
    fn push_frame(&mut self, callee: Value, args: Vec<Value>) -> RyndResult<()> {
        let entry_frame = usize::from(
            self.frames
                .first()
                .is_some_and(|frame| matches!(frame.callee, Value::Nil)),
        );
        if self.frames.len().saturating_sub(entry_frame) >= 256 {
            return Err(error("Call depth limit exceeded (256)"));
        }
        let chunk_index = match &callee {
            Value::Closure { chunk_index, .. } | Value::Function { chunk_index, .. } => {
                *chunk_index
            }
            _ => return Err(error("Expected bytecode function")),
        };
        self.frames.push(CallFrame {
            chunk_index,
            ip: 0,
            stack_offset: self.stack.len(),
            locals: args,
            callee,
        });
        Ok(())
    }
    fn take(&mut self, n: usize) -> RyndResult<Vec<Value>> {
        let start = self
            .stack
            .len()
            .checked_sub(n)
            .ok_or_else(|| error("Stack underflow"))?;
        Ok(self.stack.split_off(start))
    }
    fn pop(&mut self) -> RyndResult<Value> {
        self.stack.pop().ok_or_else(|| error("Stack underflow"))
    }
    fn peek(&self) -> RyndResult<&Value> {
        self.stack.last().ok_or_else(|| error("Stack underflow"))
    }
    pub fn call_function_internal(&mut self, callee: Value, args: Vec<Value>) -> RyndResult<Value> {
        self.call(callee, args)
    }
}
impl Runtime for Machine {
    fn call_ref(&mut self, callee: &Value, args: &[Value]) -> RyndResult<Value> {
        runtime::check_arity(callee, args.len())?;
        match callee {
            Value::Closure { .. } | Value::Function { .. } => {
                let depth = self.frames.len();
                let mut locals = self.local_pool.pop().unwrap_or_default();
                locals.extend_from_slice(args);
                self.push_frame(callee.clone(), locals)?;
                self.run_until_depth(depth)
            }
            Value::Native { func, .. } => func(args),
            Value::Builtin { name, .. } => runtime::call_builtin(self, name, args),
            Value::Compiled { func, .. } => func(self, args, callee),
            _ => Err(error("Expected callable value")),
        }
    }
    fn call(&mut self, callee: Value, args: Vec<Value>) -> RyndResult<Value> {
        self.call_ref(&callee, &args)
    }
    fn write(&mut self, text: &str) -> RyndResult<()> {
        if let Some(output) = &mut self.output_buffer {
            output.push(text.into());
            Ok(())
        } else {
            std::io::stdout()
                .write_all(text.as_bytes())
                .map_err(|e| RyndError::IoError(e.to_string()))
        }
    }
    fn get_global(&self, name: &str) -> RyndResult<Value> {
        self.globals
            .get(name)
            .cloned()
            .ok_or_else(|| error(format!("Undefined variable '{name}'")))
    }
    fn set_global(&mut self, name: &str, value: Value) {
        self.globals.insert(name.into(), value);
    }
}

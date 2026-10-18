use super::opcode::{Chunk, OpCode};
use super::runtime::{self, Runtime, error};
use super::value::{Closure, Value};
use crate::error::{RyndError, RyndResult, Span};
use crate::syntax::ast::BinaryOp;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_LAYOUT: AtomicU64 = AtomicU64::new(1);

/// Script globals in numbered slots. Bytecode reaches a global through a slot
/// linked once per chunk, so lookups never hash. Removing a global empties
/// its slot without renumbering. Every `Globals` value (clones included) has
/// its own layout id, so a machine whose globals are replaced relinks.
pub struct Globals {
    layout: u64,
    /// Shared between clones (forked fibers) until one adds a name.
    index: Rc<HashMap<Rc<str>, u32>>,
    names: Rc<Vec<Rc<str>>>,
    values: Vec<Option<Value>>,
}

impl Globals {
    pub fn new() -> Self {
        Self {
            layout: NEXT_LAYOUT.fetch_add(1, Ordering::Relaxed),
            index: Rc::default(),
            names: Rc::default(),
            values: Vec::new(),
        }
    }
    fn slot(&mut self, name: &str) -> u32 {
        if let Some(&slot) = self.index.get(name) {
            return slot;
        }
        let slot = self.values.len() as u32;
        let name: Rc<str> = name.into();
        Rc::make_mut(&mut self.index).insert(name.clone(), slot);
        Rc::make_mut(&mut self.names).push(name);
        self.values.push(None);
        slot
    }
    pub fn get(&self, name: &str) -> Option<&Value> {
        let slot = *self.index.get(name)?;
        self.values[slot as usize].as_ref()
    }
    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
    pub fn insert(&mut self, name: impl AsRef<str>, value: Value) -> Option<Value> {
        let slot = self.slot(name.as_ref());
        self.values[slot as usize].replace(value)
    }
    pub fn remove(&mut self, name: &str) -> Option<Value> {
        let slot = *self.index.get(name)?;
        self.values[slot as usize].take()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.names
            .iter()
            .zip(&self.values)
            .filter_map(|(name, value)| Some((&**name, value.as_ref()?)))
    }
    pub fn len(&self) -> usize {
        self.values.iter().filter(|value| value.is_some()).count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for Globals {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Globals {
    fn clone(&self) -> Self {
        Self {
            layout: NEXT_LAYOUT.fetch_add(1, Ordering::Relaxed),
            index: self.index.clone(),
            names: self.names.clone(),
            values: self.values.clone(),
        }
    }
}

impl<S: AsRef<str>> Extend<(S, Value)> for Globals {
    fn extend<I: IntoIterator<Item = (S, Value)>>(&mut self, iter: I) {
        for (name, value) in iter {
            self.insert(name, value);
        }
    }
}

impl<S: AsRef<str>> FromIterator<(S, Value)> for Globals {
    fn from_iter<I: IntoIterator<Item = (S, Value)>>(iter: I) -> Self {
        let mut globals = Globals::new();
        globals.extend(iter);
        globals
    }
}

impl IntoIterator for Globals {
    type Item = (String, Value);
    type IntoIter = std::vec::IntoIter<(String, Value)>;
    fn into_iter(self) -> Self::IntoIter {
        self.names
            .iter()
            .zip(self.values)
            .filter_map(|(name, value)| Some((name.to_string(), value?)))
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl std::ops::Index<&str> for Globals {
    type Output = Value;
    fn index(&self, name: &str) -> &Value {
        self.get(name).expect("no such global")
    }
}

/// Arity and chunk of a bytecode function value.
#[inline(always)]
fn bytecode_target(callee: &Value) -> Option<(usize, usize)> {
    match callee {
        Value::Closure(c) => Some((c.arity, c.chunk_index)),
        Value::Function(f) => Some((f.arity, f.chunk_index)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CallFrame {
    pub chunk_index: usize,
    pub ip: usize,
    /// Stack index of local slot 0. Arguments occupy the first slots and the
    /// callee sits just below them; operands live above the local slots.
    pub base: usize,
}

/// Calls between limit and scheduling checks (see [`Machine::tick`]).
const TICK_CALLS: u32 = 1024;

pub struct Machine {
    /// Shared with forked fiber machines; extend with [`Machine::chunks_mut`].
    pub chunks: Rc<Vec<Chunk>>,
    pub stack: Vec<Value>,
    pub frames: Vec<CallFrame>,
    pub globals: Globals,
    /// Global slot of each chunk's `names`, for `globals` with layout `linked`.
    links: Rc<Vec<Box<[u32]>>>,
    linked: u64,
    /// Shared with fibers forked from this machine, so their output is captured too.
    pub output_buffer: Option<Rc<RefCell<Vec<String>>>>,
    args_pool: Vec<Vec<Value>>,
    /// Calls left before the next [`Machine::tick`].
    countdown: u32,
    /// Call depth allowed for the current run (see `safety::Limits`).
    depth_limit: usize,
    pub debugger: Option<Box<dyn crate::debugger::DebugHook>>,
    pub debug_symbols: Vec<super::opcode::DebugSymbols>,
}

impl Machine {
    pub fn new(chunks: Vec<Chunk>) -> Self {
        Self::with_chunks(Rc::new(chunks))
    }
    pub fn with_chunks(chunks: Rc<Vec<Chunk>>) -> Self {
        Self::with_globals(chunks, runtime::globals().into_iter().collect())
    }
    fn with_globals(chunks: Rc<Vec<Chunk>>, globals: Globals) -> Self {
        Self {
            chunks,
            stack: Vec::with_capacity(256),
            frames: Vec::with_capacity(64),
            globals,
            links: Rc::default(),
            linked: 0,
            output_buffer: None,
            args_pool: Vec::new(),
            countdown: TICK_CALLS,
            depth_limit: super::safety::MAX_CALL_DEPTH,
            debugger: None,
            debug_symbols: Vec::new(),
        }
    }
    /// Chunks for appending or patching; copied first if a fiber shares them.
    pub fn chunks_mut(&mut self) -> &mut Vec<Chunk> {
        Rc::make_mut(&mut self.chunks)
    }
    pub fn enable_output_capture(&mut self) {
        self.output_buffer = Some(Rc::default());
    }
    pub fn get_captured_output(&self) -> Vec<String> {
        self.output_buffer
            .as_ref()
            .map(|parts| parts.borrow().clone())
            .unwrap_or_default()
    }
    pub fn register_fn(&mut self, name: &str, arity: usize, func: super::value::NativeFunction) {
        self.globals.insert(name, Value::native(name, arity, func));
    }
    /// Resolve the global names of chunks added since the last run.
    fn link(&mut self) {
        if self.linked != self.globals.layout {
            self.links = Rc::default();
            self.linked = self.globals.layout;
        }
        if self.links.len() == self.chunks.len() {
            return;
        }
        let links = Rc::make_mut(&mut self.links);
        for chunk in &self.chunks[links.len()..] {
            links.push(chunk.names.iter().map(|n| self.globals.slot(n)).collect());
        }
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
        let locals = self
            .chunks
            .get(chunk_index)
            .ok_or_else(|| error("Invalid function chunk"))?
            .max_locals as usize;
        // The entry frame's callee slot holds nil; tail calls never replace it.
        self.stack.resize(1 + locals, Value::Nil);
        self.frames.push(CallFrame {
            chunk_index,
            ip: 0,
            base: 1,
        });
        let result = self.run_until_depth(0);
        // Failed executions must not poison the next REPL/embedding evaluation.
        self.stack.clear();
        self.frames.clear();
        result
    }
    pub fn run_until_depth(&mut self, depth: usize) -> RyndResult<Value> {
        if self.debugger.is_some() {
            self.execute::<true>(depth)
        } else {
            self.execute::<false>(depth)
        }
    }
    /// The callee of `frame` (nil for a top-level entry frame).
    pub fn frame_callee(&self, frame: &CallFrame) -> &Value {
        &self.stack[frame.base - 1]
    }
    /// The local slots of `frame`, parameters first.
    pub fn frame_locals(&self, frame: &CallFrame) -> &[Value] {
        let reserved = self
            .chunks
            .get(frame.chunk_index)
            .map_or(0, |chunk| chunk.max_locals as usize);
        let end = (frame.base + reserved).min(self.stack.len());
        &self.stack[frame.base.min(end)..end]
    }
    fn is_entry(&self, frame: &CallFrame) -> bool {
        matches!(self.stack[frame.base - 1], Value::Nil)
    }
    /// Active function frames, excluding the top-level entry frame.
    fn function_depth(&self) -> usize {
        let entry = usize::from(self.frames.first().is_some_and(|f| self.is_entry(f)));
        self.frames.len() - entry
    }
    /// Reserve the local slots of a frame starting at `base`.
    #[inline(always)]
    fn reserve_locals(&mut self, chunks: &[Chunk], chunk_index: usize, base: usize) {
        let needed = base + chunks[chunk_index].max_locals as usize;
        if self.stack.len() < needed {
            self.stack.resize(needed, Value::Nil);
        }
    }
    /// Periodic work between calls: charge the calls made since the last
    /// tick to the run's budget and check its time and interrupt limits.
    #[cold]
    fn tick(&mut self) -> RyndResult<()> {
        let calls = u64::from(TICK_CALLS - self.countdown);
        self.countdown = TICK_CALLS;
        super::safety::charge(calls)
    }
    #[cold]
    fn locate(&mut self, error: RyndError, chunk_index: usize, ip: usize) -> RyndError {
        let span = self
            .chunks
            .get(chunk_index)
            .map_or(Span::new(1, 1), |chunk| {
                chunk
                    .spans
                    .get(ip)
                    .cloned()
                    .unwrap_or_else(|| Span::new(*chunk.lines.get(ip).unwrap_or(&1), 1))
            });
        let error = error.at(span);
        if let Some(mut debugger) = self.debugger.take() {
            if let Some(frame) = self.frames.last_mut() {
                frame.ip = ip;
            }
            let inspected = debugger.on_error(self, &error);
            self.debugger = Some(debugger);
            if let Err(stop) = inspected {
                return stop;
            }
        }
        error
    }
    fn execute<const DEBUG: bool>(&mut self, depth: usize) -> RyndResult<Value> {
        self.link();
        self.depth_limit = super::safety::call_depth_limit();
        // Chunks and links are immutable while running; holding our own
        // references lets instructions borrow them alongside `&mut self`.
        let chunks = Rc::clone(&self.chunks);
        let links = Rc::clone(&self.links);
        'frames: loop {
            let frame = *self
                .frames
                .last()
                .ok_or_else(|| error("Missing call frame"))?;
            let chunk_index = frame.chunk_index;
            let chunk = chunks
                .get(chunk_index)
                .ok_or_else(|| error("Invalid function chunk"))?;
            let code = &chunk.code[..];
            let link = &links[chunk_index][..];
            let base = frame.base;
            let mut ip = frame.ip;
            loop {
                if DEBUG {
                    self.frames.last_mut().unwrap().ip = ip;
                    if let Some(mut debugger) = self.debugger.take() {
                        let result = debugger.before_instruction(self);
                        self.debugger = Some(debugger);
                        result?;
                    }
                }
                let Some(&op) = code.get(ip) else {
                    return Err(error("Instruction pointer out of bounds"));
                };
                ip += 1;
                macro_rules! fail {
                    ($error:expr) => {{
                        let error = $error;
                        return Err(self.locate(error, chunk_index, ip - 1));
                    }};
                }
                macro_rules! check {
                    ($result:expr) => {
                        match $result {
                            Ok(value) => value,
                            Err(error) => fail!(error),
                        }
                    };
                }
                macro_rules! pop {
                    () => {
                        match self.stack.pop() {
                            Some(value) => value,
                            None => fail!(error("Stack underflow")),
                        }
                    };
                }
                macro_rules! top {
                    () => {
                        match self.stack.last_mut() {
                            Some(value) => value,
                            None => fail!(error("Stack underflow")),
                        }
                    };
                }
                // Replace the top two operands with `a op b`, integers first.
                macro_rules! binary {
                    ($op:expr, |$x:ident, $y:ident| $fast:expr) => {{
                        let b = pop!();
                        let a = top!();
                        if let (Value::Int($x), Value::Int($y)) = (&*a, &b) {
                            let ($x, $y) = (*$x, *$y);
                            *a = check!($fast);
                        } else {
                            let left = std::mem::replace(a, Value::Nil);
                            let value = check!(runtime::binary(&$op, left, b));
                            *top!() = value;
                        }
                    }};
                }
                // `top op imm`, integers first.
                macro_rules! binary_imm {
                    ($op:expr, $imm:expr, |$x:ident, $y:ident| $fast:expr) => {{
                        let $y = $imm as i64;
                        let a = top!();
                        if let Value::Int($x) = *a {
                            *a = check!($fast);
                        } else {
                            let left = std::mem::replace(a, Value::Nil);
                            let value = check!(runtime::binary(&$op, left, Value::Int($y)));
                            *top!() = value;
                        }
                    }};
                }
                macro_rules! overflow {
                    ($value:expr) => {
                        $value
                            .map(Value::Int)
                            .ok_or_else(|| error("Integer overflow"))
                    };
                }
                match op {
                    OpCode::Int(n) => self.stack.push(Value::Int(n as i64)),
                    OpCode::AddImm(imm) => {
                        binary_imm!(BinaryOp::Add, imm, |x, y| overflow!(x.checked_add(y)))
                    }
                    OpCode::SubImm(imm) => {
                        binary_imm!(BinaryOp::Sub, imm, |x, y| overflow!(x.checked_sub(y)))
                    }
                    OpCode::MulImm(imm) => {
                        binary_imm!(BinaryOp::Mul, imm, |x, y| overflow!(x.checked_mul(y)))
                    }
                    OpCode::DivImm(imm) => binary_imm!(BinaryOp::Div, imm, |x, y| if y == 0 {
                        Err(error("Division by zero"))
                    } else {
                        overflow!(x.checked_div(y))
                    }),
                    OpCode::ModImm(imm) => binary_imm!(BinaryOp::Mod, imm, |x, y| if y == 0 {
                        Err(error("Modulo by zero"))
                    } else {
                        overflow!(x.checked_rem(y))
                    }),
                    OpCode::EqualImm(imm) => {
                        binary_imm!(BinaryOp::Equal, imm, |x, y| Ok::<_, RyndError>(
                            Value::Bool(x == y)
                        ))
                    }
                    OpCode::NotEqualImm(imm) => {
                        binary_imm!(BinaryOp::NotEqual, imm, |x, y| Ok::<_, RyndError>(
                            Value::Bool(x != y)
                        ))
                    }
                    OpCode::LessImm(imm) => {
                        binary_imm!(BinaryOp::Less, imm, |x, y| Ok::<_, RyndError>(Value::Bool(
                            x < y
                        )))
                    }
                    OpCode::LessEqualImm(imm) => {
                        binary_imm!(BinaryOp::LessEqual, imm, |x, y| Ok::<_, RyndError>(
                            Value::Bool(x <= y)
                        ))
                    }
                    OpCode::GreaterImm(imm) => {
                        binary_imm!(BinaryOp::Greater, imm, |x, y| Ok::<_, RyndError>(
                            Value::Bool(x > y)
                        ))
                    }
                    OpCode::GreaterEqualImm(imm) => {
                        binary_imm!(BinaryOp::GreaterEqual, imm, |x, y| Ok::<_, RyndError>(
                            Value::Bool(x >= y)
                        ))
                    }
                    OpCode::JumpUnless { cmp, target } => {
                        let b = pop!();
                        let a = pop!();
                        let holds = match (&a, &b) {
                            (Value::Int(x), Value::Int(y)) => cmp.ints(*x, *y),
                            _ => check!(runtime::binary(&cmp.op(), a, b)).is_truthy(),
                        };
                        if !holds {
                            ip = target as usize;
                        }
                    }
                    OpCode::JumpUnlessImm { cmp, imm, target } => {
                        let a = pop!();
                        let holds = match a {
                            Value::Int(x) => cmp.ints(x, imm as i64),
                            _ => check!(runtime::binary(&cmp.op(), a, Value::Int(imm as i64)))
                                .is_truthy(),
                        };
                        if !holds {
                            ip = target as usize;
                        }
                    }
                    OpCode::Constant(i) => match chunk.constants.get(i as usize) {
                        Some(value) => self.stack.push(value.clone()),
                        None => fail!(error("Invalid constant index")),
                    },
                    OpCode::Nil => self.stack.push(Value::Nil),
                    OpCode::Pop => {
                        pop!();
                    }
                    OpCode::Add => binary!(BinaryOp::Add, |x, y| x
                        .checked_add(y)
                        .map(Value::Int)
                        .ok_or_else(|| error("Integer overflow"))),
                    OpCode::Sub => binary!(BinaryOp::Sub, |x, y| x
                        .checked_sub(y)
                        .map(Value::Int)
                        .ok_or_else(|| error("Integer overflow"))),
                    OpCode::Mul => binary!(BinaryOp::Mul, |x, y| x
                        .checked_mul(y)
                        .map(Value::Int)
                        .ok_or_else(|| error("Integer overflow"))),
                    OpCode::Div => binary!(BinaryOp::Div, |x, y| if y == 0 {
                        Err(error("Division by zero"))
                    } else {
                        x.checked_div(y)
                            .map(Value::Int)
                            .ok_or_else(|| error("Integer overflow"))
                    }),
                    OpCode::Mod => binary!(BinaryOp::Mod, |x, y| if y == 0 {
                        Err(error("Modulo by zero"))
                    } else {
                        x.checked_rem(y)
                            .map(Value::Int)
                            .ok_or_else(|| error("Integer overflow"))
                    }),
                    OpCode::Less => {
                        binary!(BinaryOp::Less, |x, y| Ok::<_, RyndError>(Value::Bool(
                            x < y
                        )))
                    }
                    OpCode::LessEqual => binary!(BinaryOp::LessEqual, |x, y| Ok::<_, RyndError>(
                        Value::Bool(x <= y)
                    )),
                    OpCode::Greater => {
                        binary!(BinaryOp::Greater, |x, y| Ok::<_, RyndError>(Value::Bool(
                            x > y
                        )))
                    }
                    OpCode::GreaterEqual => {
                        binary!(BinaryOp::GreaterEqual, |x, y| Ok::<_, RyndError>(
                            Value::Bool(x >= y)
                        ))
                    }
                    OpCode::Equal | OpCode::NotEqual => {
                        let b = pop!();
                        let a = top!();
                        let equal = *a == b;
                        *a = Value::Bool(equal == matches!(op, OpCode::Equal));
                    }
                    OpCode::Not => {
                        let a = top!();
                        *a = Value::Bool(!a.is_truthy());
                    }
                    OpCode::Neg => {
                        let a = top!();
                        if let Value::Int(i) = a
                            && let Some(n) = i.checked_neg()
                        {
                            *i = n;
                        } else {
                            let value = std::mem::replace(a, Value::Nil);
                            *top!() =
                                check!(runtime::unary(&crate::syntax::ast::UnaryOp::Neg, value));
                        }
                    }
                    OpCode::MakeList(n) => {
                        let items = check!(self.take(n as usize));
                        self.stack.push(Value::list(items));
                    }
                    OpCode::MakeTuple(n) => {
                        let items = check!(self.take(n as usize));
                        self.stack.push(Value::tuple(items));
                    }
                    OpCode::MakeMap(n) => {
                        let values = check!(self.take(n as usize * 2));
                        let mut map = BTreeMap::new();
                        for pair in values.as_chunks::<2>().0 {
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
                        let upvalues = check!(self.take(captures as usize));
                        self.stack.push(Value::Closure(Rc::new(Closure {
                            name: chunk.names[name as usize].clone(),
                            arity: arity as usize,
                            chunk_index: chunk_index as usize,
                            upvalues,
                        })));
                    }
                    OpCode::BindPattern(p) => {
                        let value = pop!();
                        match runtime::match_pattern(&chunk.patterns[p as usize], &value) {
                            Some(bindings) => self.stack.extend(bindings),
                            None => fail!(error("Pattern does not match value")),
                        }
                    }
                    OpCode::MatchPattern { pattern, fail } => {
                        let value = pop!();
                        match runtime::match_pattern(&chunk.patterns[pattern as usize], &value) {
                            Some(bindings) => self.stack.extend(bindings),
                            None => ip = fail as usize,
                        }
                    }
                    OpCode::Range(inclusive) => {
                        let end = pop!();
                        let start = pop!();
                        let range = check!(runtime::make_range(&start, &end, inclusive));
                        self.stack.push(range);
                    }
                    OpCode::GetIndex => {
                        let index = pop!();
                        let target = top!();
                        *target = check!(runtime::index(target, &index));
                    }
                    OpCode::GetField(name) | OpCode::SafeGetField(name) => {
                        let safe = matches!(op, OpCode::SafeGetField(_));
                        let target = top!();
                        *target = check!(runtime::field(target, &chunk.names[name as usize], safe));
                    }
                    OpCode::DefineGlobal(name) => {
                        let value = pop!();
                        self.globals.values[link[name as usize] as usize] = Some(value);
                    }
                    OpCode::GetGlobal(name) => {
                        match &self.globals.values[link[name as usize] as usize] {
                            Some(value) => self.stack.push(value.clone()),
                            None => fail!(error(format!(
                                "Undefined variable '{}'",
                                chunk.names[name as usize]
                            ))),
                        }
                    }
                    OpCode::DefineLocal(i) => {
                        let value = pop!();
                        match self.stack.get_mut(base + i as usize) {
                            Some(slot) => *slot = value,
                            None => fail!(error("Invalid local index")),
                        }
                    }
                    OpCode::GetLocal(i) => match self.stack.get(base + i as usize) {
                        Some(value) => self.stack.push(value.clone()),
                        None => fail!(error("Invalid local index")),
                    },
                    OpCode::TruncateLocals(n) => {
                        let end = (base + chunk.max_locals as usize).min(self.stack.len());
                        for slot in &mut self.stack[(base + n as usize).min(end)..end] {
                            *slot = Value::Nil;
                        }
                    }
                    OpCode::GetSelf => self.stack.push(self.stack[base - 1].clone()),
                    OpCode::GetUpvalue(i) => match &self.stack[base - 1] {
                        Value::Closure(closure) => match closure.upvalues.get(i as usize) {
                            Some(value) => self.stack.push(value.clone()),
                            None => fail!(error("Invalid capture index")),
                        },
                        _ => fail!(error("Function has no captured environment")),
                    },
                    OpCode::Jump(target) => ip = target as usize,
                    OpCode::JumpIfFalse(target) => {
                        if !top!().is_truthy() {
                            ip = target as usize;
                        }
                    }
                    OpCode::JumpIfTrue(target) | OpCode::ElvisJump(target) => {
                        if top!().is_truthy() {
                            ip = target as usize;
                        }
                    }
                    OpCode::PopJumpIfFalse(target) => {
                        if !pop!().is_truthy() {
                            ip = target as usize;
                        }
                    }
                    OpCode::Call(n) | OpCode::TailCall(n) => {
                        let n = n as usize;
                        let Some(callee_at) = self.stack.len().checked_sub(n + 1) else {
                            fail!(error("Stack underflow"));
                        };
                        self.countdown -= 1;
                        if self.countdown == 0 || super::safety::tripped() {
                            self.frames.last_mut().unwrap().ip = ip;
                            check!(self.tick());
                        }
                        let target = match bytecode_target(&self.stack[callee_at]) {
                            Some((arity, chunk_index)) => {
                                if arity != n {
                                    fail!(error(format!("Expected {arity} arguments, found {n}")));
                                }
                                Some(chunk_index)
                            }
                            None => None,
                        };
                        if let Some(target) = target {
                            if chunks.get(target).is_none() {
                                fail!(error("Invalid function chunk"));
                            }
                            let new_base =
                                if matches!(op, OpCode::TailCall(_)) && !self.is_entry(&frame) {
                                    // Replace the returning frame instead of stacking a new one:
                                    // move the callee and arguments down over its slots.
                                    for i in 0..=n {
                                        self.stack.swap(base - 1 + i, callee_at + i);
                                    }
                                    self.stack.truncate(base + n);
                                    let current = self.frames.last_mut().unwrap();
                                    current.chunk_index = target;
                                    current.ip = 0;
                                    base
                                } else {
                                    if self.function_depth() >= self.depth_limit {
                                        fail!(super::safety::depth_error_at(self.depth_limit));
                                    }
                                    self.frames.last_mut().unwrap().ip = ip;
                                    self.frames.push(CallFrame {
                                        chunk_index: target,
                                        ip: 0,
                                        base: callee_at + 1,
                                    });
                                    callee_at + 1
                                };
                            self.reserve_locals(&chunks, target, new_base);
                            continue 'frames;
                        }
                        let mut args = self.args_pool.pop().unwrap_or_default();
                        args.extend(self.stack.drain(callee_at + 1..));
                        let callee = self.stack.pop().unwrap();
                        self.frames.last_mut().unwrap().ip = ip;
                        let result = self.call_ref(&callee, &args);
                        args.clear();
                        self.args_pool.push(args);
                        self.stack.push(check!(result));
                    }
                    OpCode::Return | OpCode::Halt => {
                        let value = pop!();
                        let frame = self.frames.pop().unwrap();
                        self.stack.truncate(frame.base - 1);
                        if self.frames.len() <= depth {
                            return Ok(value);
                        }
                        self.stack.push(value);
                        continue 'frames;
                    }
                }
            }
        }
    }
    fn take(&mut self, n: usize) -> RyndResult<Vec<Value>> {
        let start = self
            .stack
            .len()
            .checked_sub(n)
            .ok_or_else(|| error("Stack underflow"))?;
        Ok(self.stack.split_off(start))
    }
    // Error recovery is cold; keep frame destruction out of callback dispatch.
    #[cold]
    fn unwind_call(&mut self, depth: usize, stack_len: usize) {
        self.frames.truncate(depth);
        self.stack.truncate(stack_len);
    }
    pub fn call_function_internal(&mut self, callee: Value, args: Vec<Value>) -> RyndResult<Value> {
        self.call(callee, args)
    }
}

impl Runtime for Machine {
    fn call_ref(&mut self, callee: &Value, args: &[Value]) -> RyndResult<Value> {
        runtime::check_arity(callee, args.len())?;
        if let Some((_, chunk_index)) = bytecode_target(callee) {
            {
                if super::safety::stack_exhausted() {
                    return Err(super::safety::stack_error());
                }
                let limit = super::safety::call_depth_limit();
                if self.function_depth() >= limit {
                    return Err(super::safety::depth_error_at(limit));
                }
                self.countdown -= 1;
                if self.countdown == 0 || super::safety::tripped() {
                    self.tick()?;
                }
                let Some(chunk) = self.chunks.get(chunk_index) else {
                    return Err(error("Invalid function chunk"));
                };
                let locals = chunk.max_locals as usize;
                let depth = self.frames.len();
                let stack_len = self.stack.len();
                self.stack.push(callee.clone());
                self.stack.extend_from_slice(args);
                let base = stack_len + 1;
                if self.stack.len() < base + locals {
                    self.stack.resize(base + locals, Value::Nil);
                }
                self.frames.push(CallFrame {
                    chunk_index,
                    ip: 0,
                    base,
                });
                match self.run_until_depth(depth) {
                    Ok(value) => {
                        self.stack.truncate(stack_len);
                        Ok(value)
                    }
                    Err(error) => {
                        self.unwind_call(depth, stack_len);
                        Err(error)
                    }
                }
            }
        } else {
            match callee {
                Value::Native(native) => (native.func)(args),
                Value::Builtin(def) => runtime::call_builtin_def(self, def, args),
                Value::Compiled(compiled) => (compiled.func)(self, args, callee),
                _ => Err(error("Expected callable value")),
            }
        }
    }
    fn call(&mut self, callee: Value, args: Vec<Value>) -> RyndResult<Value> {
        self.call_ref(&callee, &args)
    }
    fn write(&mut self, text: &str) -> RyndResult<()> {
        if let Some(output) = &self.output_buffer {
            output.borrow_mut().push(text.into());
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
        self.globals.insert(name, value);
    }
    fn call_depth(&self) -> usize {
        self.function_depth()
    }
    fn fork(&self) -> RyndResult<Box<dyn Runtime>> {
        let mut child = Machine::with_globals(self.chunks.clone(), self.globals.clone());
        // The clone keeps every slot number, so the links carry over.
        if self.linked == self.globals.layout {
            child.links = self.links.clone();
            child.linked = child.globals.layout;
        }
        child.output_buffer = self.output_buffer.clone();
        Ok(Box::new(child))
    }
}

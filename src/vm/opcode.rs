use super::value::Value;
use crate::error::Span;
use crate::syntax::ast::{BinaryOp, Pattern, UnaryOp};
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub enum OpCode {
    Constant(usize),
    Nil,
    Pop,
    Binary(BinaryOp),
    Unary(UnaryOp),
    MakeList(usize),
    MakeTuple(usize),
    MakeMap(usize),
    MakeClosure {
        name: Rc<str>,
        arity: usize,
        chunk_index: usize,
        captures: usize,
    },
    BindPattern(Rc<Pattern>),
    MatchPattern {
        pattern: Rc<Pattern>,
        fail: usize,
    },
    Range(bool),
    GetIndex,
    GetField(Rc<str>),
    SafeGetField(Rc<str>),
    ElvisJump(usize),
    DefineGlobal(Rc<str>),
    GetGlobal(Rc<str>),
    DefineLocal(usize),
    GetLocal(usize),
    GetUpvalue(usize),
    GetSelf,
    TruncateLocals(usize),
    Jump(usize),
    JumpIfFalse(usize),
    JumpIfTrue(usize),
    Call(usize),
    Return,
    Halt,
}

#[derive(Debug, Clone, Default)]
pub struct Chunk {
    pub code: Vec<OpCode>,
    pub constants: Vec<Value>,
    pub lines: Vec<usize>,
    pub spans: Vec<Span>,
}
impl Chunk {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn write_op(&mut self, op: OpCode, line: usize) -> usize {
        let offset = self.code.len();
        self.code.push(op);
        self.lines.push(line);
        offset
    }
    pub fn add_constant(&mut self, val: Value) -> usize {
        let index = self.constants.len();
        self.constants.push(val);
        index
    }
}

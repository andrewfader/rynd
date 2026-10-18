use super::value::Value;
use crate::error::Span;
use crate::syntax::ast::Pattern;
use std::rc::Rc;

/// One instruction. Operands are indices into the owning [`Chunk`]'s tables
/// (`constants`, `names`, `patterns`) or code offsets, so instructions are
/// `Copy` and dispatch never touches a reference count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpCode {
    Constant(u32),
    Nil,
    Pop,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Not,
    Neg,
    /// An integer literal small enough to live in the instruction.
    Int(i32),
    /// `top op imm` with an immediate integer right operand.
    AddImm(i32),
    SubImm(i32),
    MulImm(i32),
    DivImm(i32),
    ModImm(i32),
    EqualImm(i32),
    NotEqualImm(i32),
    LessImm(i32),
    LessEqualImm(i32),
    GreaterImm(i32),
    GreaterEqualImm(i32),
    /// Compare the top two operands, pop both, and jump when the comparison
    /// is false: the condition of an `if` or match guard in one dispatch.
    JumpUnless {
        cmp: Cmp,
        target: u32,
    },
    /// As `JumpUnless`, comparing the top operand with an immediate.
    JumpUnlessImm {
        cmp: Cmp,
        imm: i32,
        target: u32,
    },
    MakeList(u32),
    MakeTuple(u32),
    MakeMap(u32),
    MakeClosure {
        name: u32,
        arity: u32,
        chunk_index: u32,
        captures: u32,
    },
    BindPattern(u32),
    MatchPattern {
        pattern: u32,
        fail: u32,
    },
    Range(bool),
    GetIndex,
    GetField(u32),
    SafeGetField(u32),
    ElvisJump(u32),
    /// Global access by `names` index; each machine links names to slots.
    DefineGlobal(u32),
    GetGlobal(u32),
    DefineLocal(u32),
    GetLocal(u32),
    GetUpvalue(u32),
    GetSelf,
    TruncateLocals(u32),
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    /// Pop the condition, then jump if it was falsy (`if` and match guards).
    PopJumpIfFalse(u32),
    Call(u32),
    /// A call whose result the function returns directly: bytecode callees
    /// reuse the caller's frame, so tail recursion runs in constant space.
    TailCall(u32),
    Return,
    Halt,
}

/// A comparison fused into a conditional jump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

impl Cmp {
    pub fn op(self) -> crate::syntax::ast::BinaryOp {
        use crate::syntax::ast::BinaryOp;
        match self {
            Cmp::Equal => BinaryOp::Equal,
            Cmp::NotEqual => BinaryOp::NotEqual,
            Cmp::Less => BinaryOp::Less,
            Cmp::LessEqual => BinaryOp::LessEqual,
            Cmp::Greater => BinaryOp::Greater,
            Cmp::GreaterEqual => BinaryOp::GreaterEqual,
        }
    }
    #[inline(always)]
    pub fn ints(self, a: i64, b: i64) -> bool {
        match self {
            Cmp::Equal => a == b,
            Cmp::NotEqual => a != b,
            Cmp::Less => a < b,
            Cmp::LessEqual => a <= b,
            Cmp::Greater => a > b,
            Cmp::GreaterEqual => a >= b,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Chunk {
    pub code: Vec<OpCode>,
    pub constants: Vec<Value>,
    /// Global, field, and function names used by this chunk's instructions.
    pub names: Vec<Rc<str>>,
    pub patterns: Vec<Rc<Pattern>>,
    /// Local slots a frame of this chunk reserves (parameters first).
    pub max_locals: u32,
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
    /// Render one instruction with its operands resolved, for listings.
    pub fn describe(&self, op: &OpCode) -> String {
        let name = |i: &u32| self.names.get(*i as usize).map_or("?", |n| &**n);
        match op {
            OpCode::Constant(i) => match self.constants.get(*i as usize) {
                Some(Value::String(s)) => format!("Constant({s:?})"),
                Some(value) => format!("Constant({value})"),
                None => format!("Constant(#{i})"),
            },
            OpCode::GetGlobal(i) => format!("GetGlobal({})", name(i)),
            OpCode::DefineGlobal(i) => format!("DefineGlobal({})", name(i)),
            OpCode::GetField(i) => format!("GetField({})", name(i)),
            OpCode::SafeGetField(i) => format!("SafeGetField({})", name(i)),
            OpCode::MakeClosure {
                name: n,
                arity,
                chunk_index,
                captures,
            } => format!(
                "MakeClosure({}/{arity}, chunk {chunk_index}, {captures} captures)",
                name(n)
            ),
            other => format!("{other:?}"),
        }
    }
}

/// Kept separately from bytecode chunks so ordinary programs retain no debug data.
#[derive(Debug, Clone, Default)]
pub struct DebugSymbols {
    pub locals: Vec<Vec<String>>,
    pub captures: Vec<String>,
}

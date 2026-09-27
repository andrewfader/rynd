use super::opcode::{Chunk, DebugSymbols, OpCode};
use super::runtime::{literal, pattern_names};
use super::value::Value;
use crate::error::{RyndError, RyndResult, Span};
use crate::syntax::ast::*;
use std::collections::HashSet;

#[derive(Clone)]
struct Local {
    name: String,
    depth: usize,
}

pub struct Compiler {
    chunks: Vec<Chunk>,
    current_chunk: usize,
    base: usize,
    locals: Vec<Local>,
    upvalues: Vec<String>,
    scope_depth: usize,
    self_name: Option<String>,
    span: Span,
    debug: bool,
    debug_symbols: Vec<DebugSymbols>,
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}
impl Compiler {
    pub fn new() -> Self {
        Self::with_offset(0)
    }
    pub fn with_offset(base: usize) -> Self {
        Self {
            chunks: vec![Chunk::new()],
            current_chunk: 0,
            base,
            locals: vec![],
            upvalues: vec![],
            scope_depth: 0,
            self_name: None,
            span: Span::new(1, 1),
            debug: false,
            debug_symbols: Vec::new(),
        }
    }
    pub fn compile_debug(
        mut self,
        program: &Program,
    ) -> RyndResult<(Vec<Chunk>, Vec<DebugSymbols>)> {
        self.debug = true;
        self.debug_symbols.push(DebugSymbols::default());
        self.compile_program(program)?;
        Ok((self.chunks, self.debug_symbols))
    }
    pub fn compile(mut self, program: &Program) -> RyndResult<Vec<Chunk>> {
        self.compile_program(program)?;
        Ok(self.chunks)
    }
    fn compile_program(&mut self, program: &Program) -> RyndResult<()> {
        for (i, stmt) in program.statements.iter().enumerate() {
            self.statement(stmt, i + 1 == program.statements.len())?;
        }
        if !matches!(program.statements.last(), Some(Stmt::Expression(_))) {
            self.emit(OpCode::Nil);
        }
        self.emit(OpCode::Halt);
        Ok(())
    }
    fn chunk(&mut self) -> &mut Chunk {
        &mut self.chunks[self.current_chunk]
    }
    fn emit(&mut self, op: OpCode) -> usize {
        if self.debug {
            let names = self.locals.iter().map(|local| local.name.clone()).collect();
            self.debug_symbols[self.current_chunk].locals.push(names);
        }
        let span = self.span.clone();
        self.chunk().spans.push(span.clone());
        self.chunk().write_op(op, span.line)
    }
    fn constant(&mut self, value: Value) {
        let idx = self.chunk().add_constant(value);
        self.emit(OpCode::Constant(idx));
    }
    fn patch(&mut self, offset: usize) {
        let end = self.chunk().code.len();
        match &mut self.chunk().code[offset] {
            OpCode::Jump(n)
            | OpCode::JumpIfFalse(n)
            | OpCode::JumpIfTrue(n)
            | OpCode::ElvisJump(n) => *n = end,
            OpCode::MatchPattern { fail, .. } => *fail = end,
            _ => unreachable!(),
        }
    }
    fn begin_scope(&mut self) {
        self.scope_depth += 1;
    }
    fn end_scope(&mut self) {
        self.scope_depth -= 1;
        self.locals.retain(|v| v.depth <= self.scope_depth);
        self.emit(OpCode::TruncateLocals(self.locals.len()));
    }
    fn lookup(&self, name: &str) -> OpCode {
        if let Some(i) = self.locals.iter().rposition(|v| v.name == name) {
            OpCode::GetLocal(i)
        } else if self.self_name.as_deref() == Some(name) {
            OpCode::GetSelf
        } else if let Some(i) = self.upvalues.iter().position(|v| v == name) {
            OpCode::GetUpvalue(i)
        } else {
            OpCode::GetGlobal(name.into())
        }
    }
    fn fail(&self, message: impl Into<String>) -> RyndError {
        RyndError::CompileError {
            message: message.into(),
        }
        .at(self.span.clone())
    }
    fn bind(&mut self, pattern: &Pattern) -> RyndResult<()> {
        let names = pattern_names(pattern);
        let unique: HashSet<_> = names.iter().collect();
        if unique.len() != names.len() {
            return Err(self.fail("Duplicate variable in pattern"));
        }
        self.emit(OpCode::BindPattern(std::rc::Rc::new(pattern.clone())));
        self.bind_names(names);
        Ok(())
    }
    fn bind_names(&mut self, names: Vec<String>) {
        if self.scope_depth == 0 {
            for name in names.into_iter().rev() {
                self.emit(OpCode::DefineGlobal(name.into()));
            }
        } else {
            let base = self.locals.len();
            for name in names {
                self.locals.push(Local {
                    name,
                    depth: self.scope_depth,
                });
            }
            for idx in (base..self.locals.len()).rev() {
                self.emit(OpCode::DefineLocal(idx));
            }
        }
    }
    fn statement(&mut self, stmt: &Stmt, last: bool) -> RyndResult<()> {
        match stmt {
            Stmt::Public(inner) => {
                if self.scope_depth != 0 {
                    return Err(self.fail("pub is only allowed at module scope"));
                }
                self.statement(inner, last)?;
            }
            Stmt::Import { span, .. } => {
                return Err(RyndError::CompileError {
                    message:
                        "Imports require a file entry point (eval_file, run, or a Cargo project)"
                            .into(),
                }
                .at(span.clone()));
            }
            Stmt::Let {
                pattern,
                init,
                is_mut,
                span,
            } => {
                self.span = span.clone();
                if *is_mut {
                    return Err(self.fail("Rynd bindings are immutable; use a new let binding"));
                }
                self.compile_expression(init)?;
                self.span = span.clone();
                self.bind(pattern)?;
            }
            Stmt::Function {
                name,
                params,
                body,
                span,
            } => {
                self.span = span.clone();
                self.function(name, params, body)?;
                self.bind_names(vec![name.clone()]);
            }
            Stmt::Expression(expr) => {
                self.compile_expression(expr)?;
                if !last {
                    self.emit(OpCode::Pop);
                }
            }
            Stmt::Return { value, span } => {
                self.span = span.clone();
                if self.current_chunk == 0 {
                    return Err(self.fail("return is only valid inside a function"));
                }
                if let Some(v) = value {
                    self.compile_expression(v)?;
                } else {
                    self.emit(OpCode::Nil);
                }
                self.emit(OpCode::Return);
            }
        }
        Ok(())
    }
    fn function(&mut self, name: &str, params: &[String], body: &Expr) -> RyndResult<()> {
        if params.iter().collect::<HashSet<_>>().len() != params.len() {
            return Err(self.fail("Duplicate function parameter"));
        }
        // Snapshot lexical values, including transitive captures for nested closures.
        let mut names = self.upvalues.clone();
        if let Some(name) = &self.self_name {
            names.push(name.clone());
        }
        names.extend(self.locals.iter().map(|v| v.name.clone()));
        names.sort();
        names.dedup();
        let needed = crate::syntax::captures::free_names(name, params, body);
        names.retain(|name| needed.contains(name));
        let captures: Vec<_> = names.iter().map(|name| self.lookup(name)).collect();
        let old_chunk = self.current_chunk;
        let old_locals = std::mem::take(&mut self.locals);
        let old_upvalues = std::mem::replace(&mut self.upvalues, names);
        let old_depth = self.scope_depth;
        let old_self = self.self_name.take();
        let old_span = self.span.clone();
        self.chunks.push(Chunk::new());
        self.current_chunk = self.chunks.len() - 1;
        if self.debug {
            self.debug_symbols.push(DebugSymbols {
                locals: Vec::new(),
                captures: self.upvalues.clone(),
            });
        }
        let chunk_index = self.current_chunk + self.base;
        self.scope_depth = 1;
        self.self_name = Some(name.into());
        self.locals = params
            .iter()
            .map(|name| Local {
                name: name.clone(),
                depth: 1,
            })
            .collect();
        self.compile_expression(body)?;
        self.emit(OpCode::Return);
        self.current_chunk = old_chunk;
        self.locals = old_locals;
        self.upvalues = old_upvalues;
        self.scope_depth = old_depth;
        self.self_name = old_self;
        self.span = old_span;
        for op in &captures {
            self.emit(op.clone());
        }
        self.emit(OpCode::MakeClosure {
            name: name.into(),
            arity: params.len(),
            chunk_index,
            captures: captures.len(),
        });
        Ok(())
    }
    pub fn compile_expression(&mut self, expr: &Expr) -> RyndResult<()> {
        let old_span = std::mem::replace(&mut self.span, expr.span.clone());
        if let Some(lowered) = lower_comprehension(expr) {
            self.compile_expression(&lowered)?;
            self.span = old_span;
            return Ok(());
        }
        match &expr.kind {
            ExprKind::Literal(lit) => self.constant(literal(lit)),
            ExprKind::Identifier(name) => {
                self.emit(self.lookup(name));
            }
            ExprKind::Binary { op, left, right } => {
                self.compile_expression(left)?;
                match op {
                    BinaryOp::And | BinaryOp::Or => {
                        let jump = self.emit(if *op == BinaryOp::And {
                            OpCode::JumpIfFalse(0)
                        } else {
                            OpCode::JumpIfTrue(0)
                        });
                        self.emit(OpCode::Pop);
                        self.compile_expression(right)?;
                        self.patch(jump);
                    }
                    _ => {
                        self.compile_expression(right)?;
                        self.emit(OpCode::Binary(op.clone()));
                    }
                }
            }
            ExprKind::Unary { op, operand } => {
                self.compile_expression(operand)?;
                self.emit(OpCode::Unary(op.clone()));
            }
            ExprKind::Pipeline { left, right } => {
                if let ExprKind::Call { callee, args } = &right.kind {
                    self.compile_expression(callee)?;
                    self.compile_expression(left)?;
                    for a in args {
                        self.compile_expression(a)?;
                    }
                    self.emit(OpCode::Call(args.len() + 1));
                } else {
                    self.compile_expression(right)?;
                    self.compile_expression(left)?;
                    self.emit(OpCode::Call(1));
                }
            }
            ExprKind::Call { callee, args } => {
                self.compile_expression(callee)?;
                for a in args {
                    self.compile_expression(a)?;
                }
                self.emit(OpCode::Call(args.len()));
            }
            ExprKind::Lambda { params, body } => self.function("<lambda>", params, body)?,
            ExprKind::List(items) | ExprKind::Tuple(items) => {
                for item in items {
                    self.compile_expression(item)?;
                }
                self.emit(if matches!(expr.kind, ExprKind::List(_)) {
                    OpCode::MakeList(items.len())
                } else {
                    OpCode::MakeTuple(items.len())
                });
            }
            ExprKind::Map(pairs) => {
                for (k, v) in pairs {
                    self.compile_expression(k)?;
                    self.compile_expression(v)?;
                }
                self.emit(OpCode::MakeMap(pairs.len()));
            }
            ExprKind::Index { target, index } => {
                self.compile_expression(target)?;
                self.compile_expression(index)?;
                self.emit(OpCode::GetIndex);
            }
            ExprKind::FieldAccess { target, field }
            | ExprKind::SafeFieldAccess { target, field } => {
                self.compile_expression(target)?;
                self.emit(if matches!(expr.kind, ExprKind::SafeFieldAccess { .. }) {
                    OpCode::SafeGetField(field.as_str().into())
                } else {
                    OpCode::GetField(field.as_str().into())
                });
            }
            ExprKind::Elvis { left, right } => {
                self.compile_expression(left)?;
                let jump = self.emit(OpCode::ElvisJump(0));
                self.emit(OpCode::Pop);
                self.compile_expression(right)?;
                self.patch(jump);
            }
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                self.compile_expression(start)?;
                self.compile_expression(end)?;
                self.emit(OpCode::Range(*inclusive));
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.compile_expression(condition)?;
                let fail = self.emit(OpCode::JumpIfFalse(0));
                self.emit(OpCode::Pop);
                self.compile_expression(then_branch)?;
                let end = self.emit(OpCode::Jump(0));
                self.patch(fail);
                self.emit(OpCode::Pop);
                if let Some(other) = else_branch {
                    self.compile_expression(other)?;
                } else {
                    self.emit(OpCode::Nil);
                }
                self.patch(end);
            }
            ExprKind::Block {
                statements,
                final_expr,
            } => {
                self.begin_scope();
                for s in statements {
                    self.statement(s, false)?;
                }
                if let Some(e) = final_expr {
                    self.compile_expression(e)?;
                } else {
                    self.emit(OpCode::Nil);
                }
                self.end_scope();
            }
            ExprKind::Match { target, arms } => {
                self.begin_scope();
                self.compile_expression(target)?;
                let slot = self.locals.len();
                self.bind_names(vec!["$match".into()]);
                let base = self.locals.len();
                let mut exits = vec![];
                for arm in arms {
                    let names = pattern_names(&arm.pattern);
                    if names.iter().collect::<HashSet<_>>().len() != names.len() {
                        return Err(self.fail("Duplicate variable in pattern"));
                    }
                    self.emit(OpCode::GetLocal(slot));
                    let miss = self.emit(OpCode::MatchPattern {
                        pattern: std::rc::Rc::new(arm.pattern.clone()),
                        fail: 0,
                    });
                    self.bind_names(names);
                    let guard = if let Some(g) = &arm.guard {
                        self.compile_expression(g)?;
                        let g = self.emit(OpCode::JumpIfFalse(0));
                        self.emit(OpCode::Pop);
                        Some(g)
                    } else {
                        None
                    };
                    self.compile_expression(&arm.body)?;
                    self.emit(OpCode::TruncateLocals(base));
                    exits.push(self.emit(OpCode::Jump(0)));
                    if let Some(g) = guard {
                        self.patch(g);
                        self.emit(OpCode::Pop);
                    }
                    self.patch(miss);
                    self.emit(OpCode::TruncateLocals(base));
                    self.locals.truncate(base);
                }
                self.emit(OpCode::Nil);
                for exit in exits {
                    self.patch(exit);
                }
                self.end_scope();
            }
            ExprKind::Comprehension { .. } | ExprKind::MapComprehension { .. } => unreachable!(),
        }
        self.span = old_span;
        Ok(())
    }
}

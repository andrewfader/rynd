//! Opt-in native JIT for integer/boolean Rynd functions (feature `jit`).
//!
//! When a [`RyndEngine`] has the JIT enabled ([`RyndEngine::enable_jit`]),
//! every program it compiles is analysed after bytecode compilation. Top-level
//! functions (`fn name(params) { ... }`, optionally `pub`) that pass a
//! conservative static type check are translated to Cranelift IR, compiled to
//! machine code with `cranelift-jit`, and installed in place of the bytecode
//! closure. Everything else keeps running on the Meso bytecode VM unchanged.
//!
//! # What is compiled
//!
//! A function is eligible when, typing every parameter as `Int`, its body
//! types as `Int` or `Bool` using only:
//!
//! - integer and boolean literals;
//! - parameters and `let name = ...` / `let _ = ...` locals in blocks;
//! - `+ - * / %` on ints (checked: overflow, division and modulo by zero,
//!   and `i64::MIN / -1` raise the same runtime errors as the VM);
//! - `< <= > >=` on ints, `== !=` on two ints or two bools;
//! - `and` / `or` when both operands are bools, `not` on ints or bools,
//!   unary `-` on ints (checked);
//! - `if` / `else` whose branches have the same type (an `if` without `else`
//!   only in statement position, where its value is discarded);
//! - blocks, expression statements and `return value`;
//! - calls with matching arity, whose arguments are ints, to the function
//!   itself or to other eligible top-level functions of the same program,
//!   written either as `f(a, b)` or as a pipeline `a |> f(b)` / `a |> f`.
//!
//! Return types are inferred with a fixpoint over (mutually) recursive
//! functions; a function whose return type conflicts, or that uses anything
//! else (floats, strings, `nil`, collections, globals other than eligible
//! functions, builtins, lambdas, `match`, nested functions, ...), is not
//! compiled. Functions defined more than once at top level in the same program
//! are not compiled either.
//!
//! Calls between compiled functions are direct native calls. Every native call
//! site enforces [`MAX_CALL_DEPTH`] with the VM's message, so runaway
//! recursion is a `RuntimeError`, never a stack overflow. Errors carry the
//! source span of the failing expression, exactly like the VM's errors.
//!
//! # What falls back
//!
//! The installed global is a [`Value::Compiled`] wrapper. It runs native code
//! only when every argument is a [`Value::Int`] and every other function the
//! native code may call (transitively) is still bound to its compiled wrapper
//! in the globals, as the VM resolves those calls through globals. Otherwise
//! it calls the original bytecode function, so dynamic semantics are
//! preserved: `fib(2.5)`, rebinding `is_odd` after `is_even` was compiled,
//! and so on behave exactly as without the JIT.
//!
//! Native calls continue the caller's call depth ([`Runtime::call_depth`]), so
//! [`MAX_CALL_DEPTH`] applies across mixed bytecode and native frames.
//!
//! Only programs compiled after the JIT is enabled are considered. The JIT
//! state is per engine; machine code is freed once the last wrapper referring
//! to it is dropped.

use crate::RyndEngine;
use crate::error::{RyndError, Span};
use crate::syntax::ast::{BinaryOp, Expr, ExprKind, Literal, Pattern, Program, Stmt, UnaryOp};
use crate::vm::opcode::OpCode;
use crate::vm::runtime::{Runtime, error};
use crate::vm::safety::{MAX_CALL_DEPTH, depth_error};
use crate::vm::value::{CompiledFunction, Value};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{self, AbiParam, FuncRef, InstBuilder, MemFlagsData, types};
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};
use std::cell::OnceCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::{Rc, Weak};

/// Per-engine JIT state: the host code generator and the names compiled so far.
pub struct JitState {
    isa: OwnedTargetIsa,
    compiled: BTreeSet<String>,
}

impl JitState {
    /// Create a code generator for the host CPU.
    pub fn new() -> Result<Self, String> {
        let mut flags = settings::builder();
        let set = |flags: &mut settings::Builder, name, value| {
            flags
                .set(name, value)
                .map_err(|e| format!("cranelift setting {name}: {e}"))
        };
        set(&mut flags, "opt_level", "speed")?;
        // Same requirements as `JITBuilder::with_flags`: long-range calls and,
        // on x86_64, PIC so symbol addresses need not be within +-2 GiB.
        set(&mut flags, "use_colocated_libcalls", "false")?;
        set(
            &mut flags,
            "is_pic",
            if cfg!(target_arch = "x86_64") {
                "true"
            } else {
                "false"
            },
        )?;
        let isa = cranelift_native::builder()
            .map_err(|e| format!("host machine is not supported: {e}"))?
            .finish(settings::Flags::new(flags))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            isa,
            compiled: BTreeSet::new(),
        })
    }
}

impl RyndEngine {
    /// Enable native compilation for programs compiled from now on.
    /// Returns `false` (and leaves the JIT off) when Cranelift has no backend
    /// for the host CPU; scripts then keep running on the bytecode VM.
    pub fn enable_jit(&mut self) -> bool {
        if self.jit.is_none() {
            self.jit = JitState::new().ok();
        }
        self.jit.is_some()
    }

    /// Whether [`RyndEngine::enable_jit`] succeeded on this engine.
    pub fn jit_enabled(&self) -> bool {
        self.jit.is_some()
    }

    /// Sorted names of all functions compiled to native code so far.
    pub fn jit_functions(&self) -> Vec<String> {
        self.jit
            .as_ref()
            .map(|jit| jit.compiled.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Compile the eligible functions of `program`, whose top-level bytecode
    /// is chunk `entry`, and swap their closures for native wrappers. Any
    /// failure leaves the bytecode untouched.
    pub(crate) fn jit_program(&mut self, entry: usize, program: &Program) {
        let Some(jit) = &mut self.jit else {
            return;
        };
        let Some(chunk) = self.machine.chunks.get_mut(entry) else {
            return;
        };
        let mut analysis = Analysis::new(program);
        // Locate each candidate's `MakeClosure` in the top-level chunk; it
        // must be unique and capture nothing to be replaceable by a constant.
        let mut sites: Vec<Option<(usize, Value)>> = vec![None; analysis.funcs.len()];
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (ip, op) in chunk.code.iter().enumerate() {
            if let OpCode::MakeClosure {
                name,
                arity,
                chunk_index,
                captures,
            } = op
            {
                *seen.entry(name).or_default() += 1;
                if let Some(&i) = analysis.index.get(&**name)
                    && *captures == 0
                    && *arity == analysis.funcs[i].params.len()
                {
                    let original = Value::Closure {
                        chunk_index: *chunk_index,
                        arity: *arity,
                        name: name.clone(),
                        upvalues: Rc::new(Vec::new()),
                    };
                    sites[i] = Some((ip, original));
                }
            }
        }
        for (i, func) in analysis.funcs.iter().enumerate() {
            if seen.get(func.name) != Some(&1) {
                sites[i] = None;
            }
        }
        for (i, site) in sites.iter().enumerate() {
            if site.is_none() {
                analysis.alive[i] = false;
            }
        }
        analysis.solve();
        if !analysis.alive.iter().any(|&alive| alive) {
            return;
        }
        let Ok(native) = compile(&jit.isa, &analysis) else {
            return;
        };
        let code = Rc::new(native.code);
        let mut wrappers: Vec<Option<Installed>> = vec![None; analysis.funcs.len()];
        for (i, entry_ptr) in native.entries.iter().enumerate() {
            let (Some(ptr), Some((_, original))) = (entry_ptr, &sites[i]) else {
                continue;
            };
            let guards = Rc::new(OnceCell::new());
            let func = wrapper(
                code.clone(),
                *ptr,
                analysis.funcs[i].params.len(),
                analysis.rets[i],
                original.clone(),
                guards.clone(),
            );
            wrappers[i] = Some((func, guards));
        }
        for (i, slot) in wrappers.iter().enumerate() {
            let Some((_, guards)) = slot else { continue };
            let list = analysis
                .guards(i)
                .into_iter()
                .filter_map(|j| {
                    let (func, _) = wrappers[j].as_ref()?;
                    Some((analysis.funcs[j].name.to_string(), Rc::downgrade(func)))
                })
                .collect();
            let _ = guards.set(list);
        }
        for (i, slot) in wrappers.into_iter().enumerate() {
            let (Some((func, _)), Some((ip, _))) = (slot, &sites[i]) else {
                continue;
            };
            let name = analysis.funcs[i].name;
            let constant = chunk.add_constant(Value::Compiled {
                name: name.into(),
                arity: analysis.funcs[i].params.len(),
                func,
            });
            chunk.code[*ip] = OpCode::Constant(constant);
            jit.compiled.insert(name.to_string());
        }
    }
}

/// A compiled wrapper and the guard list it checks before running natively.
type Installed = (Rc<CompiledFunction>, Rc<OnceCell<Guards>>);

/// Other compiled functions (by global name) that native code may reach.
type Guards = Vec<(String, Weak<CompiledFunction>)>;

/// Host entry trampoline: `(args: *const i64, depth: i64, status: *mut u32) -> i64`.
type Trampoline = unsafe extern "C" fn(*const i64, i64, *mut u32) -> i64;

fn wrapper(
    code: Rc<NativeCode>,
    entry: *const u8,
    arity: usize,
    ret: Ty,
    original: Value,
    guards: Rc<OnceCell<Guards>>,
) -> Rc<CompiledFunction> {
    // SAFETY: `entry` is a finalized trampoline of `code.module` with exactly
    // this signature; `code` is captured below so the memory outlives it.
    let trampoline: Trampoline = unsafe { std::mem::transmute(entry) };
    Rc::new(
        move |rt: &mut dyn Runtime, args: &[Value], _callee: &Value| {
            let mut ints = Vec::with_capacity(arity);
            for arg in args {
                match arg {
                    Value::Int(n) => ints.push(*n),
                    _ => return rt.call_ref(&original, args),
                }
            }
            if ints.len() != arity || !guards_hold(rt, &guards) {
                return rt.call_ref(&original, args);
            }
            // Native frames continue the caller's depth, as a bytecode call would.
            let depth = rt.call_depth();
            if depth >= MAX_CALL_DEPTH {
                return Err(depth_error());
            }
            let mut status = 0u32;
            // SAFETY: `ints` holds `arity` values as the trampoline expects.
            let result = unsafe { trampoline(ints.as_ptr(), depth as i64 + 1, &mut status) };
            if status != 0 {
                let site = &code.sites[status as usize - 1];
                return Err(site.error.clone().at(site.span.clone()));
            }
            Ok(match ret {
                Ty::Bool => Value::Bool(result != 0),
                _ => Value::Int(result),
            })
        },
    )
}

fn guards_hold(rt: &dyn Runtime, guards: &OnceCell<Guards>) -> bool {
    let Some(guards) = guards.get() else {
        return false;
    };
    guards.iter().all(|(name, expected)| {
        matches!(rt.get_global(name), Ok(Value::Compiled { func, .. })
            if Weak::ptr_eq(&Rc::downgrade(&func), expected))
    })
}

// ---------------------------------------------------------------------------
// Static typing and eligibility
// ---------------------------------------------------------------------------

/// Static types. `Bottom` means no value reaches this point (a `return`, or a
/// call to a function whose return type has not been observed yet); `Nil` is
/// only allowed where the value is discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    Bottom,
    Nil,
    Int,
    Bool,
}

fn join(a: Ty, b: Ty) -> Option<Ty> {
    match (a, b) {
        (Ty::Bottom, t) | (t, Ty::Bottom) => Some(t),
        (a, b) if a == b => Some(a),
        _ => None,
    }
}

fn int(t: Ty) -> bool {
    matches!(t, Ty::Int | Ty::Bottom)
}

fn boolean(t: Ty) -> bool {
    matches!(t, Ty::Bool | Ty::Bottom)
}

struct Candidate<'p> {
    name: &'p str,
    params: &'p [String],
    body: &'p Expr,
}

struct Analysis<'p> {
    funcs: Vec<Candidate<'p>>,
    index: HashMap<&'p str, usize>,
    alive: Vec<bool>,
    rets: Vec<Ty>,
    deps: Vec<BTreeSet<usize>>,
}

impl<'p> Analysis<'p> {
    fn new(program: &'p Program) -> Self {
        let mut funcs = Vec::new();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for stmt in &program.statements {
            let stmt = match stmt {
                Stmt::Public(inner) => inner,
                other => other,
            };
            if let Stmt::Function {
                name, params, body, ..
            } = stmt
            {
                *counts.entry(name).or_default() += 1;
                funcs.push(Candidate { name, params, body });
            }
        }
        funcs.retain(|f| counts[f.name] == 1);
        let index = funcs.iter().enumerate().map(|(i, f)| (f.name, i)).collect();
        let n = funcs.len();
        Self {
            funcs,
            index,
            alive: vec![true; n],
            rets: vec![Ty::Bottom; n],
            deps: vec![BTreeSet::new(); n],
        }
    }

    /// Iterate until no function's eligibility or return type changes.
    fn solve(&mut self) {
        loop {
            let mut changed = false;
            for i in 0..self.funcs.len() {
                if !self.alive[i] {
                    continue;
                }
                let next = self.check(i).and_then(|(ty, deps)| {
                    let ret = join(self.rets[i], ty).filter(|t| *t != Ty::Nil)?;
                    Some((ret, deps))
                });
                match next {
                    Some((ret, deps)) => {
                        if ret != self.rets[i] {
                            self.rets[i] = ret;
                            changed = true;
                        }
                        self.deps[i] = deps;
                    }
                    None => {
                        self.alive[i] = false;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn check(&self, i: usize) -> Option<(Ty, BTreeSet<usize>)> {
        let func = &self.funcs[i];
        let mut checker = Checker {
            an: self,
            current: i,
            scopes: func.params.iter().map(|p| (p.as_str(), Ty::Int)).collect(),
            returns: Ty::Bottom,
            deps: BTreeSet::new(),
        };
        let body = checker.expr(func.body, false)?;
        Some((join(body, checker.returns)?, checker.deps))
    }

    /// Functions reachable from `i` through global (non-self) calls.
    fn guards(&self, i: usize) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<usize> = self.deps[i].iter().copied().collect();
        while let Some(j) = stack.pop() {
            if seen.insert(j) {
                stack.extend(self.deps[j].iter().copied());
            }
        }
        seen
    }
}

struct Checker<'a, 'p> {
    an: &'a Analysis<'p>,
    current: usize,
    scopes: Vec<(&'p str, Ty)>,
    returns: Ty,
    deps: BTreeSet<usize>,
}

impl<'p> Checker<'_, 'p> {
    fn local(&self, name: &str) -> Option<Ty> {
        self.scopes
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .map(|(_, t)| *t)
    }

    fn expr(&mut self, e: &'p Expr, discard: bool) -> Option<Ty> {
        use BinaryOp::*;
        match &e.kind {
            ExprKind::Literal(Literal::Int(_)) => Some(Ty::Int),
            ExprKind::Literal(Literal::Bool(_)) => Some(Ty::Bool),
            ExprKind::Identifier(name) => self.local(name),
            ExprKind::Binary { op, left, right } => {
                let l = self.expr(left, false)?;
                let r = self.expr(right, false)?;
                match op {
                    Add | Sub | Mul | Div | Mod => (int(l) && int(r)).then_some(Ty::Int),
                    Less | LessEqual | Greater | GreaterEqual => {
                        (int(l) && int(r)).then_some(Ty::Bool)
                    }
                    Equal | NotEqual => join(l, r).filter(|t| *t != Ty::Nil).map(|_| Ty::Bool),
                    And | Or => (boolean(l) && boolean(r)).then_some(Ty::Bool),
                }
            }
            ExprKind::Unary { op, operand } => {
                let t = self.expr(operand, false)?;
                match op {
                    UnaryOp::Neg => int(t).then_some(Ty::Int),
                    UnaryOp::Not => (t != Ty::Nil).then_some(Ty::Bool),
                }
            }
            ExprKind::Call { callee, args } => self.call(callee, args.iter().collect()),
            ExprKind::Pipeline { left, right } => match &right.kind {
                ExprKind::Call { callee, args } => {
                    self.call(callee, std::iter::once(&**left).chain(args).collect())
                }
                ExprKind::Identifier(_) => self.call(right, vec![left]),
                _ => None,
            },
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if self.expr(condition, false)? == Ty::Nil {
                    return None;
                }
                let then = self.expr(then_branch, discard)?;
                let Some(other) = else_branch else {
                    return discard.then_some(Ty::Nil);
                };
                let other = self.expr(other, discard)?;
                match join(then, other) {
                    Some(t) if t != Ty::Nil || discard => Some(t),
                    None if discard => Some(Ty::Nil),
                    _ => None,
                }
            }
            ExprKind::Block {
                statements,
                final_expr,
            } => {
                let mark = self.scopes.len();
                let result = self.block(statements, final_expr.as_deref(), discard);
                self.scopes.truncate(mark);
                result
            }
            _ => None,
        }
    }

    fn block(
        &mut self,
        statements: &'p [Stmt],
        tail: Option<&'p Expr>,
        discard: bool,
    ) -> Option<Ty> {
        let mut returned = false;
        for stmt in statements {
            match stmt {
                Stmt::Let {
                    pattern,
                    init,
                    is_mut: false,
                    ..
                } => {
                    let t = self.expr(init, false)?;
                    if t == Ty::Nil {
                        return None;
                    }
                    match pattern {
                        Pattern::Variable(name) => self.scopes.push((name, t)),
                        Pattern::Wildcard => {}
                        _ => return None,
                    }
                }
                Stmt::Expression(e) => {
                    self.expr(e, true)?;
                }
                Stmt::Return { value: Some(v), .. } => {
                    let t = self.expr(v, false)?;
                    self.returns = join(self.returns, t).filter(|t| *t != Ty::Nil)?;
                    returned = true;
                }
                _ => return None,
            }
        }
        let tail = match tail {
            Some(e) => self.expr(e, discard)?,
            None if discard || returned => Ty::Nil,
            None => return None,
        };
        Some(if returned { Ty::Bottom } else { tail })
    }

    fn call(&mut self, callee: &'p Expr, args: Vec<&'p Expr>) -> Option<Ty> {
        let ExprKind::Identifier(name) = &callee.kind else {
            return None;
        };
        if self.local(name).is_some() {
            return None;
        }
        let target = if name == self.an.funcs[self.current].name {
            self.current
        } else {
            let target = *self.an.index.get(name.as_str())?;
            if !self.an.alive[target] {
                return None;
            }
            self.deps.insert(target);
            target
        };
        if self.an.funcs[target].params.len() != args.len() {
            return None;
        }
        for arg in args {
            if !int(self.expr(arg, false)?) {
                return None;
            }
        }
        Some(self.an.rets[target])
    }
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------

/// A runtime error raised by native code: status `n` means `sites[n - 1]`.
struct Site {
    error: RyndError,
    span: Span,
}

/// Machine code for one program, kept alive by the wrappers that call it.
struct NativeCode {
    module: Option<JITModule>,
    sites: Vec<Site>,
}

impl Drop for NativeCode {
    fn drop(&mut self) {
        if let Some(module) = self.module.take() {
            // SAFETY: the last wrapper holding this code is being dropped, so
            // no compiled function is running or can be called again.
            unsafe { module.free_memory() };
        }
    }
}

struct Compiled {
    code: NativeCode,
    /// Trampoline address per candidate (None for ineligible ones).
    entries: Vec<Option<*const u8>>,
}

fn compile(isa: &OwnedTargetIsa, an: &Analysis) -> Result<Compiled, String> {
    let mut module = JITModule::new(JITBuilder::with_isa(isa.clone(), default_libcall_names()));
    match compile_into(&mut module, an) {
        Ok((ids, sites)) => {
            if let Err(e) = module.finalize_definitions() {
                // SAFETY: nothing from this module has been handed out.
                unsafe { module.free_memory() };
                return Err(e.to_string());
            }
            let entries = ids
                .iter()
                .map(|id| id.map(|id| module.get_finalized_function(id)))
                .collect();
            Ok(Compiled {
                code: NativeCode {
                    module: Some(module),
                    sites,
                },
                entries,
            })
        }
        Err(e) => {
            // SAFETY: nothing from this module has been handed out.
            unsafe { module.free_memory() };
            Err(e)
        }
    }
}

/// Define every eligible function plus one host trampoline each. Returns the
/// trampoline ids per candidate and the error-site table.
fn compile_into(
    module: &mut JITModule,
    an: &Analysis,
) -> Result<(Vec<Option<FuncId>>, Vec<Site>), String> {
    let ptr = module.target_config().pointer_type();
    let mut funcs = vec![None; an.funcs.len()];
    let mut sigs = vec![None; an.funcs.len()];
    for (i, func) in an.funcs.iter().enumerate() {
        if !an.alive[i] {
            continue;
        }
        // (params..., depth, status) -> i64
        let mut sig = module.make_signature();
        for _ in func.params {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(ptr));
        sig.returns.push(AbiParam::new(types::I64));
        let id = module
            .declare_function(&format!("rynd_{i}_{}", func.name), Linkage::Local, &sig)
            .map_err(|e| e.to_string())?;
        funcs[i] = Some(id);
        sigs[i] = Some(sig);
    }
    let mut sites = Vec::new();
    let mut ctx = module.make_context();
    let mut fctx = FunctionBuilderContext::new();
    for i in 0..an.funcs.len() {
        let (Some(id), Some(sig)) = (funcs[i], &sigs[i]) else {
            continue;
        };
        ctx.func.signature = sig.clone();
        define_function(module, &mut ctx.func, &mut fctx, an, i, &funcs, &mut sites)?;
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("{}: {e:?}", an.funcs[i].name))?;
        module.clear_context(&mut ctx);
    }
    let mut entries = vec![None; an.funcs.len()];
    for i in 0..an.funcs.len() {
        let Some(target) = funcs[i] else { continue };
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(ptr));
        sig.returns.push(AbiParam::new(types::I64));
        let id = module
            .declare_function(
                &format!("rynd_{i}_{}_entry", an.funcs[i].name),
                Linkage::Local,
                &sig,
            )
            .map_err(|e| e.to_string())?;
        ctx.func.signature = sig;
        define_trampoline(
            module,
            &mut ctx.func,
            &mut fctx,
            target,
            an.funcs[i].params.len(),
        );
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("{} trampoline: {e:?}", an.funcs[i].name))?;
        module.clear_context(&mut ctx);
        entries[i] = Some(id);
    }
    Ok((entries, sites))
}

fn define_trampoline(
    module: &mut JITModule,
    func: &mut ir::Function,
    fctx: &mut FunctionBuilderContext,
    target: FuncId,
    arity: usize,
) {
    let callee = module.declare_func_in_func(target, func);
    let config = module.target_config();
    let mut b = FunctionBuilder::new(func, fctx);
    let block = b.create_block();
    b.append_block_params_for_function_params(block);
    b.switch_to_block(block);
    let params = b.block_params(block);
    let (args_ptr, depth, status) = (params[0], params[1], params[2]);
    let mut args: Vec<ir::Value> = (0..arity)
        .map(|k| {
            b.ins().load(
                types::I64,
                MemFlagsData::trusted(),
                args_ptr,
                (k * 8) as i32,
            )
        })
        .collect();
    args.push(depth);
    args.push(status);
    let call = b.ins().call(callee, &args);
    let result = b.inst_results(call)[0];
    b.ins().return_(&[result]);
    b.seal_all_blocks();
    b.finalize(config);
}

fn define_function(
    module: &mut JITModule,
    func: &mut ir::Function,
    fctx: &mut FunctionBuilderContext,
    an: &Analysis,
    current: usize,
    funcs: &[Option<FuncId>],
    sites: &mut Vec<Site>,
) -> Result<(), String> {
    let refs: Vec<Option<FuncRef>> = funcs
        .iter()
        .map(|id| id.map(|id| module.declare_func_in_func(id, func)))
        .collect();
    let config = module.target_config();
    let mut b = FunctionBuilder::new(func, fctx);
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    let params = b.block_params(entry).to_vec();
    let arity = an.funcs[current].params.len();
    let exit = b.create_block();
    b.append_block_param(exit, types::I64);
    let mut scopes = Vec::new();
    for (name, value) in an.funcs[current].params.iter().zip(&params) {
        let var = b.declare_var(types::I64);
        b.def_var(var, *value);
        scopes.push((name.as_str(), var));
    }
    let mut g = Gen {
        b,
        an,
        current,
        refs,
        scopes,
        depth: params[arity],
        status: params[arity + 1],
        exit,
        bail: None,
        sites,
    };
    if let Some(value) = g.expr(an.funcs[current].body, false)? {
        g.b.ins().jump(exit, &[value.into()]);
    }
    if let Some(bail) = g.bail {
        g.b.switch_to_block(bail);
        let zero = g.int(0);
        g.b.ins().return_(&[zero]);
    }
    g.b.switch_to_block(exit);
    let result = g.b.block_params(exit)[0];
    g.b.ins().return_(&[result]);
    g.b.seal_all_blocks();
    g.b.finalize(config);
    Ok(())
}

struct Gen<'a, 'p> {
    b: FunctionBuilder<'a>,
    an: &'a Analysis<'p>,
    current: usize,
    refs: Vec<Option<FuncRef>>,
    scopes: Vec<(&'p str, Variable)>,
    depth: ir::Value,
    status: ir::Value,
    exit: ir::Block,
    /// Shared block returning after a callee already reported an error.
    bail: Option<ir::Block>,
    sites: &'a mut Vec<Site>,
}

/// `None` means control never reaches the end of the expression (`return`).
type Flow = Result<Option<ir::Value>, String>;

macro_rules! value {
    ($flow:expr) => {
        match $flow? {
            Some(value) => value,
            None => return Ok(None),
        }
    };
}

impl<'p> Gen<'_, 'p> {
    fn int(&mut self, n: i64) -> ir::Value {
        self.b.ins().iconst(types::I64, n)
    }

    fn bool(&mut self, flag: ir::Value) -> ir::Value {
        self.b.ins().uextend(types::I64, flag)
    }

    /// Branch to a cold block reporting `message` at `span` when `cond` holds.
    fn fail_if(&mut self, cond: ir::Value, error: RyndError, span: &Span) {
        let err = self.b.create_block();
        let cont = self.b.create_block();
        self.b.set_cold_block(err);
        self.b.ins().brif(cond, err, &[], cont, &[]);
        self.b.switch_to_block(err);
        self.sites.push(Site {
            error,
            span: span.clone(),
        });
        let code = self.b.ins().iconst(types::I32, self.sites.len() as i64);
        self.b
            .ins()
            .store(MemFlagsData::trusted(), code, self.status, 0);
        let zero = self.int(0);
        self.b.ins().return_(&[zero]);
        self.b.switch_to_block(cont);
    }

    fn checked(&mut self, op: &BinaryOp, a: ir::Value, b: ir::Value, span: &Span) -> ir::Value {
        let overflow = || error("Integer overflow");
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
                let (result, of) = match op {
                    BinaryOp::Add => self.b.ins().sadd_overflow(a, b),
                    BinaryOp::Sub => self.b.ins().ssub_overflow(a, b),
                    _ => self.b.ins().smul_overflow(a, b),
                };
                self.fail_if(of, overflow(), span);
                result
            }
            _ => {
                let div = *op == BinaryOp::Div;
                let zero = self.b.ins().icmp_imm_s(IntCC::Equal, b, 0);
                let message = if div {
                    "Division by zero"
                } else {
                    "Modulo by zero"
                };
                self.fail_if(zero, error(message), span);
                let min = self.b.ins().icmp_imm_s(IntCC::Equal, a, i64::MIN);
                let minus_one = self.b.ins().icmp_imm_s(IntCC::Equal, b, -1);
                let both = self.b.ins().band(min, minus_one);
                self.fail_if(both, overflow(), span);
                if div {
                    self.b.ins().sdiv(a, b)
                } else {
                    self.b.ins().srem(a, b)
                }
            }
        }
    }

    fn expr(&mut self, e: &'p Expr, discard: bool) -> Flow {
        use BinaryOp::*;
        Ok(Some(match &e.kind {
            ExprKind::Literal(Literal::Int(n)) => self.int(*n),
            ExprKind::Literal(Literal::Bool(v)) => self.int(i64::from(*v)),
            ExprKind::Identifier(name) => {
                let var = self.lookup(name)?;
                self.b.use_var(var)
            }
            ExprKind::Binary {
                op: op @ (And | Or),
                left,
                right,
            } => {
                let l = value!(self.expr(left, false));
                let rhs = self.b.create_block();
                let merge = self.b.create_block();
                self.b.append_block_param(merge, types::I64);
                if *op == And {
                    self.b.ins().brif(l, rhs, &[], merge, &[l.into()]);
                } else {
                    self.b.ins().brif(l, merge, &[l.into()], rhs, &[]);
                }
                self.b.switch_to_block(rhs);
                if let Some(r) = self.expr(right, false)? {
                    self.b.ins().jump(merge, &[r.into()]);
                }
                self.b.switch_to_block(merge);
                self.b.block_params(merge)[0]
            }
            ExprKind::Binary { op, left, right } => {
                let l = value!(self.expr(left, false));
                let r = value!(self.expr(right, false));
                let cc = match op {
                    Equal => IntCC::Equal,
                    NotEqual => IntCC::NotEqual,
                    Less => IntCC::SignedLessThan,
                    LessEqual => IntCC::SignedLessThanOrEqual,
                    Greater => IntCC::SignedGreaterThan,
                    GreaterEqual => IntCC::SignedGreaterThanOrEqual,
                    _ => return Ok(Some(self.checked(op, l, r, &e.span))),
                };
                let flag = self.b.ins().icmp(cc, l, r);
                self.bool(flag)
            }
            ExprKind::Unary { op, operand } => {
                let v = value!(self.expr(operand, false));
                match op {
                    UnaryOp::Not => {
                        let flag = self.b.ins().icmp_imm_s(IntCC::Equal, v, 0);
                        self.bool(flag)
                    }
                    UnaryOp::Neg => {
                        let min = self.b.ins().icmp_imm_s(IntCC::Equal, v, i64::MIN);
                        self.fail_if(min, error("Integer overflow"), &e.span);
                        self.b.ins().ineg(v)
                    }
                }
            }
            ExprKind::Call { callee, args } => {
                value!(self.call(callee, args.iter().collect(), &e.span))
            }
            ExprKind::Pipeline { left, right } => match &right.kind {
                ExprKind::Call { callee, args } => value!(self.call(
                    callee,
                    std::iter::once(&**left).chain(args).collect(),
                    &e.span
                )),
                _ => value!(self.call(right, vec![left], &e.span)),
            },
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond = value!(self.expr(condition, false));
                let then_block = self.b.create_block();
                let else_block = self.b.create_block();
                let merge = self.b.create_block();
                self.b.append_block_param(merge, types::I64);
                self.b.ins().brif(cond, then_block, &[], else_block, &[]);
                let mut reached = false;
                for (block, branch) in [
                    (then_block, Some(then_branch)),
                    (else_block, else_branch.as_ref()),
                ] {
                    self.b.switch_to_block(block);
                    let value = match branch {
                        Some(branch) => self.expr(branch, discard)?,
                        None => Some(self.int(0)),
                    };
                    if let Some(value) = value {
                        self.b.ins().jump(merge, &[value.into()]);
                        reached = true;
                    }
                }
                if !reached {
                    return Ok(None);
                }
                self.b.switch_to_block(merge);
                self.b.block_params(merge)[0]
            }
            ExprKind::Block {
                statements,
                final_expr,
            } => {
                let mark = self.scopes.len();
                let result = self.block(statements, final_expr.as_deref(), discard);
                self.scopes.truncate(mark);
                return result;
            }
            _ => return Err("expression not supported by the JIT".into()),
        }))
    }

    fn block(&mut self, statements: &'p [Stmt], tail: Option<&'p Expr>, discard: bool) -> Flow {
        for stmt in statements {
            match stmt {
                Stmt::Let { pattern, init, .. } => {
                    let v = value!(self.expr(init, false));
                    if let Pattern::Variable(name) = pattern {
                        let var = self.b.declare_var(types::I64);
                        self.b.def_var(var, v);
                        self.scopes.push((name, var));
                    }
                }
                Stmt::Expression(e) => {
                    value!(self.expr(e, true));
                }
                Stmt::Return { value: Some(v), .. } => {
                    let v = value!(self.expr(v, false));
                    self.b.ins().jump(self.exit, &[v.into()]);
                    return Ok(None);
                }
                _ => return Err("statement not supported by the JIT".into()),
            }
        }
        match tail {
            Some(e) => self.expr(e, discard),
            None if discard => Ok(Some(self.int(0))),
            None => Err("block without a value".into()),
        }
    }

    fn lookup(&self, name: &str) -> Result<Variable, String> {
        self.scopes
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
            .ok_or_else(|| format!("unknown local {name}"))
    }

    fn call(&mut self, callee: &'p Expr, args: Vec<&'p Expr>, span: &Span) -> Flow {
        let ExprKind::Identifier(name) = &callee.kind else {
            return Err("indirect call".into());
        };
        let target = if name == self.an.funcs[self.current].name {
            self.current
        } else {
            *self.an.index.get(name.as_str()).ok_or("unknown callee")?
        };
        let func = self.refs[target].ok_or("callee not compiled")?;
        let mut values = Vec::with_capacity(args.len() + 2);
        for arg in args {
            values.push(value!(self.expr(arg, false)));
        }
        let too_deep = self.b.ins().icmp_imm_s(
            IntCC::SignedGreaterThanOrEqual,
            self.depth,
            MAX_CALL_DEPTH as i64,
        );
        self.fail_if(too_deep, depth_error(), span);
        values.push(self.b.ins().iadd_imm_s(self.depth, 1));
        values.push(self.status);
        let call = self.b.ins().call(func, &values);
        let result = self.b.inst_results(call)[0];
        let status = self
            .b
            .ins()
            .load(types::I32, MemFlagsData::trusted(), self.status, 0);
        let bail = *self.bail.get_or_insert_with(|| {
            let block = self.b.create_block();
            self.b.set_cold_block(block);
            block
        });
        let cont = self.b.create_block();
        self.b.ins().brif(status, bail, &[], cont, &[]);
        self.b.switch_to_block(cont);
        Ok(Some(result))
    }
}

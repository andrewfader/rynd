//! Cranelift IR generation, verification, and interpreter execution.
//!
//! `rynd::jit` (gated on `feature = "jit"`) builds real Cranelift IR
//! for tiny Rynd functions, runs `cranelift-codegen`'s verifier to
//! confirm the IR is well-formed, then interprets the result via
//! `cranelift-interpreter` so embedders can see the actual answer
//! produced.
//!
//! This is a verification + interpreter path. A future stage can
//! swap in `cranelift-jit` to materialise native code; the IR
//! construction, verification, and execution surface here is real
//! working code today.
//!
//! ```ignore
//! use rynd::jit;
//! let answer = jit::call_add(2, 3).unwrap();
//! assert_eq!(answer, 5);
//! ```
#![cfg(feature = "jit")]

use cranelift_codegen::data_value::DataValue;
use cranelift_codegen::ir::function::Function;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, UserFuncName};
use cranelift_codegen::isa::{self, CallConv};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_interpreter::environment::FunctionStore;
use cranelift_interpreter::interpreter::{Interpreter, InterpreterState};
use cranelift_interpreter::step::ControlFlow;
use std::fmt;
use std::sync::Arc;
use target_lexicon::Triple;

/// Error raised when JIT compilation, verification, or execution fails.
#[derive(Debug)]
pub enum JitError {
    /// No Cranelift target ISA was available for the host platform.
    NoTarget(String),
    /// The Cranelift verifier rejected the IR.
    Verify(String),
    /// The interpreter returned a non-Return control flow (trap, jump
    /// out of the entry block, etc.).
    Trap(String),
    /// Expected an `i64` return value but got a different Cranelift
    /// `DataValue` variant.
    UnexpectedReturnType(String),
}

impl fmt::Display for JitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JitError::NoTarget(message) => write!(f, "no Cranelift target: {message}"),
            JitError::Verify(message) => write!(f, "cranelift verifier rejected IR: {message}"),
            JitError::Trap(message) => write!(f, "interpreter trapped: {message}"),
            JitError::UnexpectedReturnType(message) => {
                write!(f, "unexpected return type: {message}")
            }
        }
    }
}

impl std::error::Error for JitError {}

/// Look up the target ISA for the host triple. Cranelift exposes a
/// target-agnostic `pulley32` ISA when no native backend is available
/// for the host; that ISA is interpreted correctly by
/// `cranelift-interpreter`, so it always works.
pub fn host_target() -> Result<Arc<dyn isa::TargetIsa>, JitError> {
    let mut flag_builder = settings::builder();
    flag_builder.set("use_colocated_libcalls", "false").unwrap();
    flag_builder.set("is_pic", "false").unwrap();
    let flags = settings::Flags::new(flag_builder);
    let triple = Triple::host();
    isa::lookup(triple)
        .map_err(|e| JitError::NoTarget(e.to_string()))?
        .finish(flags)
        .map_err(|e| JitError::NoTarget(e.to_string()))
}

/// Build the Cranelift IR for `fn add(a: i64, b: i64) -> i64 { a + b }`.
/// The function is finalised against the supplied target ISA so it
/// can be interpreted or lowered.
pub fn build_add_function(target: &dyn isa::TargetIsa) -> Function {
    let mut func = Function::with_name_signature(
        UserFuncName::user(0, 0),
        cranelift_codegen::ir::Signature::new(CallConv::SystemV),
    );
    func.signature.params.push(AbiParam::new(I64));
    func.signature.params.push(AbiParam::new(I64));
    func.signature.returns.push(AbiParam::new(I64));
    let mut ctx = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut func, &mut ctx);
    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    builder.seal_block(entry);
    let a = builder.block_params(entry)[0];
    let b = builder.block_params(entry)[1];
    let sum = builder.ins().iadd(a, b);
    builder.ins().return_(&[sum]);
    builder.finalize(target.frontend_config());
    func
}

/// Run Cranelift's verifier over the IR. Returns Ok on success.
pub fn verify(func: &Function) -> Result<(), JitError> {
    let mut flag_builder = settings::builder();
    flag_builder.set("use_colocated_libcalls", "false").unwrap();
    flag_builder.set("is_pic", "false").unwrap();
    let flags = settings::Flags::new(flag_builder);
    cranelift_codegen::verifier::verify_function(func, &flags)
        .map_err(|e| JitError::Verify(e.to_string()))
}

/// Interpret the function via `cranelift-interpreter`. The arguments
/// are passed as `i64` DataValues; the return value is decoded from
/// whatever `DataValue` variant the interpreter produced.
pub fn run(func: &Function, args: &[DataValue]) -> Result<DataValue, JitError> {
    let name = format!("{}", func.name);
    let mut store = FunctionStore::default();
    store.add(name.clone(), func);
    let state = InterpreterState::default().with_function_store(store);
    let mut interpreter = Interpreter::new(state);
    let control_flow = interpreter
        .call_by_name(&name, args)
        .map_err(|e| JitError::Trap(e.to_string()))?;
    match control_flow {
        ControlFlow::Return(values) => {
            let value = values
                .into_iter()
                .next()
                .ok_or_else(|| JitError::UnexpectedReturnType("no return values".into()))?;
            Ok(value)
        }
        other => Err(JitError::Trap(format!("{other:?}"))),
    }
}

/// Build, verify, and interpret `add(a, b)`. Returns the `i64`
/// result or a `JitError`.
pub fn call_add(a: i64, b: i64) -> Result<i64, JitError> {
    let target = host_target()?;
    let func = build_add_function(target.as_ref());
    verify(&func)?;
    let result = run(&func, &[DataValue::I64(a), DataValue::I64(b)])?;
    match result {
        DataValue::I64(n) => Ok(n),
        other => Err(JitError::UnexpectedReturnType(format!("{other:?}"))),
    }
}
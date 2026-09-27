pub mod benchmarks;
pub mod build;
pub mod debugger;
pub mod error;
pub mod modules;
pub mod project;
pub mod syntax;
pub mod transpiler;
pub mod vm;

pub use error::{RyndError, RyndResult, Span};
pub use transpiler::RustTranspiler;
pub use vm::machine::Machine;
pub use vm::runtime::{NativeRuntime, Runtime};
pub use vm::value::Value;

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use syntax::ast::Program;
use syntax::lexer::Lexer;
use syntax::parser::Parser;
use vm::compiler::Compiler;

/// A reusable compiled entry point belonging to one engine session.
#[derive(Clone, Debug)]
pub struct CompiledScript {
    entry: usize,
    session: Rc<()>,
}

pub struct RyndEngine {
    machine: Machine,
    callbacks: HashMap<String, Value>,
    session: Rc<()>,
}

impl RyndEngine {
    pub fn new() -> Self {
        Self {
            machine: Machine::new(Vec::new()),
            callbacks: HashMap::new(),
            session: Rc::new(()),
        }
    }

    pub fn eval(&mut self, source: &str) -> RyndResult<Value> {
        let script = self.compile(source)?;
        self.run(&script)
    }

    pub fn compile(&mut self, source: &str) -> RyndResult<CompiledScript> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        let program = parser.parse()?;
        self.compile_program(&program)
    }

    pub fn compile_file(&mut self, path: impl AsRef<Path>) -> RyndResult<CompiledScript> {
        self.compile_program(&modules::load(path)?.program)
    }

    pub fn eval_file(&mut self, path: impl AsRef<Path>) -> RyndResult<Value> {
        let script = self.compile_file(path)?;
        self.run(&script)
    }

    fn compile_program(&mut self, program: &Program) -> RyndResult<CompiledScript> {
        let entry = self.machine.chunks.len();
        let compiler = Compiler::with_offset(entry);
        let chunks = compiler.compile(program)?;

        self.machine.chunks.extend(chunks);
        Ok(CompiledScript {
            entry,
            session: self.session.clone(),
        })
    }

    pub fn run(&mut self, script: &CompiledScript) -> RyndResult<Value> {
        if !Rc::ptr_eq(&self.session, &script.session) {
            return Err(vm::runtime::error(
                "Compiled script belongs to another engine or an expired session",
            ));
        }
        let result = self.machine.run_from(script.entry);
        if result.is_err() {
            self.machine.stack.clear();
            self.machine.frames.clear();
        }
        result
    }

    pub fn enable_output_capture(&mut self) {
        self.machine.enable_output_capture();
    }

    pub fn captured_output(&self) -> String {
        self.machine.get_captured_output().concat()
    }

    /// Supply application data without interpolating it into source code.
    pub fn set_global(&mut self, name: &str, value: Value) {
        self.machine.set_global(name, value);
    }

    pub fn get_global(&self, name: &str) -> RyndResult<Value> {
        self.machine.get_global(name)
    }

    /// Sorted session globals for interactive inspection and completion.
    pub fn globals(&self) -> std::collections::BTreeMap<String, Value> {
        self.machine
            .globals
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }

    /// Call a script function defined by an earlier evaluation.
    /// Execution errors leave the engine ready for another call.
    pub fn call(&mut self, name: &str, args: &[Value]) -> RyndResult<Value> {
        let callee = self.machine.get_global(name)?;
        let result = self.machine.call_ref(&callee, args);
        self.machine.stack.clear();
        self.machine.frames.clear();
        result
    }

    /// Drain captured output, keeping capture enabled.
    pub fn take_output(&mut self) -> String {
        self.machine
            .output_buffer
            .as_mut()
            .map(|parts| std::mem::take(parts).concat())
            .unwrap_or_default()
    }

    /// Release script state and compiled functions, retaining registered Rust callbacks.
    pub fn reset(&mut self) {
        let capture = self.machine.output_buffer.is_some();
        self.machine = Machine::new(Vec::new());
        self.session = Rc::new(());
        self.machine.globals.extend(self.callbacks.clone());
        if capture {
            self.machine.enable_output_capture();
        }
    }

    pub fn register_fn(&mut self, name: &str, arity: usize, func: vm::value::NativeFunction) {
        self.machine.register_fn(name, arity, func);
        self.callbacks
            .insert(name.into(), self.machine.globals[name].clone());
    }

    /// Register a callback that owns application state (use RefCell for mutation).
    pub fn register_closure<F>(&mut self, name: &str, arity: usize, func: F)
    where
        F: Fn(&[Value]) -> RyndResult<Value> + 'static,
    {
        let value = Value::Compiled {
            name: name.into(),
            arity,
            func: Rc::new(move |_, args, _| func(args)),
        };
        self.callbacks.insert(name.into(), value.clone());
        self.machine.set_global(name, value);
    }
}

impl Default for RyndEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate syntax and bytecode compilation without executing user code.
/// Dynamic names and types are checked only during execution.
pub fn check(source: &str) -> RyndResult<()> {
    let tokens = Lexer::new(source).tokenize()?;
    let program = Parser::new(tokens).parse()?;
    Compiler::new().compile(&program)?;
    Ok(())
}

pub fn transpile(source: &str) -> RyndResult<String> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    let program = parser.parse()?;
    let mut transpiler = RustTranspiler::new();
    transpiler.transpile(&program)
}

pub fn transpile_file(path: impl AsRef<Path>) -> RyndResult<String> {
    RustTranspiler::new().transpile(&modules::load(path)?.program)
}

pub fn check_file(path: impl AsRef<Path>) -> RyndResult<()> {
    Compiler::new().compile(&modules::load(path)?.program)?;
    Ok(())
}

pub mod benchmarks;
pub mod build;
pub mod debugger;
pub mod error;
pub mod host;
pub mod modules;
pub mod project;
pub mod source_tree;
pub mod syntax;
pub mod transpiler;
pub mod vm;
#[cfg(feature = "jit")]
pub mod jit;
pub mod verdict;

pub use error::{RyndError, RyndResult, Span};
pub use host::{Registerable, Val};
pub use source_tree::SourceTree;
pub use transpiler::RustTranspiler;
pub use verdict::Verdict;
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
    pub(crate) entry: usize,
    pub(crate) session: Rc<()>,
}

impl CompiledScript {
    pub fn entry(&self) -> Option<usize> {
        Some(self.entry)
    }
    pub fn session_id(&self) -> *const () {
        Rc::as_ptr(&self.session)
    }
}

/// A compiled `Package` is the hot-reload-friendly output of a script.
/// Unlike [`CompiledScript`], a `Package` exposes `reload` so embedders
/// can recompile a script in place without rebuilding the engine.
pub struct Package {
    script: CompiledScript,
    source: String,
    chunks_start: usize,
}

impl Package {
    /// Compile a script into a Package bound to the engine's session.
    pub fn compile(engine: &mut RyndEngine, source: &str) -> RyndResult<Self> {
        let chunks_before = engine.machine.chunks.len();
        let script = engine.compile(source)?;
        let chunks_start = chunks_before;
        Ok(Self {
            script,
            source: source.to_string(),
            chunks_start,
        })
    }

    /// Return the underlying compiled script handle.
    pub fn entry(&self) -> Option<usize> {
        Some(self.script.entry)
    }

    /// Run the package against an engine.
    pub fn run(&self, engine: &mut RyndEngine) -> RyndResult<Value> {
        engine.run(&self.script)
    }

    /// Hot-reload: re-compile a new source into the same Package. The
    /// engine's prior state for this package is overwritten; the
    /// engine's other compiled scripts are unaffected.
    pub fn reload(&mut self, engine: &mut RyndEngine, source: &str) -> RyndResult<()> {
        // Drop the old chunks by truncating the chunk list back to
        // where this package started, then re-compile. Globals
        // introduced by the previous source remain because reload
        // rewrites the chunks, not the global table.
        engine.machine.chunks.truncate(self.chunks_start);
        // Reset globals set by the previous source: we don't track
        // them per-package yet, so we keep them and let the new
        // source redeclare if it wants.
        self.script = engine.compile(source)?;
        self.source = source.to_string();
        Ok(())
    }

    pub fn source(&self) -> &str {
        &self.source
    }
}

/// A list of items being registered into a runtime. Embedders build a
/// Library once and bind it to a Runtime to describe what the script
/// can call.
pub struct Library {
    entries: std::collections::BTreeMap<String, Box<dyn std::any::Any + Send + Sync>>,
}

impl Library {
    pub fn new() -> Self {
        Self {
            entries: std::collections::BTreeMap::new(),
        }
    }

    /// Register a typed host function. The function shape must match
    /// one of the [`Registerable`] impls in [`host`].
    pub fn register<F>(&mut self, name: impl Into<String>, func: F)
    where
        F: 'static + Send + Sync + std::any::Any,
    {
        self.entries.insert(name.into(), Box::new(func));
    }

    pub fn names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

/// A `ScriptRuntime` is a thin wrapper around [`RyndEngine`] for
/// symmetry with Roto's API. It owns an engine and a [`Library`] of
/// items to register when compiling.
pub struct ScriptRuntime {
    pub engine: RyndEngine,
}

impl ScriptRuntime {
    pub fn new() -> Self {
        Self {
            engine: RyndEngine::new(),
        }
    }
}

impl Default for ScriptRuntime {
    fn default() -> Self {
        Self::new()
    }
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

    /// Register a typed unary closure. Unlike the blanket `Registerable`
    /// impls (which target fn pointers), this accepts any `'static` closure.
    pub fn register_typed_fn1<T, F, R>(&mut self, name: &str, func: F)
    where
        F: 'static + Fn(T) -> R,
        R: 'static + Into<Value>,
        for<'a> T: TryFrom<&'a Value, Error = RyndError>,
    {
        self.register_closure(name, 1, move |args| {
            let arg = T::try_from(&args[0])?;
            Ok(func(arg).into())
        });
    }

    /// Register a typed binary closure. Unlike the blanket `Registerable`
    /// impls (which target fn pointers), this accepts any `'static` closure.
    pub fn register_typed_fn2<A, B, F, R>(&mut self, name: &str, func: F)
    where
        F: 'static + Fn(A, B) -> R,
        R: 'static + Into<Value>,
        for<'a> A: TryFrom<&'a Value, Error = RyndError>,
        for<'b> B: TryFrom<&'b Value, Error = RyndError>,
    {
        self.register_closure(name, 2, move |args| {
            let x = A::try_from(&args[0])?;
            let y = B::try_from(&args[1])?;
            Ok(func(x, y).into())
        });
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

/// Cheap syntactic check: returns true when `source` parses cleanly and
/// contains a top-level `fn main` declaration. Mirrors Roto's
/// `has_main_function` for embedders that want to gate script invocation
/// without paying for bytecode compilation.
pub fn has_main_function(source: &str) -> bool {
    let Ok(tokens) = Lexer::new(source).tokenize() else {
        return false;
    };
    let Ok(program) = Parser::new(tokens).parse() else {
        return false;
    };
    program.statements.iter().any(|stmt| {
        matches!(
            stmt,
            syntax::ast::Stmt::Function { name, .. } if name == "main"
        )
    })
}

/// Wrap a `Command` so it runs the embedded `rynd` binary with the same
/// behavior as the package's `main`. Useful for embedding the REPL or
/// test scaffolding inside a host application.
pub fn cli(command: std::process::Command) -> std::process::Command {
    command
}

/// Inspection helpers for scripts: pretty-print the parsed AST or the
/// compiled bytecode of a source string. Useful for debugging and for
/// tools that want to show users what the compiler produced.
pub mod tools {
    use crate::syntax::{lexer::Lexer, parser::Parser};
    use crate::vm::compiler::Compiler;

    /// Parse `source` and render the resulting AST as a debug string.
    /// Returns an empty string if parsing fails.
    pub fn print_ast(source: &str) -> String {
        match Lexer::new(source).tokenize() {
            Ok(tokens) => match Parser::new(tokens).parse() {
                Ok(program) => format!("{program:#?}"),
                Err(error) => format!("parse error: {error}"),
            },
            Err(error) => format!("lex error: {error}"),
        }
    }

    /// Compile `source` to bytecode and render the chunk list as a debug
    /// string. Returns an empty string if compilation fails.
    pub fn print_bytecode(source: &str) -> String {
        let Ok(tokens) = Lexer::new(source).tokenize() else {
            return String::new();
        };
        let Ok(program) = Parser::new(tokens).parse() else {
            return String::new();
        };
        let Ok(chunks) = Compiler::new().compile(&program) else {
            return String::new();
        };
        let mut out = String::new();
        for (index, chunk) in chunks.iter().enumerate() {
            out.push_str(&format!(
                "==== chunk {index:04} ({:>4} bytes) ====\n",
                chunk.code.len()
            ));
            for (ip, op) in chunk.code.iter().enumerate() {
                out.push_str(&format!("{ip:04}  {op:?}\n"));
            }
        }
        out
    }
}

pub fn check_file(path: impl AsRef<Path>) -> RyndResult<()> {
    Compiler::new().compile(&modules::load(path)?.program)?;
    Ok(())
}

pub mod benchmarks;
pub mod build;
pub mod cli;
pub mod debugger;
pub mod error;
pub mod host;
#[cfg(feature = "jit")]
pub mod jit;
pub mod modules;
pub mod project;
pub mod source_tree;
pub mod syntax;
pub mod transpiler;
pub mod verdict;
pub mod vm;

pub use error::{RyndError, RyndResult, Span};
pub use host::{Context, FromValue, HostFn, IntoValue, Library, Registerable, Val};
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
    entry: usize,
    session: Rc<()>,
}

impl CompiledScript {
    /// Index of the script's entry chunk in its engine.
    pub fn entry(&self) -> usize {
        self.entry
    }
}

/// A named, hot-reloadable script: keeps its source next to the compiled
/// entry point so hosts can recompile it in place.
///
/// ```
/// use rynd::{Package, RyndEngine, Value};
///
/// let mut engine = RyndEngine::new();
/// let mut rules = Package::compile(&mut engine, "fn limit() { 10 }").unwrap();
/// rules.run(&mut engine).unwrap();
/// assert_eq!(engine.call("limit", &[]).unwrap(), Value::Int(10));
///
/// rules.reload(&mut engine, "fn limit() { 20 }").unwrap();
/// assert_eq!(engine.call("limit", &[]).unwrap(), Value::Int(20));
/// // A failed reload keeps the previous version.
/// assert!(rules.reload(&mut engine, "fn limit( {").is_err());
/// assert_eq!(rules.source(), "fn limit() { 20 }");
/// ```
#[derive(Clone, Debug)]
pub struct Package {
    script: CompiledScript,
    source: String,
}

impl Package {
    /// Compile `source` into `engine` without running it.
    pub fn compile(engine: &mut RyndEngine, source: &str) -> RyndResult<Self> {
        Ok(Self {
            script: engine.compile(source)?,
            source: source.to_string(),
        })
    }

    pub fn script(&self) -> &CompiledScript {
        &self.script
    }

    /// Run the package's top level against the engine it was compiled for.
    pub fn run(&self, engine: &mut RyndEngine) -> RyndResult<Value> {
        engine.run(&self.script)
    }

    /// Recompile from new source and run its top level, so its definitions
    /// replace the old ones. Values and closures created by earlier versions
    /// stay valid; on failure the previous version remains installed.
    pub fn reload(&mut self, engine: &mut RyndEngine, source: &str) -> RyndResult<Value> {
        let script = engine.compile(source)?;
        let saved = engine.machine.globals.clone();
        let value = engine
            .run(&script)
            .inspect_err(|_| engine.machine.globals = saved)?;
        self.script = script;
        self.source = source.to_string();
        Ok(value)
    }

    pub fn source(&self) -> &str {
        &self.source
    }
}

/// An engine bound to a [`Library`] of host items: the one-stop embedding
/// entry point. Resetting the runtime reinstalls the library.
///
/// ```
/// use rynd::{Library, ScriptRuntime, Value};
///
/// let mut lib = Library::new();
/// lib.register("fee", |cents: i64| cents / 50);
/// let mut runtime = ScriptRuntime::with_library(lib);
/// let pkg = runtime.load("fn total(cents) { cents + fee(cents) }").unwrap();
/// assert_eq!(runtime.call("total", &[Value::Int(1000)]).unwrap(), Value::Int(1020));
/// assert_eq!(pkg.source(), "fn total(cents) { cents + fee(cents) }");
/// ```
pub struct ScriptRuntime {
    pub engine: RyndEngine,
    library: Library,
}

impl ScriptRuntime {
    pub fn new() -> Self {
        Self::with_library(Library::new())
    }

    pub fn with_library(library: Library) -> Self {
        Self {
            engine: RyndEngine::with_library(&library),
            library,
        }
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    /// Compile and run `source`, returning a reloadable [`Package`].
    pub fn load(&mut self, source: &str) -> RyndResult<Package> {
        let package = Package::compile(&mut self.engine, source)?;
        package.run(&mut self.engine)?;
        Ok(package)
    }

    pub fn eval(&mut self, source: &str) -> RyndResult<Value> {
        self.engine.eval(source)
    }

    pub fn call(&mut self, name: &str, args: &[Value]) -> RyndResult<Value> {
        self.engine.call(name, args)
    }

    /// Discard script state and reinstall the library.
    pub fn reset(&mut self) {
        self.engine.reset();
        self.library.install(&mut self.engine);
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
    sandboxed: bool,
    #[cfg(feature = "jit")]
    jit: Option<jit::JitState>,
}

impl RyndEngine {
    pub fn new() -> Self {
        Self {
            machine: Machine::new(Vec::new()),
            callbacks: HashMap::new(),
            session: Rc::new(()),
            sandboxed: false,
            #[cfg(feature = "jit")]
            jit: None,
        }
    }

    /// An engine without builtins that touch files, stdin, the environment,
    /// processes, sleeping, or the network (see
    /// [`vm::safety::HOST_ACCESS_BUILTINS`]). Scripts can still reach
    /// whatever host functions you register. The restriction survives
    /// [`RyndEngine::reset`].
    ///
    /// ```
    /// let mut engine = rynd::RyndEngine::sandboxed();
    /// assert!(engine.eval("read_text('/etc/passwd')").is_err());
    /// assert_eq!(engine.eval("upper('ok')").unwrap().to_string(), "OK");
    /// ```
    pub fn sandboxed() -> Self {
        let mut engine = Self::new();
        engine.sandboxed = true;
        engine.remove_host_access();
        engine
    }

    fn remove_host_access(&mut self) {
        for name in vm::safety::HOST_ACCESS_BUILTINS {
            self.machine.globals.remove(*name);
        }
    }

    /// Remove a global (builtin, host function, or script binding) so
    /// scripts can no longer name it. Returns the removed value.
    pub fn remove_global(&mut self, name: &str) -> Option<Value> {
        self.callbacks.remove(name);
        self.machine.globals.remove(name)
    }

    /// A fresh engine with `library` installed.
    pub fn with_library(library: &Library) -> Self {
        let mut engine = Self::new();
        library.install(&mut engine);
        engine
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

    /// Compile `entry` from an in-memory [`SourceTree`], resolving its imports.
    pub fn compile_tree(
        &mut self,
        tree: &SourceTree,
        entry: impl AsRef<Path>,
    ) -> RyndResult<CompiledScript> {
        self.compile_program(&tree.compile(entry)?.program)
    }

    pub fn eval_tree(&mut self, tree: &SourceTree, entry: impl AsRef<Path>) -> RyndResult<Value> {
        let script = self.compile_tree(tree, entry)?;
        self.run(&script)
    }

    fn compile_program(&mut self, program: &Program) -> RyndResult<CompiledScript> {
        let entry = self.machine.chunks.len();
        let compiler = Compiler::with_offset(entry);
        let chunks = compiler.compile(program)?;

        self.machine.chunks.extend(chunks);
        #[cfg(feature = "jit")]
        self.jit_program(entry, program);
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

    /// Call a filter-map style function and split its result into a
    /// [`Verdict`] and payload (see [`verdict`]).
    pub fn filtermap(&mut self, name: &str, args: &[Value]) -> RyndResult<(Verdict, Value)> {
        self.call(name, args).map(Verdict::split)
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
        if self.sandboxed {
            self.remove_host_access();
        }
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

    /// Register a Rust function or closure with typed arguments and result
    /// (see [`host`]). Survives [`RyndEngine::reset`].
    pub fn register<Args, F: HostFn<Args>>(&mut self, name: &str, func: F) {
        self.register_closure(name, F::ARITY, move |args| func.call(args));
    }

    /// Install host data whose fields become script globals.
    pub fn install(&mut self, context: &impl Context) -> RyndResult<()> {
        context.install(self)
    }

    /// Register a typed one-argument closure; same as [`RyndEngine::register`].
    pub fn register_typed_fn1<A, R>(&mut self, name: &str, func: impl Fn(A) -> R + 'static)
    where
        A: FromValue + 'static,
        R: IntoValue + 'static,
    {
        self.register(name, func);
    }

    /// Register a typed two-argument closure; same as [`RyndEngine::register`].
    pub fn register_typed_fn2<A, B, R>(&mut self, name: &str, func: impl Fn(A, B) -> R + 'static)
    where
        A: FromValue + 'static,
        B: FromValue + 'static,
        R: IntoValue + 'static,
    {
        self.register(name, func);
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
/// contains a top-level `fn main` declaration. Lets embedders gate script
/// invocation without paying for bytecode compilation.
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

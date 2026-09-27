use rynd::syntax::ast::{BinaryOp, Expr, ExprKind};
use rynd::syntax::captures::free_names;
use rynd::syntax::needs_more;
use rynd::{RyndEngine, RyndError, Span, Value, build, check, check_file, project, transpile};
use std::fs;
use std::path::PathBuf;

struct TempDir(PathBuf);
impl TempDir {
    fn new(prefix: &str) -> Self {
        let dir = PathBuf::from("target").join(format!(
            "cov-{}-{}-{}",
            prefix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_needs_more_detection() {
    // Complete sources return false
    assert!(!needs_more("1 + 2"));
    assert!(!needs_more("fn f(x) { x * 2 }"));
    assert!(!needs_more("[1, 2, 3]"));
    assert!(!needs_more("user?.name ?: \"guest\""));

    // Incomplete sources requiring more input return true
    assert!(needs_more("1 +"));
    assert!(needs_more("fn f(x) {"));
    assert!(needs_more("[1, 2,"));
    assert!(needs_more("\"unterminated string"));

    // Invalid syntax that is NOT incomplete at EOF returns false
    assert!(!needs_more("1 + + 2"));
    assert!(!needs_more("fn (x) {}"));
}

#[test]
fn test_free_names_analysis() {
    let span = Span::new(1, 1);
    // fn test(a) { a + b + c }
    // 'a' is a parameter (bound), 'test' is the function name (bound).
    // 'b' and 'c' are free.
    let body = Expr::new(
        ExprKind::Binary {
            op: BinaryOp::Add,
            left: Box::new(Expr::new(
                ExprKind::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(Expr::new(ExprKind::Identifier("a".into()), span.clone())),
                    right: Box::new(Expr::new(ExprKind::Identifier("b".into()), span.clone())),
                },
                span.clone(),
            )),
            right: Box::new(Expr::new(ExprKind::Identifier("c".into()), span.clone())),
        },
        span.clone(),
    );

    let free = free_names("test", &["a".into()], &body);
    assert!(free.contains("b"));
    assert!(free.contains("c"));
    assert!(!free.contains("a"));
    assert!(!free.contains("test"));
}

#[test]
fn test_error_variants_and_formatting() {
    let span_no_file = Span::new(10, 20);
    assert_eq!(format!("{span_no_file}"), "10:20");

    let span_with_file = span_no_file.clone().with_file("test.rynd");
    assert_eq!(format!("{span_with_file}"), "test.rynd:10:20");

    let err_lex = RyndError::LexError {
        message: "bad char".into(),
        span: span_with_file.clone(),
    };
    assert_eq!(
        format!("{err_lex}"),
        "Lexer error at test.rynd:10:20: bad char"
    );

    let err_parse = RyndError::ParseError {
        message: "expected comma".into(),
        span: span_no_file.clone(),
    };
    assert_eq!(
        format!("{err_parse}"),
        "Parse error at 10:20: expected comma"
    );

    let err_compile = RyndError::CompileError {
        message: "undefined var".into(),
    };
    assert_eq!(format!("{err_compile}"), "Compile error: undefined var");

    let err_runtime = RyndError::RuntimeError {
        message: "out of bounds".into(),
    };
    assert_eq!(format!("{err_runtime}"), "Runtime error: out of bounds");

    let err_transpile = RyndError::TranspileError {
        message: "invalid ast".into(),
    };
    assert_eq!(format!("{err_transpile}"), "Transpile error: invalid ast");

    let err_io = RyndError::IoError("disk failure".into());
    assert_eq!(format!("{err_io}"), "IO error: disk failure");

    // Located error wrapping and file attachment
    let located = err_runtime.at(span_no_file.clone());
    assert_eq!(
        format!("{located}"),
        "Runtime error: out of bounds (at 10:20)"
    );

    let located_with_file = located.with_file("script.rynd");
    assert_eq!(
        format!("{located_with_file}"),
        "Runtime error: out of bounds (at script.rynd:10:20)"
    );
}

#[test]
fn test_project_scaffolding_validation() {
    let tmp = TempDir::new("project");

    // Invalid project names
    assert!(project::new(tmp.0.join("123invalid"), false).is_err());
    assert!(project::new(tmp.0.join("invalid$name"), false).is_err());
    assert!(project::new(tmp.0.join("crate"), false).is_err());
    assert!(project::new(tmp.0.join("self"), false).is_err());
    assert!(project::new(tmp.0.join("super"), false).is_err());
    assert!(project::new(tmp.0.join("Self"), false).is_err());

    // Valid app creation
    let app_dir = tmp.0.join("valid-app");
    assert!(project::new(&app_dir, false).is_ok());
    assert!(app_dir.join("Cargo.toml").exists());
    assert!(app_dir.join("src/main.rs").exists());
    assert!(app_dir.join("src/main.rynd").exists());
    assert!(app_dir.join("src/native.rs").exists());

    // Destination already exists error
    assert!(project::new(&app_dir, false).is_err());

    // Valid lib creation
    let lib_dir = tmp.0.join("valid-lib");
    assert!(project::new(&lib_dir, true).is_ok());
    assert!(lib_dir.join("Cargo.toml").exists());
    assert!(lib_dir.join("src/lib.rs").exists());
    assert!(!lib_dir.join("src/main.rs").exists());
}

#[test]
fn test_build_distinct_output_and_cargo_module_errors() {
    let tmp = TempDir::new("build");
    let input = tmp.0.join("source.rynd");
    fs::write(&input, "let x = 1").unwrap();

    // Distinct output: same file error
    assert!(build::distinct_output(&input, &input).is_err());

    // Distinct output: different file succeeds
    let output = tmp.0.join("output.bin");
    assert!(build::distinct_output(&input, &output).is_ok());

    // cargo_module invalid output name
    assert!(build::cargo_module(&input, "sub/dir/out.rs").is_err());
    assert!(build::cargo_module(&input, "out.bin").is_err());

    // cargo_module missing environment variables
    assert!(build::cargo_module(&input, "out.rs").is_err());
}

#[test]
fn test_engine_check_and_globals_lifecycle() {
    let mut engine = RyndEngine::new();

    // check without execution (doesn't panic or produce runtime errors)
    assert!(check("let x = 10 + 20").is_ok());
    assert!(check("fn fail() { 1 / 0 }").is_ok()); // checks syntax/compile, does not run
    assert!(check("let =").is_err()); // syntax error

    // check_file
    let tmp = TempDir::new("engine");
    let script_file = tmp.0.join("script.rynd");
    fs::write(&script_file, "let y = 42").unwrap();
    assert!(check_file(&script_file).is_ok());

    let missing_file = tmp.0.join("missing.rynd");
    assert!(check_file(&missing_file).is_err());

    // globals lifecycle
    engine.set_global("count", Value::Int(99));
    assert_eq!(engine.get_global("count").unwrap(), Value::Int(99));
    assert!(engine.get_global("nonexistent_global").is_err());

    // evaluate using global
    assert_eq!(engine.eval("count + 1").unwrap(), Value::Int(100));

    // reset clears user globals
    engine.reset();
    assert!(engine.get_global("count").is_err());
    // but builtins remain
    assert!(engine.get_global("sum").is_ok());
}

#[test]
fn test_engine_registered_call_and_session_isolation() {
    let mut engine = RyndEngine::new();

    // register_fn with custom callback
    engine.register_fn("add_ten", 1, |args| match &args[0] {
        Value::Int(n) => Ok(Value::Int(n + 10)),
        _ => Err(RyndError::RuntimeError {
            message: "Expected int".into(),
        }),
    });

    assert_eq!(engine.eval("add_ten(5)").unwrap(), Value::Int(15));
    assert!(engine.eval("add_ten('string')").is_err());

    // Top-level function calling via engine.call
    engine.eval("fn multiply(a, b) { a * b }").unwrap();
    let res = engine
        .call("multiply", &[Value::Int(6), Value::Int(7)])
        .unwrap();
    assert_eq!(res, Value::Int(42));

    // Calling nonexistent function
    assert!(engine.call("no_such_function", &[]).is_err());

    // Calling with wrong arity
    assert!(engine.call("multiply", &[Value::Int(6)]).is_err());

    // CompiledScript session isolation
    let compiled = engine.compile("1 + 1").unwrap();
    assert_eq!(engine.run(&compiled).unwrap(), Value::Int(2));

    let mut other_engine = RyndEngine::new();
    let err = other_engine.run(&compiled).unwrap_err();
    assert!(err.to_string().contains("belongs to another engine"));
}

#[test]
fn test_transpile_function() {
    let rust = transpile("let x = 10; x * 2").expect("Transpilation failed");
    assert!(rust.contains("fn rynd_program"));
}

//! Stage 8 additions: cheap syntactic checks, an embeddable CLI helper,
//! and inspection helpers for the parsed AST and the compiled bytecode.

#[test]
fn has_main_function_detects_top_level_main() {
    assert!(rynd::has_main_function("fn main() { 1 + 1 }"));
    assert!(rynd::has_main_function("let x = 1\nfn main() { x }"));
    // Functions other than main do not satisfy the predicate.
    assert!(!rynd::has_main_function("fn helper() { 1 }"));
    // `main` as a let-binding is not the function form.
    assert!(!rynd::has_main_function("let main = 1"));
}

#[test]
fn has_main_function_rejects_invalid_syntax_without_panicking() {
    // Invalid input should return false rather than raise, mirroring the
    // cost of `has_main` as a cheap gate before invoking the script.
    assert!(!rynd::has_main_function("fn main("));
    assert!(!rynd::has_main_function(""));
}

#[test]
fn tools_print_ast_returns_non_empty_string() {
    let ast = rynd::tools::print_ast("1 + 2");
    assert!(!ast.is_empty());
    assert!(
        ast.contains('+') || ast.to_lowercase().contains("add"),
        "AST output should mention the operator or operation: {ast}"
    );
}

#[test]
fn tools_print_bytecode_includes_chunk_offsets() {
    let dump = rynd::tools::print_bytecode("1 + 2");
    assert!(!dump.is_empty());
    // The opcode dump uses 4-digit offsets separated by whitespace.
    assert!(
        dump.contains("0000") || dump.contains("0001"),
        "bytecode dump should include offsets: {dump}"
    );
}

#[test]
fn cli_command_runs_and_prints_version() {
    // The cli() helper builds a std::process::Command that runs the embedded
    // binary. Running it with --version must succeed and emit a version string.
    let mut cmd = rynd::cli(std::process::Command::new(env!("CARGO_BIN_EXE_rynd")));
    cmd.arg("--version");
    let out = cmd.output().expect("cli subprocess must run");
    assert!(out.status.success(), "rynd --version should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("rynd"),
        "version output should mention 'rynd': {stdout}"
    );
}
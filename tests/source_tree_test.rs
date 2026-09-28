//! Stage 6: SourceTree — in-memory multi-file sources for embedders.

#[test]
fn source_tree_add_and_count() {
    let mut tree = rynd::SourceTree::new();
    assert_eq!(tree.len(), 0);
    tree.add("a.rynd", "let x = 1");
    tree.add("b.rynd", "let y = 2");
    assert_eq!(tree.len(), 2);
    assert!(tree.contains("a.rynd"));
    assert!(!tree.contains("missing.rynd"));
}

#[test]
fn source_tree_compilees_entry_yields_program() {
    let mut tree = rynd::SourceTree::new();
    tree.add("main.rynd", "fn main() { 1 + 1 }");
    let bundle = tree.compile("main.rynd").expect("compile entry");
    // The compiled bundle has the file list.
    assert_eq!(bundle.files.len(), 1);
    assert_eq!(
        bundle.files[0].to_str().unwrap(),
        "main.rynd",
        "file list preserves the entry-point path"
    );
    // The program contains the function declaration.
    assert!(bundle.program.statements.iter().any(|stmt| matches!(
        stmt,
        rynd::syntax::Stmt::Function { name, .. } if name == "main"
    )));
}

#[test]
fn source_tree_compile_missing_entry_errors() {
    let tree = rynd::SourceTree::new();
    let result = tree.compile("missing.rynd");
    assert!(result.is_err());
}

#[test]
fn source_tree_compile_invalid_syntax_errors() {
    let mut tree = rynd::SourceTree::new();
    tree.add("main.rynd", "fn main( ");
    let result = tree.compile("main.rynd");
    assert!(result.is_err());
}
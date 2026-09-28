//! In-memory multi-file source trees.
//!
//! [`SourceTree`] holds a map of named source files and a `compile`
//! entry point that lexes+parses each file independently. It is the
//! minimal counterpart to Roto's `FileTree` for embedders that want to
//! hand the engine a playground, a REPL session, or a test fixture
//! without touching the filesystem.
//!
//! The tree is intentionally simple: imports between files are not
//! resolved, and there is no on-disk canonicalization. Use
//! [`crate::modules::load`] when you need full module graph semantics.
use crate::error::RyndResult;
use crate::modules::ModuleBundle;
use crate::syntax::ast::Program;
use crate::syntax::lexer::Lexer;
use crate::syntax::parser::Parser;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// An ordered collection of named source files keyed by relative path.
#[derive(Clone, Debug, Default)]
pub struct SourceTree {
    files: BTreeMap<PathBuf, String>,
}

impl SourceTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace a file. Paths are stored as given; use forward
    /// slashes for cross-platform consistency.
    pub fn add(&mut self, path: impl Into<PathBuf>, source: impl Into<String>) {
        self.files.insert(path.into(), source.into());
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn contains(&self, path: impl AsRef<Path>) -> bool {
        self.files.contains_key(path.as_ref())
    }

    pub fn get(&self, path: impl AsRef<Path>) -> Option<&str> {
        self.files.get(path.as_ref()).map(String::as_str)
    }

    pub fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.files.keys()
    }

    /// Lex+parse the entry-point file. Each sibling file is lexed+parsed
    /// so its errors are caught early, but only the entry point's program
    /// is returned. The bundle's `files` list enumerates every file in
    /// the tree in sorted path order.
    pub fn compile(&self, entry: impl AsRef<Path>) -> RyndResult<ModuleBundle> {
        let entry_path = entry.as_ref().to_path_buf();
        let entry_source = self
            .files
            .get(&entry_path)
            .ok_or_else(|| crate::RyndError::IoError(format!(
                "entry file not in tree: {}",
                entry_path.display()
            )))?;
        // Lex+parse every file up front; surface sibling errors before
        // returning the entry's program.
        for (path, source) in &self.files {
            let mut tokens = Lexer::new(source).tokenize()?;
            for token in &mut tokens {
                token.span.file = Some(path.to_string_lossy().as_ref().into());
            }
            Parser::new(tokens).parse().map_err(|e| {
                e.with_file(path.to_string_lossy().as_ref().to_string())
            })?;
        }
        let mut tokens = Lexer::new(entry_source).tokenize()?;
        for token in &mut tokens {
            token.span.file = Some(entry_path.to_string_lossy().as_ref().into());
        }
        let program = Parser::new(tokens).parse()?;
        let files = self.files.keys().cloned().collect();
        Ok(ModuleBundle {
            program: Program {
                statements: program.statements,
            },
            files,
            exports: Vec::new(),
        })
    }
}
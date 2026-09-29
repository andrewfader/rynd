//! In-memory multi-file sources.
//!
//! [`SourceTree`] maps relative paths to source text and resolves `import`
//! statements between them exactly like files on disk, without touching the
//! filesystem. Use it for playgrounds, sandboxes, REPL sessions, generated
//! code, and test fixtures.
//!
//! ```
//! use rynd::{RyndEngine, SourceTree, Value};
//!
//! let mut tree = SourceTree::new();
//! tree.add("lib/tax.rynd", "pub fn with_tax(cents) { cents * 108 / 100 }");
//! tree.add("main.rynd", "import \"lib/tax.rynd\" as tax\ntax.with_tax(1000)");
//!
//! let mut engine = RyndEngine::new();
//! assert_eq!(engine.eval_tree(&tree, "main.rynd").unwrap(), Value::Int(1080));
//! ```
use crate::error::{RyndError, RyndResult};
use crate::modules::{self, ModuleBundle, SourceProvider};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// An ordered collection of named source files keyed by relative path.
#[derive(Clone, Debug, Default)]
pub struct SourceTree {
    files: BTreeMap<PathBuf, String>,
}

impl SourceTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace a file. `a/./b.rynd` and `a/c/../b.rynd` name the same file.
    pub fn add(&mut self, path: impl AsRef<Path>, source: impl Into<String>) {
        self.files.insert(normalize(path.as_ref()), source.into());
    }

    pub fn remove(&mut self, path: impl AsRef<Path>) -> Option<String> {
        self.files.remove(&normalize(path.as_ref()))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn contains(&self, path: impl AsRef<Path>) -> bool {
        self.files.contains_key(&normalize(path.as_ref()))
    }

    pub fn get(&self, path: impl AsRef<Path>) -> Option<&str> {
        self.files
            .get(&normalize(path.as_ref()))
            .map(String::as_str)
    }

    pub fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.files.keys()
    }

    /// Resolve `entry` and everything it imports into one program. The bundle
    /// lists the files actually loaded and the entry's public exports.
    pub fn compile(&self, entry: impl AsRef<Path>) -> RyndResult<ModuleBundle> {
        modules::load_from(self, entry.as_ref())
    }
}

impl SourceProvider for SourceTree {
    fn canonical(&self, path: &Path) -> RyndResult<PathBuf> {
        let path = normalize(path);
        if self.files.contains_key(&path) {
            Ok(path)
        } else {
            Err(RyndError::IoError(format!(
                "{}: not found in source tree",
                path.display()
            )))
        }
    }

    fn read(&self, canonical: &Path) -> RyndResult<String> {
        self.files.get(canonical).cloned().ok_or_else(|| {
            RyndError::IoError(format!("{}: not found in source tree", canonical.display()))
        })
    }
}

/// Lexically resolve `.` and `..` so tree paths never depend on a real directory.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
        }
    }
    out
}

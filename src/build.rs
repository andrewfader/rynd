//! Native compilation and build.rs integration for Cargo applications/libraries.
use crate::{RustTranspiler, RyndError, RyndResult};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn io(error: impl std::fmt::Display) -> RyndError {
    RyndError::IoError(error.to_string())
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Compile generated Rust to a standalone executable. The destination is only
/// replaced after rustc succeeds. No compiler or temporary source ships in it.
pub fn native(rust: &str, output: impl AsRef<Path>) -> RyndResult<()> {
    let output = output.as_ref();
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io)?
        .as_nanos();
    let scratch = Scratch(parent.join(format!(".rynd-build-{}-{unique}", std::process::id())));
    fs::create_dir(&scratch.0).map_err(io)?;
    let source = scratch.0.join("program.rs");
    let binary = scratch.0.join("program");
    fs::write(&source, rust).map_err(io)?;
    let result = Command::new("rustc")
        .args(["--edition=2024", "--crate-name", "rynd_program", "-O"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .map_err(|e| io(format!("Cannot run rustc: {e}")))?;
    if !result.status.success() {
        return Err(io(format!(
            "Native compilation failed:\n{}",
            String::from_utf8_lossy(&result.stderr)
        )));
    }
    fs::rename(&binary, output).map_err(io)
}

/// Reject aliases of the source file before writing generated output.
pub fn distinct_output(input: &Path, output: &Path) -> RyndResult<()> {
    if let (Ok(a), Ok(b)) = (input.canonicalize(), output.canonicalize()) {
        if a == b {
            return Err(io("Output must not overwrite the input source"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let a = fs::metadata(a).map_err(io)?;
            let b = fs::metadata(b).map_err(io)?;
            if a.dev() == b.dev() && a.ino() == b.ino() {
                return Err(io("Output is a hard link to the input source"));
            }
        }
    }
    Ok(())
}

/// Use from a Cargo build.rs. Resolve imports, emit direct Rust into OUT_DIR,
/// and tell Cargo about every source file that can invalidate the result.
pub fn cargo_module(entry: impl AsRef<Path>, output_name: &str) -> RyndResult<()> {
    if Path::new(output_name).components().count() != 1 || !output_name.ends_with(".rs") {
        return Err(io("Generated module name must be a single .rs filename"));
    }
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| io("CARGO_MANIFEST_DIR is missing; call cargo_module from build.rs"))?;
    let output = std::env::var_os("OUT_DIR")
        .ok_or_else(|| io("OUT_DIR is missing; call cargo_module from build.rs"))?;
    let bundle = crate::modules::load(Path::new(&manifest).join(entry))?;
    for file in &bundle.files {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    let rust = RustTranspiler::new().transpile_module(&bundle.program, &bundle.exports)?;
    fs::write(Path::new(&output).join(output_name), rust).map_err(io)
}

//! Safety knobs documented in `SECURITY.md`.
//!
//! These constants let embedders reason about the engine's resource limits
//! without scraping the source. Changing them here is intentionally coupled
//! with the matching checks in [`crate::vm::machine`] and [`crate::vm::runtime`];
//! the constants below must match those checks exactly.

/// Maximum call-stack depth before the interpreter raises a runtime error.
///
/// Recursion deeper than this returns `RuntimeError("Call depth limit exceeded ...")`
/// instead of overflowing the stack. Embedders running untrusted scripts
/// can fork the engine and patch the depth check, but the constants below
/// describe the default behavior.
pub const MAX_CALL_DEPTH: usize = 256;
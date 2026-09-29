//! Resource limits enforced by every Rynd backend (see `SECURITY.md`).

/// Maximum script call depth. The bytecode VM, generated native Rust, and the
/// optional JIT all raise a runtime error instead of overflowing the stack.
pub const MAX_CALL_DEPTH: usize = 256;

/// The error every backend reports when [`MAX_CALL_DEPTH`] is exceeded.
pub fn depth_error() -> crate::error::RyndError {
    super::runtime::error(format!("Call depth limit exceeded ({MAX_CALL_DEPTH})"))
}

/// Builtins that reach outside the engine: files, stdin, environment,
/// processes, sleeping, and TCP. [`crate::RyndEngine::sandboxed`] removes them.
pub const HOST_ACCESS_BUILTINS: &[&str] = &[
    "read_text",
    "read_stdin",
    "read_lines",
    "cat",
    "write_text",
    "append_text",
    "read_bytes",
    "write_bytes",
    "cwd",
    "env",
    "list_dir",
    "file_info",
    "exists",
    "mkdir_all",
    "sleep_ms",
    "run_process",
    "tcp_listen",
    "tcp_accept",
    "tcp_connect",
    "tcp_local_addr",
    "socket_read",
    "socket_read_bytes",
    "socket_write",
    "socket_close",
    "socket_set_nonblocking",
    "socket_write_http_chunk",
    "socket_finish_http_chunks",
    "socket_read_chunk",
];

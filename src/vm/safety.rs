//! Resource limits enforced by every Rynd backend (see `SECURITY.md`).

use std::cell::Cell;

/// Maximum script call depth. The bytecode VM, generated native Rust, and the
/// optional JIT all raise a runtime error instead of overflowing the stack.
/// Self-recursive tail calls run in constant space and never count against it.
pub const MAX_CALL_DEPTH: usize = 10_000;

/// Stack reserved for threads Rynd starts to run programs (CLI runs,
/// generated binaries, fibers). Pages are committed only when touched.
pub const PROGRAM_STACK_BYTES: usize = 512 * 1024 * 1024;

/// Usable stack assumed on a thread Rynd did not start (an embedding host's
/// thread), measured from the first script call seen on it.
const FOREIGN_STACK_BUDGET: usize = 1024 * 1024;

/// Headroom kept below a known stack size for the frames between checks.
const STACK_MARGIN: usize = 256 * 1024;

thread_local! {
    /// (address near the stack's base, usable bytes below it); 0 = unmeasured.
    static STACK: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

fn stack_address() -> usize {
    let marker = 0u8;
    std::hint::black_box(std::ptr::addr_of!(marker)) as usize
}

/// Run `f` on a new thread with a [`PROGRAM_STACK_BYTES`] stack, so deep
/// recursion reaches [`MAX_CALL_DEPTH`] instead of the platform's default.
pub fn run_with_program_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    run_with_stack(PROGRAM_STACK_BYTES, f)
}

pub fn run_with_stack<T: Send + 'static>(
    bytes: usize,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let thread = std::thread::Builder::new()
        .stack_size(bytes)
        .spawn(move || {
            claim_stack(bytes);
            f()
        });
    match thread {
        Ok(handle) => handle
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
        Err(e) => panic!("could not start a {bytes}-byte program thread: {e}"),
    }
}

/// Record that the current thread, starting about here, has `bytes` of stack.
pub fn claim_stack(bytes: usize) {
    STACK.with(|s| s.set((stack_address(), bytes.saturating_sub(STACK_MARGIN))));
}

/// Whether a nested call would run this thread's stack too low. Checked by
/// every backend where script calls recurse on the native stack.
pub fn stack_exhausted() -> bool {
    let here = stack_address();
    STACK.with(|s| {
        let (base, budget) = s.get();
        if base == 0 {
            s.set((here, FOREIGN_STACK_BUDGET));
            return false;
        }
        base.saturating_sub(here) > budget
    })
}

/// The error every backend reports when [`MAX_CALL_DEPTH`] is exceeded.
pub fn depth_error() -> crate::error::RyndError {
    super::runtime::error(format!("Call depth limit exceeded ({MAX_CALL_DEPTH})"))
}

/// The error for running out of native stack before [`MAX_CALL_DEPTH`].
pub fn stack_error() -> crate::error::RyndError {
    super::runtime::error(
        "Call depth limit exceeded (stack space); run on a thread with a larger stack",
    )
}

/// Builtins that reach outside the engine: files, stdin, environment,
/// processes, sleeping, TCP and HTTP, and on-disk stores.
/// [`crate::RyndEngine::sandboxed`] removes them.
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
    "serve",
    "read_http_request",
    "http_request",
    "fetch",
    "plug_send",
    "store_open",
];

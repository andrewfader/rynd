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
        Err(e) => panic!("could not start a {}-byte program thread: {}", bytes, e),
    }
}

/// Record that the current thread, starting about here, has `bytes` of stack.
pub fn claim_stack(bytes: usize) {
    STACK.with(|s| s.set((stack_address(), bytes.saturating_sub(STACK_MARGIN))));
}

/// This thread's recorded stack (see [`claim_stack`]), saved and restored
/// by the fiber scheduler as it switches stacks.
pub fn stack_claim() -> (usize, usize) {
    STACK.with(|s| s.get())
}

pub fn set_stack_claim(claim: (usize, usize)) {
    STACK.with(|s| s.set(claim));
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
    depth_error_at(MAX_CALL_DEPTH)
}

/// The error for exceeding a call depth limit of `limit`.
pub fn depth_error_at(limit: usize) -> crate::error::RyndError {
    super::runtime::error(format!("Call depth limit exceeded ({limit})"))
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

// ---------------------------------------------------------------- limits

/// Resource limits for running untrusted scripts. Each run (`eval`, `run`,
/// `call`, ...) of a limited engine gets a fresh budget; exceeding any limit
/// stops the run with [`RyndError::LimitExceeded`](crate::RyndError), which
/// scripts cannot catch. `None` means unlimited.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Limits {
    /// Work units: one per function call, plus one per element a builtin
    /// reads or builds (and a builtin's own loop count, such as hash rounds).
    pub operations: Option<u64>,
    /// Wall-clock time per run, including time fibers spend waiting.
    pub time: Option<std::time::Duration>,
    /// Bytes of strings and collections created per run, counted when
    /// created (memory freed later is not credited back).
    pub allocation: Option<u64>,
    /// Longest string a script may build, in bytes.
    pub string_bytes: Option<usize>,
    /// Most elements in one list, tuple, or map.
    pub collection_len: Option<usize>,
    /// Script call depth; never above [`MAX_CALL_DEPTH`].
    pub call_depth: Option<usize>,
    /// Largest source text the engine compiles, in bytes.
    pub source_bytes: Option<usize>,
    /// Fibers alive at once.
    pub fibers: Option<usize>,
}

impl Limits {
    /// No limits beyond the always-on call depth and nesting caps.
    pub fn none() -> Self {
        Self::default()
    }
    /// Conservative limits for untrusted input, used by
    /// [`crate::RyndEngine::sandboxed`]. Adjust fields as needed.
    pub fn sandbox() -> Self {
        Self {
            operations: Some(50_000_000),
            time: Some(std::time::Duration::from_secs(5)),
            allocation: Some(512 * 1024 * 1024),
            string_bytes: Some(16 * 1024 * 1024),
            collection_len: Some(1_000_000),
            call_depth: Some(1_000),
            source_bytes: Some(1024 * 1024),
            fibers: Some(1_000),
        }
    }
    pub fn is_unlimited(&self) -> bool {
        *self == Self::default()
    }
}

/// Stops a running script from another thread: the run ends with
/// [`RyndError::LimitExceeded`](crate::RyndError) at its next check.
#[derive(Clone, Debug, Default)]
pub struct Interrupt(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Interrupt {
    pub fn new() -> Self {
        Self::default()
    }
    /// Ask the script to stop. Stays set until [`Interrupt::reset`].
    pub fn interrupt(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn reset(&self) {
        self.0.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn is_interrupted(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

struct Budget {
    limits: Limits,
    operations: u64,
    allocated: u64,
    deadline: Option<std::time::Instant>,
    interrupt: Option<Interrupt>,
    /// The first limit exceeded; every later check reports it again.
    exceeded: Option<String>,
}

thread_local! {
    static BUDGET: std::cell::RefCell<Option<Budget>> = const { std::cell::RefCell::new(None) };
    /// Whether a budget is active, readable without borrowing it.
    static LIMITED: Cell<bool> = const { Cell::new(false) };
    /// Whether the active budget is exhausted (or interrupted).
    static TRIPPED: Cell<bool> = const { Cell::new(false) };
}

/// Restores the previous budget (if any) when a limited run ends.
pub struct BudgetScope {
    previous: Option<Budget>,
    limited: bool,
    tripped: bool,
}

impl Drop for BudgetScope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        BUDGET.with(|b| *b.borrow_mut() = previous);
        LIMITED.with(|l| l.set(self.limited));
        TRIPPED.with(|t| t.set(self.tripped));
    }
}

/// Start a run under `limits` on this thread until the scope is dropped.
pub fn enter(limits: &Limits, interrupt: Option<Interrupt>) -> BudgetScope {
    let active = !limits.is_unlimited() || interrupt.is_some();
    let budget = active.then(|| Budget {
        deadline: limits.time.map(|t| std::time::Instant::now() + t),
        limits: limits.clone(),
        operations: 0,
        allocated: 0,
        interrupt,
        exceeded: None,
    });
    let previous = BUDGET.with(|b| std::mem::replace(&mut *b.borrow_mut(), budget));
    BudgetScope {
        previous,
        limited: LIMITED.with(|l| l.replace(active)),
        tripped: TRIPPED.with(|t| t.replace(false)),
    }
}

/// Whether a run on this thread has limits. Cheap; check before charging.
#[inline(always)]
pub fn limited() -> bool {
    LIMITED.with(|l| l.get())
}

/// Whether the current run has exceeded a limit or was interrupted.
#[inline(always)]
pub fn tripped() -> bool {
    TRIPPED.with(|t| t.get())
}

fn exceeded(budget: &mut Budget, what: String) -> crate::error::RyndError {
    let message = budget.exceeded.get_or_insert(what).clone();
    TRIPPED.with(|t| t.set(true));
    crate::error::RyndError::LimitExceeded { message }
}

fn with_budget<T>(f: impl FnOnce(&mut Budget) -> T) -> Option<T> {
    BUDGET.with(|b| b.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Check time, interrupts, and any limit already exceeded.
pub fn check() -> crate::error::RyndResult<()> {
    if !limited() {
        return Ok(());
    }
    with_budget(|budget| {
        if let Some(what) = &budget.exceeded {
            return Err(exceeded(budget, what.clone()));
        }
        if budget
            .interrupt
            .as_ref()
            .is_some_and(Interrupt::is_interrupted)
        {
            return Err(exceeded(budget, "interrupted by the host".into()));
        }
        // No let-chains: generated programs compile this file as edition 2015.
        match (budget.deadline, budget.limits.time) {
            (Some(deadline), Some(time)) if std::time::Instant::now() >= deadline => {
                Err(exceeded(budget, format!("time ({time:?})")))
            }
            _ => Ok(()),
        }
    })
    .unwrap_or(Ok(()))
}

/// Charge `units` of work to the current run.
pub fn charge(units: u64) -> crate::error::RyndResult<()> {
    if !limited() {
        return Ok(());
    }
    with_budget(|budget| {
        budget.operations = budget.operations.saturating_add(units);
        match budget.limits.operations {
            Some(max) if budget.operations > max => {
                Err(exceeded(budget, format!("operations ({max})")))
            }
            _ => Ok(()),
        }
    })
    .unwrap_or(Ok(()))?;
    check()
}

/// Refuse to build a collection of `len` elements before allocating it.
pub fn reserve_items(len: usize) -> crate::error::RyndResult<()> {
    if !limited() {
        return Ok(());
    }
    with_budget(|budget| match budget.limits.collection_len {
        Some(max) if len > max => Err(exceeded(budget, format!("collection length ({max})"))),
        _ => Ok(()),
    })
    .unwrap_or(Ok(()))?;
    charge(len as u64)
}

/// Refuse to build a string of `bytes` before allocating it.
pub fn reserve_string(bytes: usize) -> crate::error::RyndResult<()> {
    if !limited() {
        return Ok(());
    }
    with_budget(|budget| match budget.limits.string_bytes {
        Some(max) if bytes > max => Err(exceeded(budget, format!("string length ({max} bytes)"))),
        _ => Ok(()),
    })
    .unwrap_or(Ok(()))?;
    charge(bytes as u64 / 64)
}

/// The longest string the current run may build.
pub fn string_cap() -> usize {
    if !limited() {
        return usize::MAX;
    }
    with_budget(|b| b.limits.string_bytes.unwrap_or(usize::MAX)).unwrap_or(usize::MAX)
}

/// Record a value the run just created. Infallible, because values are
/// built everywhere; an overrun trips the budget, and the run stops at its
/// next call or check.
#[inline]
pub fn created(elements: usize, bytes: usize) {
    if limited() {
        created_slow(elements, bytes);
    }
}

#[cold]
fn created_slow(elements: usize, bytes: usize) {
    with_budget(|budget| {
        if budget.exceeded.is_some() {
            return;
        }
        budget.allocated = budget.allocated.saturating_add(bytes as u64);
        budget.operations = budget.operations.saturating_add(elements as u64);
        let limits = &budget.limits;
        let over = if limits.allocation.is_some_and(|max| budget.allocated > max) {
            Some(format!(
                "allocation ({} bytes)",
                limits.allocation.unwrap_or(0)
            ))
        } else if limits.operations.is_some_and(|max| budget.operations > max) {
            Some(format!("operations ({})", limits.operations.unwrap_or(0)))
        } else if limits.collection_len.is_some_and(|max| elements > max) {
            Some(format!(
                "collection length ({})",
                limits.collection_len.unwrap_or(0)
            ))
        } else if limits
            .string_bytes
            .is_some_and(|max| bytes > max && elements == 0)
        {
            Some(format!(
                "string length ({} bytes)",
                limits.string_bytes.unwrap_or(0)
            ))
        } else {
            None
        };
        if let Some(what) = over {
            exceeded(budget, what);
        }
    });
}

/// The call depth allowed for the current run.
pub fn call_depth_limit() -> usize {
    if !limited() {
        return MAX_CALL_DEPTH;
    }
    with_budget(|b| {
        b.limits
            .call_depth
            .map_or(MAX_CALL_DEPTH, |d| d.min(MAX_CALL_DEPTH))
    })
    .unwrap_or(MAX_CALL_DEPTH)
}

/// When the current run must end, for waits that would outlast it.
pub fn deadline() -> Option<std::time::Instant> {
    if !limited() {
        return None;
    }
    with_budget(|b| b.deadline).flatten()
}

/// The most fibers the current run may have alive.
pub fn fiber_limit() -> Option<usize> {
    if !limited() {
        return None;
    }
    with_budget(|b| b.limits.fibers).flatten()
}

/// Report that the run started more fibers than allowed.
pub fn fibers_exceeded(max: usize) -> crate::error::RyndError {
    with_budget(|budget| exceeded(budget, format!("fibers ({max})"))).unwrap_or_else(|| {
        crate::error::RyndError::LimitExceeded {
            message: format!("fibers ({max})"),
        }
    })
}

/// Render a value as text without exceeding the string limit: a small list
/// that shares one large list many times must not print gigabytes.
pub fn render(value: &super::value::Value) -> crate::error::RyndResult<String> {
    let cap = string_cap();
    if cap == usize::MAX {
        return Ok(value.to_string());
    }
    struct Capped {
        out: String,
        cap: usize,
    }
    impl std::fmt::Write for Capped {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            if self.out.len() + s.len() > self.cap {
                return Err(std::fmt::Error);
            }
            self.out.push_str(s);
            Ok(())
        }
    }
    let mut capped = Capped {
        out: String::new(),
        cap,
    };
    match std::fmt::Write::write_fmt(&mut capped, format_args!("{value}")) {
        Ok(()) => Ok(capped.out),
        Err(_) => reserve_string(cap.saturating_add(1)).map(|_| String::new()),
    }
}

/// Deepest nesting of brackets, blocks, and patterns the parser accepts,
/// so compiling hostile input cannot overflow the stack.
pub const MAX_SYNTAX_NESTING: usize = 200;

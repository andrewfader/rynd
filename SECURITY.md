# Security considerations

If you allow users to submit untrusted Rynd scripts to your application,
treat the engine as a process that can read files, open sockets, fork
processes, allocate memory, and recurse. Sandboxing is your responsibility,
not the engine's.

## What an untrusted script can do today

The bytecode VM and the Rust transpiler share the host's process authority.
A malicious or buggy script can:

- **Crash the host** by exhausting memory with infinite recursion (the VM
  bounds call depth at 256 and surfaces it as a runtime error, but the host
  process still pays for stack growth before the limit trips). Long-running
  scripts that build large lists, deeply nested maps, or huge strings can
  also OOM the host.
- **Loop forever** in a `while` expression. The interpreter has no execution
  budget.
- **Run a while loop that emits unbounded output** via `print`/`println`,
  filling the host's stdout.
- **Open TCP listeners or connections** (`tcp_listen`, `tcp_connect`,
  `socket_*`). With `run_process` it can launch arbitrary subprocesses.
  Disabling the `tcp` feature removes the network surface; `run_process`
  is always available because it sits in the core scripting module.
- **Read or write any path the host can read or write**, including through
  `read_text`, `write_text`, `read_bytes`, `write_bytes`, `cwd`, `env`,
  `list_dir`, `file_info`, `exists`, and `mkdir_all`. There is no
  filesystem chroot.
- **Execute arbitrary `rustc` during compilation** via the `transpiler`
  feature (`rynd::RustTranspiler` + `rynd::build::cargo_module`). Compile
  jobs spawned by `build::native` run with the host's filesystem access
  and environment.

## Recommended mitigations

The host application must impose limits the engine does not:

- **Validate length and source size** of submitted scripts before
  compilation. A reasonable upper bound is the smallest program that solves
  the user's problem plus a generous margin; reject anything larger.
- **Compile and run untrusted scripts in a separate process** with a CPU
  time limit, a memory cap, a writable scratch directory, a network
  namespace you control, and a watchdog that reaps crashed children. The
  VM does not isolate scripts from each other.
- **Limit concurrent script execution** with a process-wide semaphore.
  Even benign scripts that spawn green fibers or wait on channels can
  consume memory indefinitely.
- **Pre-register only the host functions the script needs.** A script
  that calls `run_process` because you registered it can do anything
  the host process can. If you only need to filter strings, don't expose
  `run_process` or the TCP builtins. Disable the `tcp`, `concurrency`,
  and `transpiler` cargo features and recompile the engine for the
  untrusted-script deployment.
- **Treat host callbacks as the security boundary.** Scripts only see
  what you register. A filter that validates input against a regex
  before forwarding is safer than a filter that hands the host's full
  application state to the script.

## What the engine does guarantee

- **No file or network access without an explicit builtin call.** A pure
  expression cannot reach the filesystem.
- **Integer overflow is a runtime error** in both the VM and the
  transpiler. There is no wrap-around arithmetic by default.
- **Call depth is bounded at 256** by default. Exceeding it raises a
  runtime error rather than overflowing the stack. To lower the limit
  for untrusted scripts, fork the engine and instrument the depth check.
- **Dynamic value checks** are exhaustive: every variant of `Value` is
  matched, and tagged `Err(message)` from `attempt` is data, not a panic.

## What the engine does not guarantee

- **No memory ceiling per script.** A script that builds `[1, 2, 3, ...]`
  one element at a time in a loop will OOM the host.
- **No CPU budget per script.** `while true {}` runs forever.
- **No filesystem or network isolation.** The engine has the same
  authority as the host process.
- **No panic recovery from host callbacks.** If a registered Rust closure
  panics, the panic propagates to the host process.

## Reporting soundness issues

The bytecode interpreter relies on unsafe-free Rust but generates machine
code paths that share much of the runtime's invariants. Treat any soundness
issue (segfault, undefined behavior, out-of-bounds memory access) as a
high-priority bug. Please file an issue with a reproducer.
-
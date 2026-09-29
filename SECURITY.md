# Security

Rynd runs with the authority of the process that hosts it. A script can do what
its builtins and your registered functions can do, so the question for
untrusted code is which of those it gets.

## Capabilities

A `RyndEngine::new()` script can read and write files (`read_text`, `write_text`,
`read_bytes`, `list_dir`, `mkdir_all`, ...), read stdin and environment
variables, run programs with `run_process`, sleep, and open TCP listeners and
connections. Generated native Rust has the same builtins.

`RyndEngine::sandboxed()` removes all of them
(`rynd::vm::safety::HOST_ACCESS_BUILTINS` lists each one) and keeps the
restriction across `reset()`. What remains is computation: collections, text,
JSON, pattern matching, fibers, channels, and the host functions you register.
`remove_global(name)` removes any other individual builtin.

Registered functions are the security boundary. A script can call exactly what
you install; expose narrow operations (`lookup_price(sku)`) rather than broad
ones (`query(sql)`).

## Limits the engine enforces

- **Call depth** is capped at `rynd::vm::safety::MAX_CALL_DEPTH` (256) in the
  VM, generated Rust, and the JIT. Exceeding it is a runtime error, not a stack
  overflow.
- **Integer overflow**, division by zero, and modulo by zero are runtime errors
  in every backend and build profile.
- **Script errors are values** to the host: every failure is a `RyndError`, and
  the engine stays usable after one.
- **No `unsafe`** in the default build. The opt-in `jit` feature executes
  Cranelift-generated machine code for integer/boolean functions that pass a
  static type check; everything else stays in the bytecode VM.

## Limits the host must enforce

The engine has no instruction budget and no memory quota. Recursion is bounded,
but a script can still iterate over a huge `range`, build large collections, or
wait forever on a channel. For untrusted input:

- Cap source size before compiling.
- Run scripts in a separate process with CPU-time and memory limits
  (`setrlimit`, cgroups, or a container) and kill it on timeout.
- Use `sandboxed()` unless the script genuinely needs host access, and give
  that process its own scratch directory and network policy when it does.
- Catch panics at your host-function boundary; a panic inside a registered Rust
  closure propagates to the caller like any Rust panic.
- `rynd build` and `rynd::build::native` invoke `rustc` on generated code;
  build untrusted scripts only inside the same sandbox.

## Reporting

Report crashes, panics reachable from script source, or memory-safety issues
with a reproducer. Panics in the parser, compiler, or VM on any input are bugs.

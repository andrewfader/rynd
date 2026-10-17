# Security

Rynd runs with the authority of the process that hosts it. A script can do what
its builtins and your registered functions can do, so the question for
untrusted code is which of those it gets.

## Capabilities

A `RyndEngine::new()` script can read and write files (`read_text`, `write_text`,
`read_bytes`, `list_dir`, `mkdir_all`, ...), read stdin and environment
variables, run programs with `run_process`, sleep, open TCP listeners and
connections, serve and fetch HTTP, and open on-disk stores. Generated native
Rust has the same builtins.

`RyndEngine::sandboxed()` removes all of them
(`rynd::vm::safety::HOST_ACCESS_BUILTINS` lists each one) and keeps the
restriction across `reset()`. What remains is computation: collections, text,
JSON, hashing and tokens, pattern matching, fibers, channels, in-memory
stores, and the host functions you register.
`remove_global(name)` removes any other individual builtin.

Registered functions are the security boundary. A script can call exactly what
you install; expose narrow operations (`lookup_price(sku)`) rather than broad
ones (`query(sql)`).

## Limits the engine enforces

- **Call depth** is capped at `rynd::vm::safety::MAX_CALL_DEPTH` (10,000) in
  the VM, generated Rust, and the JIT, and by a stack-headroom check on threads
  Rynd did not start. Exceeding either is a runtime error, not a stack
  overflow. Tail calls do not nest, so a tail-recursive loop runs until it
  returns.
- **Integer overflow**, division by zero, and modulo by zero are runtime errors
  in every backend and build profile.
- **Script errors are values** to the host: every failure is a `RyndError`, and
  the engine stays usable after one.
- **`unsafe` is confined** to two `Send` impls: one that moves `Rc`-based
  values between the threads of a fiber group, and one for the stores a group
  owns. Only one thread of a group runs script code at a time, and the
  handoff goes through a mutex. Stores refuse access from other groups. The
  opt-in `jit` feature also executes Cranelift-generated machine code for
  integer/boolean functions that pass a static type check.
- **HTTP input is bounded**: request heads at 64 KiB (431), bodies at 16 MiB
  (413), idle keep-alive connections at 5 seconds, and header values with
  newlines are refused on both server and client.
- **Passwords** use salted PBKDF2-HMAC-SHA256 with 100,000 rounds; hash and
  token checks compare in constant time. Randomness comes from `/dev/urandom`,
  and a missing random source is an error, never a silent fallback.

## Limits the host must enforce

The engine has no instruction budget and no memory quota. Recursion is bounded,
but a script can still iterate over a huge `range`, build large collections, or
keep a fiber spinning without yielding. Waits that can never complete are
reported as deadlocks, but a fiber blocked on a socket waits as long as the
socket does. For untrusted input:

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

# Implementation review

Reviewed 2026-09-26: syntax, module loading, bytecode compilation and execution,
shared runtime, Rust generation, Cargo scaffolds, CLI/REPL, embedding, examples,
conformance checks, and benchmark harness.

## Supported workflows

Rynd supports functional application logic that integrates with Rust and ships
through Cargo. Immutable values, closures, pipelines, comprehensions, pattern
matching, modules, source locations, reusable compiled scripts, and host callbacks
work in the VM and compiled applications. The data helpers include JSON,
sorting, grouping, collection search, and explicit runtime error recovery.

[REQUIREMENTS.md](REQUIREMENTS.md) maps workflows to tests.
[LANGUAGE.md](LANGUAGE.md) specifies syntax and execution semantics.

## Changes from this review

Both compilers now use shared lexical free-name analysis to select closure
captures. A closure retains the outer locals referenced by its body, including
references needed by nested closures. Parameters, local declarations, pattern
bindings, and comprehension variables shadow outer names at their lexical scope.
Initializers see the preceding bindings. Recursive functions retain their self
reference, and globals resolve through the runtime.

This releases unused host values when a factory function returns and avoids
cloning unrelated local values while creating native closures. Tests check host
reference lifetimes and execute the same scope-sensitive programs in the VM and
optimized native binaries. Cargo scaffolds include the shared analysis module.

Typed Rust conversions now cover scalar values, borrowed strings, and inbound
lists/maps. Adapters use `TryFrom` to report type errors through normal runtime
recovery; exact scalar matching preserves integer precision. The embedding
example exercises conversion from script values into Rust booleans.

The conformance runner preserves executable checks for numeric behavior,
destructuring, captures, short-circuit evaluation, Unicode indexing, error exits,
REPL recovery, embedding, examples, and benchmark compiler failures. It writes
results to `target/conformance/results.json`.

The scripting toolkit adds immutable batch and zip operations, text filters and
replacement, JSON Lines, byte/file helpers, and explicit process execution. A
streaming CLI reuses compiled expressions per record. Single-quoted strings,
conditional statement modifiers, word operators, and destructured parameters
support these workflows.

The source debugger supports conditional breakpoints, stepping, exception
inspection, lexical values, frame selection, multiline expressions, and timing.
A separate debug loop keeps hooks out of normal bytecode dispatch. REPL commands
add history/replay, loading, inspection, completion lists, and timing.

## 2026-10-17: concurrent fibers, HTTP, and a reference application

Fibers used to run one after another, but now run concurrently. Each fiber has its own thread
and stack, and a baton serializes script execution within a program. The
baton moves at waits (`await_fiber`, `receive`, `channel_recv`, nursery joins,
`yield_fiber`, `sleep_ms`) and around blocking I/O. Mailboxes and channels block
instead of failing when empty, nurseries cancel running siblings, and
unsatisfiable waits raise `Deadlock` errors. Both the VM and generated Rust fork
a runtime per fiber (`Runtime::fork`); output capture is shared.

Sockets moved I/O outside their registry lock and release the baton while
blocked. A new HTTP module serves HTTP/1.1 (a fiber per connection, keep-alive,
pipelining, chunked bodies, 400/413/431/500) and makes client requests. Query
strings, URL decoding, and percent-decoded route params feed the existing
Plug helpers, and `socket_read_chunk` now decodes chunk framing.

Crypto, time, and storage builtins close the remaining gaps for backend work
without dependencies: SHA-256, HMAC, PBKDF2, base64, HS256 JWTs (RFC vectors in
unit tests), OS randomness, RFC 3339 time, slugs, and an append-only document
store with torn-write recovery and compaction.

Recursion is the loop construct, so tail calls now reuse frames: all tail calls
in the VM, and self tail calls in generated Rust and the JIT. The depth limit
rose from 256 to 10,000. Program threads get large lazily-committed stacks,
and a stack-headroom guard covers host threads of unknown size.

Generated standalone code is compiled by plain `rustc`, i.e. edition 2015, so
runtime modules avoid let-chains and 2024-only temporary lifetimes.
`examples/conduit` implements the RealWorld API, and `tests/conduit_test.rs`
drives it over HTTP on the VM and as a native binary, including a restart and
a stalled client.

## Verification

```sh
sh scripts/check.sh
sh scripts/coverage.sh
cargo bench --offline --bench rynd_benchmarks
```

The check script runs formatting, Clippy with warnings denied, debug and release
tests, the embedding example, VM/native conformance checks, and independent
Python JSON comparisons. Coverage enforces a 90% source-line threshold and writes
its report to `target/coverage/coverage.json`.

Final validation: 150 tests pass in debug and release. Formatting and Clippy
with warnings denied pass. The conformance suite passes 37 VM/native programs,
three REPL sessions, embedding, and two benchmark-failure checks. Independent
JSON checks pass 124 inputs and 20 rejections on each target. Source-line
coverage is 92.35% (4,938/5,347), above the 90% gate.
See [BENCHMARKS.md](BENCHMARKS.md) for new workload timings and baseline comparisons.

## 2026-10-17 (later): interpreter speed, coroutine fibers, and resource limits

**Interpreter.** Instructions are `Copy` with operands indexed into per-chunk
tables (constants, names, patterns), so dispatch no longer clones reference
counts. Frames keep their locals in slots of the shared operand stack, with
arguments landing in place; tail calls move the callee and arguments down
over the returning frame. Globals live in numbered slots that each machine
links once per chunk, replacing a string hash per lookup. Integer arithmetic
and comparisons have inline fast paths; integer literals are immediates, and
`if` conditions and match guards fuse their comparison into the jump.
`Value` shrank from 48 to 16 bytes: every non-scalar is one `Rc` (variants,
closures, host functions), and builtins are a pointer into their static table,
so dispatch to a builtin's module no longer searches by name. Generated Rust
binds parameters by reference and skips unused self clones.

**Fibers.** Fibers are now stackful coroutines on the program's thread
(`vm/fiber.rs`: lazily committed stacks with guard regions, a short assembly
context switch for x86-64 and AArch64, Linux and macOS). Sockets are
non-blocking and park their fiber on a `poll` reactor; name resolution,
processes, and stdin run on helper threads. Waits register on precise keys
(a fiber, mailbox, or channel) instead of re-checking every parked fiber, and
timers, I/O waits, and cancellation all wake the right fiber. Cancellation now
interrupts socket waits. Script values never cross threads, which removes the
`Send` impls the thread-per-fiber design needed (stores became thread-local).
Scheduling stays cooperative, preserving the atomicity of code between waits.

**Limits.** `Limits` bounds operations, wall time, allocation, string and
collection size, call depth, source size, and live fibers per run, and
`Interrupt` stops a run from another thread. `sandboxed()` applies
`Limits::sandbox()`. Builtins that can expand shared structure check sizes
before allocating, `attempt` cannot catch a limit, and the budget follows a
run across its fibers' waits. Independently of limits, the parser bounds
nesting at 200 levels, and deeply nested values drop, compare, and print
without native recursion, so neither hostile source nor hostile data can
overflow the stack.

Bytecode listings name their operands (`GetGlobal(fib)`, `SubImm(1)`), and
`Value` patterns for variants and callables changed shape (`Value::Variant(v)`
with `v.name`/`v.values`, constructors `Value::compiled`, `Value::closure`,
...); `Value::List`/`Map` payloads deref to `Vec`/`BTreeMap` as before.
`tests/sandbox_test.rs` and `tests/fiber_runtime_test.rs` cover the new
behavior; see [BENCHMARKS.md](BENCHMARKS.md) for timings.

## Development directions

Future development can extend editor integration, formatting with comment
preservation, lazy collection protocols, optional preemption between calls,
non-blocking connects without a helper thread, fiber support on more
platforms, and richer typed Rust adapters. Release packaging and a project
license are distribution decisions.

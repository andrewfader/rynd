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

## Resource limits

`RyndEngine::sandboxed()` also applies `Limits::sandbox()` to every run, and
`engine.set_limits(limits)` applies limits to any engine. Each run (`eval`,
`run`, `call`, `filtermap`, ...) gets a fresh budget:

| `Limits` field | Sandbox default | What it bounds |
| --- | --- | --- |
| `operations` | 50,000,000 | Function calls, plus one unit per element a builtin reads or builds and per hash round. Bytecode has no backward jumps, so every loop is a call and is counted. |
| `time` | 5 s | Wall-clock time, including time fibers spend waiting. |
| `allocation` | 512 MiB | Bytes of strings, lists, maps, and variants created during the run (counted at creation, not credited when freed). |
| `string_bytes` | 16 MiB | The longest string a script may build. |
| `collection_len` | 1,000,000 | Elements in one list, tuple, or map. |
| `call_depth` | 1,000 | Nested script calls (never above 10,000). |
| `source_bytes` | 1 MiB | Source text passed to `compile`/`eval`. |
| `fibers` | 1,000 | Fibers alive at once. |

Builtins that could expand a small value into a huge one check their output
size before allocating: `range`, string `+` and `*`, `join`, `replace`,
`concat`, `push`, `flatten`, `windows`, `to_string`, printing, and `to_json`
(a list sharing one large list a thousand times cannot print gigabytes).
Quadratic builtins such as `uniq` are charged for their work.

Exceeding a limit raises `RyndError::LimitExceeded` (`error.is_limit()`). The
run cannot recover from it: `attempt` re-raises it, and every later check in the
run fails again, so the host always sees it. The engine stays usable for the
next run. `engine.interrupt_handle()` returns an `Interrupt` another thread can
trigger (a watchdog, a cancelled request); the run stops at its next check, at
most about a thousand calls or 50 ms of waiting later. The opt-in `jit` feature
compiles nothing for limited engines, since native code would skip the checks.

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
- **Nesting cannot overflow the stack**: the parser rejects source nested
  more than 200 levels (brackets, blocks, patterns, or chained operators), and
  values nested arbitrarily deep (a million-level list built by a loop) drop,
  compare, and print without recursing on the native stack.
- **`unsafe` is confined** to the fiber layer (`vm/fiber.rs`: stack mappings,
  `poll`, and the context switch; `vm/concurrency.rs`: handing a blocking call
  to a helper thread while its fiber waits). Script values never leave the
  thread that created them. The opt-in `jit` feature also executes
  Cranelift-generated machine code for integer/boolean functions that pass a
  static type check.
- **HTTP input is bounded**: request heads at 64 KiB (431), bodies at 16 MiB
  (413), idle keep-alive connections at 5 seconds, and header values with
  newlines are refused on both server and client.
- **Passwords** use salted PBKDF2-HMAC-SHA256 with 100,000 rounds; hash and
  token checks compare in constant time. Randomness comes from `/dev/urandom`,
  and a missing random source is an error, never a silent fallback.

## Limits the host must enforce

The allocation limit counts what a run creates, not what is live, and is an
estimate; it is not a hard memory quota. Host functions run outside the
budget: a registered function that loops or allocates is yours to bound. For
hostile input, defense in depth still applies:

- Use `sandboxed()` (or `set_limits`) unless the script genuinely needs
  unlimited resources, and give scripts that need host access their own
  scratch directory and network policy.
- For input that may be actively malicious, also run the engine in a separate
  process with CPU-time and memory limits (`setrlimit`, cgroups, or a
  container) and kill it on timeout.
- Catch panics at your host-function boundary; a panic inside a registered Rust
  closure propagates to the caller like any Rust panic.
- `rynd build` and `rynd::build::native` invoke `rustc` on generated code;
  build untrusted scripts only inside the same sandbox.

## Reporting

Report crashes, panics reachable from script source, or memory-safety issues
with a reproducer. Panics in the parser, compiler, or VM on any input are bugs.

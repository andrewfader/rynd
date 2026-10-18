# Rynd supported workflows

The following workflows are implemented and exercised by executable checks.

| Requested outcome | Delivered behavior | Evidence / entry points |
| --- | --- | --- |
| Language and runtime names | `rynd` crate/CLI, `.rynd` sources, `RyndEngine`, `RyndError`, `RyndResult`; Meso identifies the VM/runtime | Cargo.toml, CLI version/help, all current examples/tests |
| A functional language layer over Rust | Expression-valued blocks, first-class functions, captures, immutable data, pipelines, structural patterns, guards, tagged values, comprehensions | Parser/VM tests, BDD scenarios, optimized native E2E tests |
| Applications and libraries | `rynd new`, `new --lib`, Cargo build/run/test/check, generated library exports | tests/project_test.rs builds and moves an application, builds a library, and consumes it from an independent Rust crate |
| Reusable modules | Relative imports, explicit `pub` exports, isolated private names, canonical dependency deduplication, cycle diagnostics | tests/modules_test.rs; examples/orders |
| Rust ecosystem interoperability | Rust crate dependencies and native adapters callable from Rynd; Rust calls compiled Rynd exports using shared values/errors | Generated scaffold tests and project test using a separate Rust dependency |
| Native compilation to Rust | Direct Rust expressions/closures; std-only standalone output; Cargo build.rs generation; one-command native `build` | Native E2E tests, module native test, project integration tests, CLI native build test |
| Ruby/Elixir/Coffee-inspired ergonomics, including ternaries | Short-circuit `? :`, `unless`, nested `#{...}` interpolation, `|>`, comprehensions, pattern matching, `?.`, `?:` | inspired_expression_semantics native parity test; LANGUAGE.md precedence and truthiness contract |
| Input-driven programs | UTF-8 file/stdin input, script arguments, string helpers, numeric conversion, output | sales_report.rynd, CLI file/stdin/native workflow tests |
| Rust embedding | Typed value exchange, stateful callbacks, function calls, output capture/drain, recovery after errors, compile-once handles | examples/embedded_rules.rs; usability and module tests |
| Typed host API | `register` for Rust fns/closures (0–8 args), `FromValue`/`IntoValue`, `record!` struct bridge and contexts, `register!`, `Library`, `ScriptRuntime`, atomic `Package::reload`, in-memory `SourceTree` imports, `Accept`/`Reject` filter-map verdicts, `cli::main_with` | tests/host_test.rs, embedding_test.rs, filtermap_verdict_test.rs; examples/embedded_rules.rs |
| Untrusted-script hygiene | `RyndEngine::sandboxed()` removes host-access builtins and applies per-run `Limits` (operations, time, allocation, string and collection size, call depth, source size, fibers) that `attempt` cannot catch; `Interrupt` handles; bounded syntax nesting; deep values drop/print/compare without native recursion; shared call-depth limit across VM, native, JIT | tests/safety_test.rs; tests/sandbox_test.rs; SECURITY.md |
| Concurrent fibers | Stackful coroutines on the program's thread, a `poll` reactor for sockets, helper threads for resolution/processes/stdin, switch points at waits and I/O, fail-fast nurseries that cancel even I/O waits, blocking mailboxes/channels, deadlock errors, independent schedulers per host thread | tests/concurrency_test.rs; tests/fiber_runtime_test.rs; fiber conformance cases |
| Web services | `serve` (fiber per connection, keep-alive, chunked bodies, 400/413/431/500 handling), `fetch`/`http_request`, query and URL decoding, Plug helpers, streaming responses | tests/http_test.rs; examples/sinatra_web_server.rynd |
| Backend building blocks | SHA-256/HMAC/PBKDF2/base64/JWT against RFC vectors, OS randomness and UUIDs, RFC 3339 time, slugs, an append-only document store with torn-write recovery and compaction | src/vm/crypto.rs unit tests; tests/stdlib_test.rs |
| A reference web application | The RealWorld Conduit API (users, JWT auth, profiles, follows, articles, feed, favorites, comments, tags, CORS) persisted across restarts, on the VM and as a native binary | tests/conduit_test.rs; examples/conduit |
| Recursion as iteration | Tail calls reuse frames (VM: all; native and JIT: self tail calls); 10,000-frame depth limit on large program stacks; stack-headroom guard on foreign threads | tests/modules_test.rs, jit_test.rs; tail-call conformance cases |
| Opt-in native JIT | `--features jit` + `enable_jit()` compiles statically typed int/bool functions with Cranelift, VM fallback for everything else, identical errors | tests/jit_test.rs |
| Execution performance | Borrowed native call arguments; shared instruction metadata; deferred error-span cloning; reusable compilation | Benchmark harness measures complete VM, reusable VM, native execution, startup, frontend, and a typed Rust baseline |
| E2E and BDD-style verification | Behavior scenarios plus real CLI/native/Cargo/library workflows, exact stdout/stderr/status checks, independent expected results | tests/bdd_scenarios.rs, e2e_transpiler_test.rs, cli_test.rs, modules_test.rs, project_test.rs |
| High coverage and checked assumptions | Enforced 90% source-line gate, debug/release suites, warnings-as-errors Clippy, VM/native conformance checks | scripts/check.sh, scripts/coverage.sh, scripts/check_conformance.py |
| Reliable build failures | Generated projects build/run/test; missing compiler, bad imports, bad input, and failed compilation report errors | Failure-path tests and benchmark compiler-failure checks |

## Reproduce the result

```sh
sh scripts/check.sh
sh scripts/coverage.sh
cargo bench --offline --bench rynd_benchmarks
cargo install --offline --path .
rynd new hello
rynd run hello --offline -- Ada
rynd test hello --offline
rynd build hello --offline --release
```

The check script also runs the embedded Rust example and VM/native conformance checks.
Project tests verify moving a scaffold, adding a real Rust dependency, changing an
imported Rynd file, and rejecting source errors instead of reusing stale output.
The native build test verifies that missing compilers preserve an existing output
and that generation cannot overwrite the input source.

Closures capture the lexical values their bodies use, including references in
nested closures. Tests cover shadowing, recursion, patterns, comprehensions, and
release of unused host values.

Typed Rust adapters use checked scalar conversions and borrowed strings; inbound
Rust lists and maps convert recursively. See tests/value_conversions.rs.

Streaming line processing, batch/text/system helpers, syntax conveniences, and
interactive debugging are covered by tests/scripting_tools.rs,
tests/debugger_test.rs, and the CLI/native suites. See [SCRIPTING.md](SCRIPTING.md)
and [DEBUGGING.md](DEBUGGING.md) for commands and semantics.

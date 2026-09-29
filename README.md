# Rynd

A small functional language for Rust applications. Write `.rynd`, compile to Rust,
let Cargo handle the rest. The build system already has a job.

Dynamic values, immutable bindings, pipelines, closures, pattern matching, and
comprehensions. Rust crates supply the native capabilities; **Meso**, the bytecode
VM, handles interactive execution and embedding.

```rynd
fn total_with_tax(prices, rate) {
    prices
        |> map(\price -> price * (100 + rate) / 100)
        |> sum()
}

let cents = total_with_tax([100, 250, 400], 8)
println("Total: #{cents} cents") # Total: 810 cents
```

## Try it

From this checkout, with Rust edition 2024 support:

```sh
cargo install --path .
rynd eval 'range(1, 100) |> filter(\x -> x % 5 == 0) |> sum()'
rynd run examples/sales_report.rynd -- examples/data/sales.tsv 2000
rynd repl
```

JSON in, useful data out:

```sh
printf '[3,1,2]' | rynd eval 'read_stdin() |> parse_json() |> sort() |> to_json()'
```

Sort, group, search, and transform collections. Recover with `attempt` and
`Ok`/`Err`. Try the [JSON report](examples/json_report.rynd) on real records.

The toolchain uses Rust's standard library. The dependency tree is a stump
(the opt-in `jit` feature grows a Cranelift branch).

## Work with records

```sh
rynd lines "upper(line) if contains(line, 'warn')" app.log
rynd lines 'parse_json(line).name' people.jsonl
rynd debug examples/debug_rules.rynd
```

Pair lists with `zip`, destructure callback parameters, batch with `chunks`,
transform text, and invoke programs with `run_process`. The REPL and debugger
include introspection, history, and expression benchmarks.

## Concurrency and web services

Run lightweight green fibers, coordinate tasks with fail-fast structured
nurseries, isolate state with actor mailboxes, and stream with CSP channels:

```rynd
# Structured nursery: runs tasks concurrently, canceling siblings if any fail
let results = nursery(\n -> {
    n.spawn(\ -> fetch("https://api-1.local"))
    n.spawn(\ -> fetch("https://api-2.local"))
})
```

Build modular web services and micro-frameworks using native TCP sockets,
Sinatra/Phoenix-style Plugs, route pattern matching, and HTTP streaming:

```rynd
fn router(conn) {
    let m = match_route(conn.path, "/api/nodes/:id")
    if m.tag == "Some" and conn.method == "GET" {
        return json_response(conn, 200, {"id": m.value.id, "status": "healthy"})
    }
    text_response(conn, 404, "Not Found")
}

let response = conn("GET", "/api/nodes/worker-1") |> router()
```

Inspect the [orchestration platform](examples/orchestrator_platform.rynd) and
[Sinatra web server](examples/sinatra_web_server.rynd) examples.

## Ship it

```sh
rynd new hello
rynd run hello --offline -- Ada
rynd test hello --offline
rynd build hello --offline --release
```

You get an ordinary Cargo project: `.rynd` modules, Rust adapters in
`src/native.rs`, and a pinned compiler snapshot. Add crates to `Cargo.toml`;
register adapters using `rynd::Value`. Cargo handles compilation and rebuilds.

Use `rynd new rules --lib` for a library. Rust callers access Rynd exports through
`Application::call` and `Application::get`.

For a standalone executable:

```sh
rynd build examples/fibonacci.rynd -o target/fib
./target/fib
```

`rynd compile` emits Rust source for inspection. Yes, you can read the lowering.

## Embed it

```rust
use rynd::{RyndEngine, Value};

let mut engine = RyndEngine::new();
engine.set_global("prices", Value::list(vec![Value::Int(100), Value::Int(250)]));
assert_eq!(engine.eval("prices |> sum()").unwrap(), Value::Int(350));
```

Rust functions register as they are; conversions are inferred from the signature:

```rust
engine.register("clamp", |x: i64, lo: i64, hi: i64| x.clamp(lo, hi));
rynd::record! { pub struct Order { pub id: i64, pub total: f64 } } // derive, no proc-macro crate
```

Bundle host APIs in a `Library`, hot-reload rules with `Package::reload`, resolve
imports from an in-memory `SourceTree`, return `Accept(x)`/`Reject(x)` from
filter-map scripts, and hand less-trusted code a `RyndEngine::sandboxed()`.
`rynd::cli::main_with(|engine| ...)` ships the whole CLI with your functions
built in. Need integer hot loops at machine speed? `--features jit` and
`engine.enable_jit()`: Cranelift compiles what it can prove, the VM runs the rest.
See the [embedded rules example](examples/embedded_rules.rs).

## Documentation

- [Language](LANGUAGE.md) — syntax, semantics, and Rust interop.
- [Scripting](SCRIPTING.md) — streaming lines, batches, text, files, and processes.
- [Interactive development](DEBUGGING.md) — REPL, debugger, and expression benchmarks.
- [Examples](examples/) — pipelines, modules, reports, and embedding.
- [Benchmarks](BENCHMARKS.md) — timings with the methodology attached.
- [Requirements](REQUIREMENTS.md) — supported workflows and verification.
- [Codebase review](REVIEW.md) — implementation decisions and development priorities.
- [Security](SECURITY.md) — sandboxing guidance for untrusted scripts.

```sh
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
```

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

Fibers are stackful coroutines on one thread: a couple of microseconds to
start, a fraction of one to switch, and ten thousand at once is routine.
Sockets wait in a single `poll` loop, so one slow client never stalls the rest.
Nurseries cancel siblings on the first failure (even mid-read), actors own
mailboxes, channels block until fed, and a wait that can never finish is
reported as a deadlock instead of a hang:

```rynd
let results = nursery(\n -> {
    n.spawn(\ -> fetch("http://127.0.0.1:8080/api/health").status)
    n.spawn(\ -> fetch("http://127.0.0.1:8081/api/health").status)
})
```

`serve` speaks HTTP/1.1 with keep-alive and chunked bodies, one fiber per
connection. Plugs are functions from conn to conn:

```rynd
fn app(conn) {
    let m = match_route(conn.path, "/api/nodes/:id")
    if m.tag == "Some" and conn.method == "GET" {
        return json_response(conn, 200, {"id": m.value.id, "status": "healthy"})
    }
    text_response(conn, 404, "Not Found")
}
serve("127.0.0.1:8080", app)
```

The batteries a backend reaches for are built in, with no crates: SHA-256 and
HMAC, PBKDF2 password hashes, HS256 JWTs, base64, UUIDs, RFC 3339 time, and
a crash-safe document store. Tail calls loop in constant space, which matters
in a language where recursion is the loop.

[examples/conduit](examples/conduit) is the [RealWorld](https://realworld-docs.netlify.app)
"Conduit" API in about 500 lines of Rynd, tested over HTTP on the VM and as a
native binary:

```sh
PORT=3000 JWT_SECRET=change-me rynd run examples/conduit/main.rynd
rynd build examples/conduit/main.rynd -o conduit && ./conduit
```

Also: the [orchestration platform](examples/orchestrator_platform.rynd) and the
[Sinatra-style server](examples/sinatra_web_server.rynd).

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
filter-map scripts, and hand less-trusted code a `RyndEngine::sandboxed()`:
no host access, and per-run limits on work, time, memory, sizes, depth, and
fibers that scripts cannot catch, plus an `Interrupt` handle for watchdogs.
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

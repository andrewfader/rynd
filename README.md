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

The toolchain uses Rust's standard library. The dependency tree is a stump.

## Work with records

```sh
rynd lines "upper(line) if contains(line, 'warn')" app.log
rynd lines 'parse_json(line).name' people.jsonl
rynd debug examples/debug_rules.rynd
```

Pair lists with `zip`, destructure callback parameters, batch with `chunks`,
transform text, and invoke programs with `run_process`. The REPL and debugger
include introspection, history, and expression benchmarks.

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

Register Rust callbacks, call script functions, or compile once and run repeatedly.
Use one engine per thread for trusted scripts. See the
[embedded rules example](examples/embedded_rules.rs) for host state and callbacks.

## Documentation

- [Language](LANGUAGE.md) — syntax, semantics, and Rust interop.
- [Scripting](SCRIPTING.md) — streaming lines, batches, text, files, and processes.
- [Interactive development](DEBUGGING.md) — REPL, debugger, and expression benchmarks.
- [Examples](examples/) — pipelines, modules, reports, and embedding.
- [Benchmarks](BENCHMARKS.md) — timings with the methodology attached.
- [Requirements](REQUIREMENTS.md) — supported workflows and verification.
- [Codebase review](REVIEW.md) — implementation decisions and development priorities.

```sh
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
```

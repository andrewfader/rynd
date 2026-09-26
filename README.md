# Rynd

Rynd is a functional language layer for **Rust applications and libraries**.
Write application logic in `.rynd` modules, compile it to Rust with Cargo, and use
Rust crates for native capabilities. Its expression-oriented syntax combines
pipelines, immutable values, closures, pattern matching, comprehensions,
interpolation, ternaries, and safe navigation.

**Rynd** names the language and toolchain. **Meso** names its bytecode VM/runtime
component, used for interactive execution and embedding. The CLI and Rust crate
are both `rynd`; the main embedding type is `RyndEngine`.

The native target emits Rust expressions and closures with an embedded dynamic
value runtime. It does not ship the parser or execute bytecode. Both targets share
arithmetic, comparison, pattern matching, indexing, and builtin implementations.
This is a dynamic language; it does not promise zero allocations or the performance
of handwritten, statically typed Rust.

There are three workflows: Cargo applications/libraries, standalone native
programs, and VM execution/embedding. The same frontend and value semantics serve
all three. Rynd is an early language with explicit dynamic boundaries to Rust;
it does not replace Rust's ownership/type system or implement Elixir's actor/OTP
runtime. See [LANGUAGE.md](LANGUAGE.md) for precise semantics and
[REQUIREMENTS.md](REQUIREMENTS.md) for the requested capabilities and evidence.

## Build and run

The project has no third-party crate dependencies. It uses Rust edition 2024 and
is tested with Rust 1.97.0. Build the CLI locally, or install it with Cargo:

```sh
cargo build --release
cargo install --path .
rynd run examples/pipeline_data.rynd
rynd eval 'range(1, 100) |> filter(\x -> x % 5 == 0) |> sum()'
rynd repl
```

## Build a Rust-backed application or library

```sh
rynd new hello
rynd run hello --offline -- Ada
# Hello, Ada!
rynd test hello --offline
rynd build hello --offline --release

rynd new rules --lib
rynd build rules --offline --release
```

These are ordinary Cargo projects. You can also run `cargo build`, `cargo test`,
and `cargo run` inside them. Project commands forward Cargo options; file commands
use the direct compiler. The generated layout is:

```text
Cargo.toml              Rust crate dependencies and package configuration
build.rs                Rynd-to-Rust compilation, with dependency tracking
src/main.rynd           Rynd entry module and public exports
src/greetings.rynd      An imported Rynd module
src/native.rs           Rust functions registered for Rynd to call
src/lib.rs              Rust API for the compiled Rynd exports
src/main.rs             Application entry point (omitted for --lib)
.rynd/compiler/         Pinned, portable compiler/runtime source snapshot
```

Add ordinary Rust dependencies to `Cargo.toml`, call them in `src/native.rs`, and
register functions that accept/return `rynd::Value`. Rynd calls them like normal
functions. Rust consumers use the generated `Application::new(args)`,
`call(export, values)`, and `get(export)` API; `Application::exports()` lists the
public interface. The generated library re-exports `Value` and `RyndResult`.
Compile-time errors stop Cargo builds; imported-file edits trigger regeneration.

The scaffold includes a Rust→Rynd→Rust integration test and a vendored toolchain
so it builds offline and after being moved, without an unpublished registry
dependency. Commit that snapshot and Cargo.lock. In a workspace with several
Rynd crates, configure them to share one `rynd` runtime dependency path so their
`Value` types have the same Rust crate identity. Upgrade the pinned compiler
deliberately rather than mixing snapshots. Only `rynd new` copies the toolchain;
compiled executables do not need the CLI, compiler, or source files installed.

Compile a standalone program directly:

```sh
rynd build examples/fibonacci.rynd -o target/fib
./target/fib
```

Or inspect/edit the generated Rust yourself:

```sh
rynd compile examples/fibonacci.rynd -o target/fib.rs
rustc -O target/fib.rs -o target/fib
./target/fib
```

Standalone generated programs use only Rust's standard library; Cargo projects
can add Rust dependencies. Errors include Rynd source files and positions and
exit with status 1 on both targets. File-mode `rynd run` and standalone binaries
print the final non-nil expression, in addition to explicit script output.
`rynd eval` and the REPL also display nil results.

## Modules and concise expressions

In `math.rynd`:

```rynd
let multiplier = 2
pub fn double(x) { x * multiplier }
pub fn describe(x) { x >= 0 ? "nonnegative" : "negative" }
```

In another file:

```rynd
import "math.rynd" as math
let values = [1, 2, 3] |> map(math.double)
println("Doubled: #{values}; total: #{sum(values)}")
unless contains(values, 0) { println("No zero values") }
```

Imports are relative to the importing file. `pub fn` and `pub let` expose module
members; other declarations are private. Dependencies initialize once per loaded
module graph, in dependency order; cycles are rejected with an import chain.
Private names do not leak between modules. File commands, `eval_file`,
`compile_file`, and Cargo builds resolve imports; `eval` of anonymous text does
not have a module base directory. A Cargo application's generated Rust entry
point calls the root's `pub fn main()`; file-mode scripts execute their top-level
expressions, so call `main()` explicitly there if needed.

## A useful script: summarize a sales file

The included [sales report](examples/sales_report.rynd) reads simple tab-separated
customer names and integer amounts in cents. It reports the total and customers
whose sales reach a configurable threshold:

```sh
rynd run examples/sales_report.rynd -- examples/data/sales.tsv 2000
# Or read the same records from a pipe:
cat examples/data/sales.tsv | rynd run examples/sales_report.rynd
```

Both produce:

```text
Sales: 3
Total cents: 7100
Customers at or above 2000 cents: Grace, Linus
```

This format has no header or quoting; use a host library for full CSV parsing.
Blank lines are ignored, and malformed rows or amounts cause a nonzero exit.
The same script works as a standalone binary:

```sh
rynd compile examples/sales_report.rynd -o target/report.rs
rustc -O target/report.rs -o target/report
./target/report examples/data/sales.tsv 2000
```

For a quick shell transformation:

```sh
printf '10\n20\n30\n' | rynd eval 'read_stdin() |> lines() |> map(parse_int) |> sum()'
# 60
rynd check examples/sales_report.rynd
```

`args` is a list of script argument strings, excluding the program/script name.
Use `--` before arguments to `rynd eval`; it is optional for `rynd run`.
Generated binaries receive their arguments directly. Embedded engines start with
an empty `args` list, which the host can replace. `rynd run -` and `rynd compile -`
read source from stdin; that consumes stdin, so use a file or `eval` when piping
data to `read_stdin()`. `rynd check` validates parsing and compilation without
executing code; dynamic names and types are checked when the program runs.

The REPL accepts multiline functions, collections, strings, and incomplete
expressions. Use `:help`, `:cancel` to discard incomplete input, `:reset` to clear
the session, and `:quit` to exit. Prompts appear only on a terminal.

## Language

Pipelines insert their left side as the first argument: `x |> f(y)` is `f(x, y)`.

```rynd
let numbers = range(1, 21)
let total = numbers
    |> filter(\x -> x % 2 == 0)
    |> map(\x -> x * 10)
    |> sum()
println(total) # 1100
```

Both list and map comprehensions support a filter and lexical captures:

```rynd
fn scale(factor) {
    [x * factor for x in 1..=4 if x % 2 == 0]
}
let squares = {to_string(x): x * x for x in 1..=4}
println(scale(10)) # [20, 40]
println(squares)   # {"1": 1, "2": 4, "3": 9, "4": 16}
```

Functions and lambdas are first-class values. Local values are captured by value;
reference-counted strings and collections make cloning immutable values cheap.
Nested closures and local recursive functions work on both targets:

```rynd
fn make_adder(offset) { \x -> x + offset }
let add_ten = make_adder(10)
println([1, 2] |> map(add_ten)) # [11, 12]
```

Patterns support literals, variables, `_`, nested tuples/lists, and tagged
`Some`, `None`, `Ok`, and `Err` values. Match bindings are local to their arm:

```rynd
let Some((name, scores)) = Some(("Ada", [10, 20]))
let answer = match Some(42) {
    Some(x) if x > 0 => x,
    _ => 0
}
println(answer) # 42
```

A failed `let` pattern is an error; an unmatched `match` evaluates to nil. Blocks
return their last expression unless it ends in a semicolon. Explicit `return`
is valid inside functions. `let` bindings are immutable; use a new binding to
shadow a value. `let mut` is rejected rather than silently accepting unsupported
mutation. Top-level names persist between engine evaluations; functions resolve
global names at call time. Local captures retain their declaration-time values.

Maps use string keys (other key values are formatted as strings), deterministic
sorted display, and last-entry-wins duplicate keys. `{}` is an empty map; an empty
function or conditional block returns nil. List/tuple/string indexing accepts
negative indices and returns nil out of bounds. String indexing and `len` count
Unicode scalar values, not bytes or grapheme clusters.

`?.` returns nil for absent fields and non-record values. `?:` uses **truthiness**,
like Groovy, rather than Kotlin's strictly null-only fallback:

```rynd
let user = {"name": "Grace Hopper"}
println(user?.title ?: "Engineer") # Engineer
println(0 ?: 99)                    # 99
println(Some(42) ?: 99)             # Some(42), not an unwrapped integer
```

Nil, false, numeric zero, NaN, empty strings/lists/maps, `None`, and `Err` are
falsey. `Some(x)` uses `x`'s truthiness. Tuples and functions are truthy.
`&&`, `||`, and `?:` short-circuit and evaluate their left operand once.

Integers are signed 64-bit with checked arithmetic: overflow and integer division
or remainder by zero are errors in debug and release builds. Floats use IEEE 754
arithmetic and exact equality, with numerically consistent mixed integer/float
comparisons, including integers above the exact f64 range. Decimal and scientific
notation are supported. Ranges `a..b` exclude the upper bound; `a..=b` include it.

Builtins: `map`, `filter`, `reduce`, `sum`, `range`, `len`, `head`, `tail`, `push`,
`concat`, `to_map`, `to_string`, `abs`, `min`, `max`, `print`, and `println`.
`map`/`filter` accept unary callbacks; `reduce(list, initial, callback)` accepts a
binary callback. Collection transforms return new values without changing their
inputs. Updates copy vectors when constructing new lists; this is not a
persistent-tree collection implementation.

Text and input helpers work on both execution targets:

| Function | Behavior |
| --- | --- |
| `lines(text)` | Split LF/CRLF lines; omit a final empty line. Empty input gives `[]`. |
| `split(text, separator)` | Literal separator; retain empty fields. An empty separator splits at Unicode scalar boundaries, including empty first/last fields. |
| `trim(text)` | Remove leading/trailing Unicode whitespace. |
| `join(strings, separator)` | Join a list of strings; reject non-string elements. |
| `parse_int(text)` | Trim and parse a signed decimal i64; reject malformed values and overflow. |
| `parse_float(text)` | Trim and parse a finite f64, including scientific notation. |
| `contains(value, item)` | Check a substring, list/tuple membership, or a string map key. |
| `read_text(path)` | Read an entire UTF-8 file; report read/encoding errors. |
| `read_stdin()` | Read all remaining UTF-8 stdin until EOF; intended for `run`/`eval` and native binaries. |

Conversions raise runtime errors on invalid input. Input and collection helpers
materialize their data in memory; they are intended for modest data sets, not
streaming large files. File/stdin reads are available to scripts in embedded
engines too, so run trusted code only.

Semicolons are optional between statements. A call/index opening on a new line
starts a new expression; pipelines can continue across lines. Comments start with
`#` or `//`.

## Embed in Rust

```rust
use rynd::{RyndEngine, Value};

let mut engine = RyndEngine::new();
engine.register_fn("calculate_tax", 1, |args| {
    match &args[0] {
        Value::Int(subtotal) => Ok(Value::Int((*subtotal as f64 * 0.08) as i64)),
        _ => Ok(Value::Nil),
    }
});
let tax_sum = engine.eval(r#"
    [100, 250, 400]
    |> map(\price -> calculate_tax(price))
    |> sum()
"#).unwrap();
assert_eq!(tax_sum, Value::Int(60));
```

Use `enable_output_capture()` and `captured_output()` to collect exact script
output. Functions and closures survive subsequent `eval` calls; an evaluation
error clears execution frames so the engine can be reused. Effects completed
before an error (output and global definitions) are retained. `reset()` releases
script globals and compiled code while preserving registered Rust callbacks and
output-capture mode. It restores registered callbacks even if a script shadowed
their names. `take_output()` drains captured output while keeping capture enabled.

For real application integration, use `set_global(name, Value)` to supply data,
`get_global(name)` to retrieve it, and `call(name, &[Value])` to invoke an already
defined script function without reparsing source. `register_closure` accepts
callbacks that own host state; `Rc<RefCell<_>>` can hold mutable application data.
The runnable [embedded rules example](examples/embedded_rules.rs) records approval
decisions through a captured callback:

```sh
cargo run --offline --example embedded_rules
```

`compile(source)` or `compile_file(path)` returns a `CompiledScript`; repeated
`run(&script)` calls reuse its bytecode and accept updated globals without
reparsing or growing the engine's code storage. Handles belong to their engine
session: another engine or `reset()` invalidates them. `eval_file(path)` executes
a complete module graph directly. Reloading a file creates a new graph evaluation;
use a compiled handle when you want deliberate reuse.

A session retains compiled chunks to keep existing functions valid; call `reset`
when ending a session. Values use `Rc` and are not `Send`/`Sync`; use a separate
engine per thread. Script function depth is limited to 256 to report recursion
errors before exhausting the native stack. There is no CPU/memory sandbox or
execution timeout. `register_fn` accepts function pointers and `register_closure`
accepts owned `'static` closures. Registered host callbacks apply to the VM and
are not automatically linked into standalone native programs. Return
`RyndResult::Err` from callbacks to report errors; host panics are not caught.

## Verification and benchmarks

```sh
cargo test --offline
cargo test --offline --release
cargo clippy --offline --all-targets -- -D warnings
python3 scripts/audit_claims.py
sh scripts/check.sh
sh scripts/coverage.sh
cargo bench --offline --bench rynd_benchmarks
# Equivalent shared harness through the CLI:
cargo run --offline --release -- bench
```

Tests cover the original BDD examples, optimized native execution, exact stdout
and stderr parity, exit status, captures, destructuring, numeric boundaries,
parser error paths, REPL persistence, embedding, and benchmark failures. Expected
values are asserted independently of backend agreement. The audit script also
compiles the Rust embedding example from this README and runs every checked-in
Rynd example entry point on both targets, including the multi-module order program.

Current verification passes **122 tests in each of debug and release**, including
70 optimized native E2E tests, and measures **90.16% source line coverage**.
Additional integration tests compile module graphs and generated Cargo projects,
consume a generated library from an independent Rust crate, and verify Rust crate
adapters and rebuilds. The installed CLI has also created, run, and tested a Cargo
application from outside the repository.

Coverage requires `cargo-llvm-cov` and the toolchain's `llvm-tools-preview`
component. `scripts/coverage.sh` measures source coverage (excluding tests and
generated copies under target) and
requires at least 90% line coverage. Coverage is a measurement, not proof that
all possible programs are correct.

The dependency-free benchmark harness warms up, collects 20 batch samples,
reports means, medians, and Student-t 95% intervals, and validates computed
results. It measures parsing, complete VM evaluation, precompiled VM execution,
native computation in-process, native execution including process startup, and a
handwritten typed Rust iterator baseline. Compilation is timed separately.
Samples are host-dependent; intervals do not account for all systematic noise.

Benchmark compilation, launch, and result-validation failures return nonzero.
Temporary native binaries are built under `target/bench_run` and cleaned up;
`RYND_BENCH_DIR` can select another writable, executable location. See
[BENCHMARKS.md](BENCHMARKS.md) for the measured results and methodology.

[CLAIM_AUDIT.md](CLAIM_AUDIT.md) preserves the original transcript findings and
records their remediation. Historical claims about strict TDD or consultancy
affiliation are not claims made by this project.

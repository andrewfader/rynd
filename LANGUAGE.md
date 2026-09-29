# Rynd language and Rust overlay contract

Rynd provides dynamic, expression-oriented application logic compiled to Rust.
Its native backend emits direct Rust expressions and closures with checked Rynd
value operations. Meso is the bytecode VM used by interactive and embedded runs.
Cargo projects use Rust's normal
crate system, linker, release profiles, testing tools and native dependencies.

## Ruby, Elixir and CoffeeScript influences

The language combines these expression forms:

- Ruby/Elixir-style `"Hello #{name}"` interpolation accepts full expressions,
  including nested strings and maps. Expressions run once, left to right, and
  use normal value display. Escape a literal marker as `\#{name}`.
- `condition ? yes : no` is a right-associative expression. It evaluates the
  condition once and only the chosen branch. `unless condition { body } else
  { fallback }` is the inverse of `if`; omitting `else` yields nil when skipped.
- Elixir-style `value |> function(args)` inserts the value as the first argument.
  Functions, closures, pattern matching and immutable data compose normally.
- List and map comprehensions build transformed collections with an optional
  filter: `[x * 2 for x in xs if x > 0]` and `{x: x * x for x in xs}`.
- Safe navigation `value?.field` returns nil for missing fields/non-record values.
  Groovy-style `value ?: fallback` uses truthiness and short-circuits, returning
  the original value when truthy.
- Semicolons are optional; blocks and conditionals return values. Parenthesized
  calls and braced blocks are explicit, so indentation does not change parsing.

Single-quoted strings are literal: `'#{name}'` keeps its text. Only `\\` and
`\'` escape characters in single quotes. Double quotes support interpolation
and newline/tab escapes. `and`, `or`, and `not` are aliases for `&&`, `||`, and
`!`, with the same precedence and short-circuit behavior.

Expression statements and returns accept same-line modifiers:
`println('ready') if ready`, `work() unless skipped`, and `return 0 if empty`.
A skipped expression produces nil. Function and lambda parameters accept
structural patterns, including tuples, lists, variants, and `_`:
`fn add((a, b)) { a + b }`, `\(a, b) -> a + b`. A mismatched argument pattern
raises an error. Named bindings across parameters must be unique. Lambdas support
both backslash syntax (`\x, y -> x + y`), pipe syntax (`|x, y| x + y`), and
zero-argument forms (`\ -> 42`).

Trailing commas are permitted in function definitions, call arguments, tuples,
lists, and pattern destructuring: `(a, b,)`, `[1, 2,]`, `fn f(x, y,) { x + y }`.
Strings support repetition via `*` (`"abc" * 3` produces `"abcabcabc"`; non-positive
counts yield `""`) and mixed concatenation via `+` (`"count: " + 42` produces
`"count: 42"`, `42 + " items"` produces `"42 items"`). Safe navigation operates
safely on scalar values without errors (`42?.missing` produces nil). Tagged variants
support field introspection: `.tag` (or `.name`) returns the variant's name, and
`.value` accesses its payload (yielding nil for payloadless variants like `None`).

Truthiness is intentionally the existing Rynd/Groovy-like rule: nil, false, zero,
NaN, empty strings/lists/maps, `None`, `Err`, and `Reject` are falsey; `Some(x)` follows x.
This differs from Ruby and Elixir, where zero and empty collections are truthy.
Use explicit comparisons when portability matters.

Operator precedence, weakest first:

| Level | Operators |
| --- | --- |
| Conditional | `condition ? yes : no` (right associative) |
| Fallback | `?:` |
| Logical | `||`, then `&&` |
| Equality | `==`, `!=` |
| Comparison | `<`, `<=`, `>`, `>=` |
| Pipeline | `|>` |
| Range | `..`, `..=` |
| Arithmetic | `+`, `-`, then `*`, `/`, `%` |
| Unary | `!`, unary `-` |
| Access/call | `()`, `[]`, `.`, `?.` |

For example, `xs |> sum() > 10 ? "large" : "small"` compares the pipeline result.
Branches may contain pipelines. Operators other than ternary are left associative;
parentheses make unusual combinations explicit. A call/index opening on a new
line starts a new expression. Continue a pipeline with `|>` on the next line.

## Values, functions and errors

Values include signed checked i64 integers, f64 floats, Unicode strings,
immutable lists/tuples/maps, tagged variants, and first-class functions/closures.
Map keys are strings; other key expressions are formatted as strings. Map order
is deterministic. Indexing accepts negative indices and returns nil out of bounds;
strings index Unicode scalar values, not bytes or grapheme clusters.

Functions are `fn name(args) { body }`; lambdas are `\x, y -> expression`.
Lexical captures retain declaration-time values used by the function or its
nested closures. Captures omit unrelated locals. Functions resolve
module globals at call time and can call functions declared later in the module.
`let` creates an immutable binding; a new `let` may shadow it. `let mut` is rejected.
Explicit `return` is valid inside functions. Ending the final expression with a
semicolon makes a block return nil.

Patterns support literals, `_`, variables, nested tuples/lists, and tagged
variants such as `Some`, `None`, `Ok`, `Err`, `Accept`, and `Reject`. Match guards run after matching and binding. Arm
bindings remain local. Unmatched `match` yields nil; a failed `let` pattern is an
error. Integer overflow and zero division/remainder are errors in every build
profile. Floats follow IEEE arithmetic; mixed comparisons preserve integer
precision. `parse_float` accepts only finite values.

Parse, compile and runtime errors include positions; file entry points preserve
originating module paths. Runtime errors are `RyndResult::Err` at the Rust API and
nonzero exits in executable workflows. This is separate from tagged `Err(value)`,
which scripts can match as ordinary data. Host callback panics are not caught.

## Modules and public interfaces

`import "relative/path.rynd" as alias` loads a module relative to its importer.
Canonical paths identify shared dependencies, so two aliases initialize the same
module once per graph. Import cycles and excessive import depth are rejected.
Top-level initialization runs in dependency order; no module code runs during
`check` or Rust code generation. File I/O required to read source still occurs.

Only `pub fn` and `pub let` declarations appear in a module's exported record.
Use `alias.function(value)` or `alias.constant`. Other bindings are private and
qualified internally, including references from exported functions. Imports and
`pub` are allowed only at module scope. Anonymous `eval` has no import base path;
use `eval_file`, `compile_file`, or a file/Cargo CLI command.

A Cargo project's root exports form its Rust library interface. Generated
`Application::call` invokes exported functions; `get` returns exported values.
`Application::new` registers Rust functions and initializes the Rynd graph.
For a binary project its Rust entry point calls exported `main()`. Top-level
file-mode scripts keep their ordinary expression execution behavior.

## Rust interoperability and deployment

Rynd code and Rust code share `rynd::Value` and `RyndResult` within a Cargo project.
Register ordinary Rust functions and closures (0–8 arguments) with
`engine.register(name, f)`; arguments convert through `FromValue` and results
through `IntoValue`, so type mismatches become script errors, not panics.
Supported types: `Value`, `bool`, `String`, `i64` and every other integer type
(range-checked), `f64` (ints widen), `Vec<T>`, `Option<T>` (`Some`/`None`,
`nil` reads as `None`), `BTreeMap`/`HashMap<String, T>`, tuples up to 4, `()`,
and `RyndResult<T>` returns, whose `Err` becomes a recoverable script error.
`register_closure(name, arity, f)` and `register_fn` remain for raw `&[Value]`
adapters. Rust consumers call compiled Rynd exports directly through the
generated library API.

```rust
engine.register("clamp", |x: i64, lo: i64, hi: i64| x.clamp(lo, hi));
rynd::register!(engine, parse_sku, price_of); // registered under their Rust names

rynd::record! {
    #[derive(Clone)]
    pub struct Order { pub id: i64, pub total: f64, pub note: Option<String> }
}
engine.register("discount", |mut o: Order| { o.total *= 0.9; o }); // maps in, maps out
engine.install(&order)?; // fields become globals: `id`, `total`, `note`
```

`record!` is a dependency-free derive: it defines the struct and its
map conversions, with field-qualified errors (`Order.total: Expected float`).
Nested records, lists of records, and `Option` fields work.

Embedding building blocks:

- `Library` bundles functions and constants; `RyndEngine::with_library(&lib)`
  or `ScriptRuntime::with_library(lib)` installs it (again after `reset`).
- `Package` holds a script's source and compiled entry; `reload(engine, src)`
  swaps definitions atomically: a compile or top-level runtime error keeps
  the previous version.
- `SourceTree` is an in-memory file tree whose `import`s resolve like files on
  disk: `engine.eval_tree(&tree, "main.rynd")`.
- Filter-map scripts return `Accept(value)` or `Reject(value)`;
  `engine.filtermap(name, args)` yields `(Verdict, payload)`. `Reject` is falsey
  and `filter_map` keeps accepted payloads.
- `RyndEngine::sandboxed()` drops file, process, environment, and network
  builtins; see [SECURITY.md](SECURITY.md).
- `rynd::cli::main_with(|engine| ...)` ships the whole `rynd` CLI (run, eval,
  lines, repl, debug) from your binary with your functions preinstalled.
- `rynd::tools::print_ast` / `print_bytecode` and `has_main_function` inspect
  sources without running them.
- With the opt-in `jit` cargo feature, `engine.enable_jit()` (or
  `rynd run --jit`) compiles integer/boolean functions that pass a static type
  check to machine code with Cranelift. Calls with other argument types, and
  every other function, run on the VM; errors, spans, and the call-depth limit
  are identical. `jit_functions()` lists what was compiled; see `src/jit.rs`.

Checked `TryFrom<&Value>` conversions also extract `i64`, `f64`, `bool`,
`String`, and borrowed `&str`; `Value::from` builds values from Rust data.
Floats preserve their bits, including non-finite values.

Adapters provide an explicit dynamic ABI. Implement strongly typed, concurrent,
asynchronous or performance-critical components in Rust adapters. A Cargo
workspace shares one runtime dependency for a common `Value` type. Scaffolds
pin a portable compiler and runtime source snapshot.

The standalone compiler needs `rustc` at build time; Cargo projects need Cargo
and a compatible Rust toolchain. Built executables need neither installed.
Scaffolds use no third-party dependencies until you add some. Standard Cargo
lockfiles/profiles govern dependencies and release builds.

## Runtime model

Collections and input helpers materialize in memory. Values use `Rc`; create an
independent runtime per thread or convert values to Rust data for transfer.
Script call depth is bounded at 256. Scripts and native adapters execute with
normal process authority; `RyndEngine::sandboxed()` removes host-access builtins
for less-trusted scripts (see [SECURITY.md](SECURITY.md)).

See [README.md](README.md) for executable workflows and verification, and
[BENCHMARKS.md](BENCHMARKS.md) for measured performance.

## Core builtins

`map(xs, f)`, `filter(xs, predicate)`, and `reduce(xs, initial, f)` produce
transformed lists or a folded value; reduce callbacks receive accumulator and item.
`sum(xs)` adds numbers with checked integer arithmetic. `range(start, end)` builds
integers up to the exclusive end. `head(xs)` returns the first item or nil;
`tail(xs)`, `push(xs, value)`, and `concat(xs, ys)` create new lists.
`to_map(pairs)` accepts two-element tuples/lists, with last-entry-wins keys.
`len(value)` counts collection entries or Unicode scalar values in strings.
`abs(x)`, `min(x, y)`, and `max(x, y)` operate on numbers.

`lines(text)` splits LF/CRLF lines and omits a terminal empty line.
`split(text, separator)` splits literally and retains empty fields;
`trim(text)` removes surrounding Unicode whitespace; `join(strings, separator)`
joins strings. `parse_int(text)` parses a trimmed decimal i64 and
`parse_float(text)` a trimmed finite f64. `contains(value, item)` checks substring,
list/tuple membership, or map-key presence. Invalid conversions return errors.

`read_text(path)` and `read_stdin()` read all UTF-8 text from a file or stdin.
`to_string(value)` uses Rynd display formatting; `print(value)` and
`println(value)` write it. `args` contains script argument strings. `Some(x)`,
`None`, `Ok(x)`, `Err(x)`, `Accept(x)`, and `Reject(x)` construct tagged values
for pattern matching.

## JSON and collection pipelines

`parse_json(text)` reads JSON data into nil, booleans, signed i64 integers,
finite f64 numbers, strings, lists, and string-keyed maps. `to_json(value)` emits
compact JSON with sorted object keys and escaped strings. Both work in the VM,
standalone executables, and Cargo projects.

Integer tokens preserve all 64 bits; values outside i64 produce an error. Decimal
and exponent tokens use f64 rounding. Duplicate object keys keep the last value.
Unicode escapes support surrogate pairs; lone surrogates produce errors. Parsing
and encoding allow up to 128 nested containers. JSON conversion reports errors for
unsupported values (functions, tuples, variants, NaN, infinity), malformed syntax,
and trailing input. Parse errors include a one-based byte position. Strings are
data, including any text that resembles Rynd interpolation.

```rynd
let data = "[{\"name\":\"Ada\",\"score\":42}]" |> parse_json()
data |> sort_by(\row -> -row.score) |> to_json()
```

Collection helpers take the collection first, so each composes with `|>`.
All return new values and preserve their inputs.

| Function | Behavior |
| --- | --- |
| `sort(xs)` | Stable ascending numeric or string order. Mixed integers/floats compare precisely; NaN and incompatible types produce errors. |
| `sort_by(xs, key)` | Stable ascending order by a unary key function; computes each key once, in input order. Negate numeric keys for descending order. |
| `group_by(xs, key)` | Map of string keys to lists; preserves input order within each group. Requires string keys. |
| `keys(map)`, `values(map)`, `entries(map)` | Sorted key order; entries are `(key, value)` tuples usable with `to_map`. |
| `take(xs, n)`, `skip(xs, n)` | First n / remaining items. Counts are nonnegative integers, capped at the input length. |
| `enumerate(xs)` | `(index, value)` tuples, starting at zero. |
| `zip(xs, ys)` | Pairs through the shorter input's length. |
| `any(xs, predicate)`, `all(xs, predicate)` | Short-circuit using truthiness. Empty input returns false / true respectively. |
| `find(xs, predicate)` | First matching item as `Some(value)`, or `None`; short-circuits. |
| `filter_map(xs, f)` | One pass; keeps the payload of `Some(value)`/`Accept(value)`, skips `None`/`Reject(_)`. Preserves falsey payloads such as zero and nil. |
| `flat_map(xs, f)` | Concatenates the lists returned by f, in input order. |

These operations use eager lists. `find`, `any`, and `all` avoid scanning the tail
once their answer is known. `filter_map` combines selection and transformation
into a single traversal and output list; tagged-value allocation and callbacks
still have a cost. `sort_by` caches keys before its O(n log n) stable sort. Each
unary callback is arity-checked even for an empty input.

## Map operations and dynamic dispatch

Rynd provides functional map manipulation builtins and dynamic dispatch helpers:

| Function | Behavior |
| --- | --- |
| `merge(m1, m2)` | Returns a new map containing all key-value pairs from `m1` and `m2`. Keys present in both maps take their values from `m2`. |
| `put(map, key, value)` | Returns a new immutable map with `key` set to `value`. `key` must be a string. |
| `get(map, key, default)` | Retrieves the value associated with `key`. If `key` is absent or its value is `nil`, returns `default`. |
| `has_key(map, key)` | Returns `true` if `map` contains the specified string `key`, `false` otherwise. |
| `delete(map, key)` | Returns a new immutable map with `key` removed. |
| `apply(fn_or_name, args)` | Dynamically invokes a function (by global string name or callable value) with arguments passed as a list. |
| `call_method(map, method_name, args)` | Object-style dynamic dispatch. Retrieves the function at `map[method_name]` and invokes it with `map` as `self` followed by `args`. |

```rynd
let base_config = {"host": "localhost", "port": 8080}
let prod_config = base_config |> put("port", 443) |> put("ssl", true)

let account = {
    "balance": 100,
    "deposit": \self, amount -> put(self, "balance", self.balance + amount),
    "summary": \self -> "Balance: #{self.balance}"
}
let updated = call_method(account, "deposit", [50])
println(call_method(updated, "summary", [])) # Balance: 150
```

## Structured concurrency, fibers, actors, and channels

Rynd provides green fibers, nurseries with automatic failure cascading, actor mailboxes, and CSP channels:

### Green fibers and structured nurseries

- `spawn(\ -> expression)` launches a cooperative fiber and returns a `Fiber(id)` handle.
- `await_fiber(handle)` pauses the current execution until the fiber or actor finishes, returning its result or propagating any error.
- `yield_fiber()` cooperatively pauses the fiber to allow scheduler progress.
- `self_id()` returns the integer identifier of the current fiber.
- `nursery(\n -> body)` creates a structured concurrency scope. Inside `body`, child tasks are scheduled with `n.spawn(\ -> task_body)`. If any child task fails, the nursery immediately cancels sibling tasks and propagates the error to the caller. When all tasks succeed, `nursery` returns a list containing every child task's result in spawn order.

```rynd
# Structured nursery: parallel execution with fail-fast cancellation
let results = nursery(\n -> {
    n.spawn(\ -> fetch_metrics("server-1"))
    n.spawn(\ -> fetch_metrics("server-2"))
    n.spawn(\ -> fetch_metrics("server-3"))
})
```

### Actor model and mailboxes

- `spawn_actor(\mailbox -> loop)` spawns an actor fiber with an isolated mailbox and returns an `Actor(id)` handle.
- `send(actor, message)` enqueues a message into the actor's mailbox asynchronously.
- `receive(mailbox)` blocks until a message arrives in the mailbox and dequeues it.
- `receive_timeout(mailbox, timeout_ms)` awaits a message up to `timeout_ms` milliseconds; returns `Some(message)` on receipt or `None` on expiration.
- `create_mailbox()` constructs an independent mailbox handle.

```rynd
let worker = spawn_actor(\mb -> {
    let task = receive(mb)
    process(task)
})
send(worker, {"job": "resize_image", "file": "avatar.png"})
let result = await_fiber(worker)
```

### CSP channels

- `channel()` creates an unbounded FIFO channel and returns a 2-tuple `(tx, rx)`.
- `channel_send(tx, value)` writes a value into the channel.
- `channel_recv(rx)` blocks until a value is available and removes it.
- `channel_try_recv(rx)` non-blockingly checks the channel, returning `Some(value)` if available or `None` if empty.

```rynd
let (tx, rx) = channel()
let producer = spawn(\ -> {
    [10, 20, 30] |> each(\x -> channel_send(tx, x * 2))
})
await_fiber(producer)
let first = channel_recv(rx) # 20
```

## TCP networking, HTTP streaming, and web frameworks

Rynd supports native TCP sockets, HTTP/1.1 parsing and streaming, and a Plug-based modular web framework inspired by Sinatra and Phoenix:

### Native TCP sockets

| Function | Behavior |
| --- | --- |
| `tcp_listen(addr)` | Binds a TCP listener to `addr` (e.g. `"127.0.0.1:8080"`). Returns a `TcpListener(id)` handle. |
| `tcp_accept(listener)` | Accepts an incoming connection. Returns `(stream, peer_addr)` pair or `nil` if non-blocking and no connection is pending. |
| `tcp_connect(addr)` | Connects to a remote TCP endpoint. Returns a `TcpStream(id)` handle. |
| `tcp_local_addr(listener)` | Returns the local bound address string (useful with `"127.0.0.1:0"` port allocation). |
| `socket_read(stream, max_bytes)` | Reads up to `max_bytes` from the stream as a UTF-8 string. |
| `socket_read_bytes(stream, max_bytes)` | Reads up to `max_bytes` from the stream as an integer list of bytes (`[0–255]`). |
| `socket_write(stream, data)` | Writes a UTF-8 string or a byte list to the stream. Returns the number of bytes written. |
| `socket_close(handle)` | Closes a TCP stream or listener. |
| `socket_set_nonblocking(handle, bool)` | Toggles non-blocking mode on the listener or stream. |

### HTTP parsing and chunked transfer

- `parse_http_request(raw_text)` parses an HTTP/1.1 wire request into a record:
  `{"method": "GET", "path": "/path", "version": "HTTP/1.1", "headers": {"host": "..."}, "body": "..."}`.
- `format_http_response(status, headers, body)` serializes a standard HTTP/1.1 response string with appropriate status phrase and calculated `Content-Length`.
- `socket_write_http_chunk(stream, data)` streams a chunk formatted as `HEX_LEN\r\nDATA\r\n`.
- `socket_finish_http_chunks(stream)` emits the terminal HTTP chunk `0\r\n\r\n`.
- `socket_read_chunk(stream, max_bytes)` reads a chunked stream payload.

### Plug web framework and routing

Rynd includes a composable pipeline architecture for web services:

| Function | Behavior |
| --- | --- |
| `conn(method, path)` | Initializes a connection map: `{"method": method, "path": path, "params": {}, "query": {}, "headers": {}, "body": "", "status": 200, "resp_headers": {"content-type": "text/html; charset=utf-8"}, "resp_body": "", "halted": false}`. |
| `match_route(path, pattern)` | Compares `path` against a route pattern with `:param` capture segments and `*wildcard` rest parameters. Returns `Some(params_map)` or `None`. |
| `halt(conn)` | Sets `conn.halted = true` to short-circuit downstream plugs in a pipeline. |
| `put_status(conn, status)` | Updates the response status code on `conn`. |
| `put_resp_header(conn, key, val)` | Adds or overrides a response header on `conn`. |
| `put_resp_body(conn, body)` | Replaces the response body on `conn`. |
| `text_response(conn, status, body)` | Configures status, sets `content-type: text/plain`, updates body, and halts `conn`. |
| `json_response(conn, status, data)` | Serializes `data` to JSON via `to_json()`, sets `content-type: application/json`, and halts `conn`. |
| `html_response(conn, status, html)` | Configures status, sets `content-type: text/html`, updates body, and halts `conn`. |
| `plug_send(conn, stream)` | Formats the final HTTP response and writes it to the active TCP stream. |

```rynd
fn router(conn) {
    if conn.halted { return conn }

    let m = match_route(conn.path, "/api/users/:id")
    if m.tag == "Some" and conn.method == "GET" {
        return json_response(conn, 200, {"id": m.value.id, "active": true})
    }

    text_response(conn, 404, "Not Found")
}

let response = conn("GET", "/api/users/42")
    |> (\c -> put_resp_header(c, "x-request-id", "req-1"))()
    |> router()
println(response.resp_body) # {"active":true,"id":"42"}
```

## Recoverable errors

`attempt(function, argument_list)` returns `Ok(value)` or `Err(message)` for a
runtime failure. Use pattern matching to recover at the point that owns the
policy. An ordinary returned `Err(value)` is data and becomes `Ok(Err(value))`.

```rynd
fn parse_or_zero(text) {
    match attempt(parse_int, [text]) {
        Ok(value) => value,
        Err(message) => 0
    }
}
["10", "bad", "20"] |> map(parse_or_zero) |> sum() # 30
```

Failed calls release their execution frames and temporary operands. Completed
output and host effects remain visible. Arguments are evaluated before entering
`attempt`; wrap an expression in a zero-argument lambda to catch its evaluation:
`attempt(\ -> 1 / 0, [])`. Host panics propagate outside this recovery mechanism.

See [SCRIPTING.md](SCRIPTING.md) for batch/text/system helpers and streaming CLI
workflows, and [DEBUGGING.md](DEBUGGING.md) for interactive development.

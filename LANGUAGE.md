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
  builtins and applies `Limits::sandbox()` to every run;
  `engine.set_limits(Limits { .. })` limits any engine, and
  `engine.interrupt_handle()` stops a run from another thread. See
  [SECURITY.md](SECURITY.md).
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
Fibers stay on the thread that started them, so script values never cross
threads. Script call depth is bounded at 10,000 nested calls; tail calls do not
count. Source nesting is bounded at 200 levels, and deeply nested values drop,
print, and compare without recursing on the native stack. Scripts and native
adapters execute with normal process authority; `RyndEngine::sandboxed()`
removes host-access builtins and limits each run for less-trusted scripts (see
[SECURITY.md](SECURITY.md)).

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

## Fibers, nurseries, actors, and channels

A fiber is a function running on its own stack: a coroutine on the thread
that runs the program. One fiber runs at a time, switching at `yield_fiber`,
`await_fiber`, `receive`, `channel_recv`, nursery joins, `sleep_ms`, and I/O.
Sockets and HTTP wait in one `poll` loop, so ten thousand fibers waiting on
ten thousand sockets cost ten thousand small stacks, not threads. Name
resolution, `run_process`, and `read_stdin` run on a helper thread while their
fiber waits. Between switch points a fiber runs uninterrupted, so a
read-check-write sequence without I/O is atomic. CPU-bound loops do not
preempt; call `yield_fiber()` to share. Fiber stacks reserve address space
lazily, so recursion inside a fiber reaches the same depth limit as anywhere
else. Fibers need x86-64 or AArch64 on Linux or macOS; elsewhere `spawn`
reports an error.

| Function | Behavior |
| --- | --- |
| `spawn(\ -> body)` | Starts a fiber and returns `Fiber(id)`. The caller keeps running until its next switch point. |
| `await_fiber(handle)` | Waits for a fiber or actor and returns its result, or raises its error. A handle can be awaited once. |
| `yield_fiber()` | Lets every other ready fiber run first. |
| `self_id()` | `Fiber(id)` of the caller; the program's first thread is `Fiber(0)`. |
| `nursery(\n -> body)` | Structured scope: `n.spawn(task)` starts children, which all finish before `nursery` returns their results in spawn order. The first failure cancels the remaining children and is raised once they stop. |
| `spawn_actor(\mailbox -> body)` | Starts a fiber with its own mailbox; returns `Actor(id)` for `send` and `await_fiber`. |
| `send(actor_or_mailbox, message)`, `create_mailbox()`, `close_mailbox(mb)` | Unbounded FIFO mailboxes. |
| `receive(mailbox)` | Waits for the next message. |
| `receive_timeout(mailbox, ms)` | `Some(message)`, or `None` after `ms` milliseconds. |
| `channel()` | `(Sender, Receiver)` pair of an unbounded FIFO channel. |
| `channel_send(tx, value)`, `channel_recv(rx)`, `channel_try_recv(rx)` | Send; wait for a value; or take one if present (`Some`/`None`). |
| `close_channel(side)`, `is_channel_closed(side)` | Closing either side drops pending values and wakes blocked receivers with an error. |

Cancellation takes effect at the child's next switch point (sleeps and socket
waits end early); a cancelled child reports `Fiber cancelled`. A wait that nothing could ever
satisfy, such as `receive` on an empty mailbox with no other runnable fiber,
raises a `Deadlock` error instead of hanging. Engines on different OS threads
schedule independently.

```rynd
fn pong(mb, n) {
    if n == 0 { "done" } else {
        let (from, x) = receive(mb)
        send(from, x + 1)
        pong(mb, n - 1)       # a tail call: actors loop in constant space
    }
}
let ponger = spawn_actor(\mb -> pong(mb, 1))
let me = create_mailbox()
send(ponger, (me, 41))
receive(me)                   # 42

nursery(\n -> {
    n.spawn(\ -> fetch("http://127.0.0.1:3000/api/tags").status)
    n.spawn(\ -> { sleep_ms(50); "slept alongside the request" })
})
```

## Tail calls and recursion depth

Recursion is how Rynd loops. A call whose result the function returns directly
(the last expression, a branch of a final `if`/`match`, or a `return`) reuses
the caller's frame: self-recursive tail calls run in constant space on every
backend, and the VM extends this to all tail calls. Other calls nest up to
`MAX_CALL_DEPTH` (10,000) frames. CLI runs, generated binaries, Cargo project
entry points, and fibers get 256–512 MiB stack reservations (committed only
as used), so the limit, not the stack, decides; on a host thread of unknown
size Rynd stops at a conservative stack budget with the same depth error.
Debug sessions keep every frame so backtraces and step-over stay exact.

## HTTP services and clients

`serve(target, handler)` runs an HTTP/1.1 server. `target` is an address
string, a `TcpListener`, or `{"listener"|"addr": ..., "max_connections": n}`.
Each connection gets a fiber that reads requests (fixed-length or chunked
bodies, keep-alive and pipelining), calls `handler(conn)`, and writes the conn
it returns. A handler error becomes a logged `500`; malformed requests get
`400`, oversized heads `431`, and bodies over 16 MiB `413`. Without
`max_connections` the server runs until the program exits; with it, `serve`
returns the number of requests served.

The handler's `conn` is a map:

| Key | Contents |
| --- | --- |
| `method`, `path`, `version` | Request line; `path` excludes the query string. |
| `query_string`, `query` | Raw query and its decoded map (`+` is a space; repeated keys keep the last). |
| `headers`, `body` | Lowercase header names; body text. |
| `params` | Empty; routers fill it from `match_route`. |
| `remote_addr`, `socket` | Peer address and the `TcpStream`, for streaming. |
| `status`, `resp_headers`, `resp_body`, `halted` | The response under construction. |

Return the conn with `"sent": true` after writing the response yourself, for
example with `socket_write_http_chunk`. `read_http_request(stream)` reads one
request into the same shape for hand-rolled accept loops.

| Function | Behavior |
| --- | --- |
| `fetch(url)` | `GET`; returns `{"status", "reason", "headers", "body"}`. |
| `http_request(opts)` | `{"method", "url", "headers", "body", "timeout_ms"}`. A non-string `body` is sent as JSON. Reads fixed-length, chunked, and close-delimited responses. `http://` only; TLS belongs to a Rust adapter. |
| `url_encode(text)`, `url_decode(text)` | RFC 3986 percent-encoding (unreserved characters pass through). |
| `parse_query(text)`, `query_string(map)` | Form-style query strings to and from maps. |

### Plugs and routing

| Function | Behavior |
| --- | --- |
| `conn(method, target)` | A conn for tests and pipelines; `target` may carry a query string. |
| `match_route(path, pattern)` | `Some(params)` for `:name` segments and a trailing `*rest`, percent-decoded; else `None`. |
| `halt(conn)`, `put_status(conn, n)`, `put_resp_header(conn, k, v)`, `put_resp_body(conn, text)` | Return updated conns. |
| `text_response`, `json_response`, `html_response` | `(conn, status, body)`: set status, content type (UTF-8 text, or `application/json`), and body, and halt. `json_response` encodes data with `to_json`. |
| `plug_send(conn, stream)` | Write the conn's response to a stream and close it. |
| `parse_http_request(text)`, `format_http_response(status, headers, body)` | Wire-format conversions with standard reason phrases and `Content-Length`. |

```rynd
fn app(conn) {
    let m = match_route(conn.path, "/api/users/:id")
    if m.tag == "Some" and conn.method == "GET" {
        return json_response(conn, 200, {"id": m.value.id, "q": conn.query.q})
    }
    text_response(conn, 404, "Not Found")
}
serve("127.0.0.1:8080", app)
```

[examples/conduit](examples/conduit) implements the RealWorld API on these
pieces: routing, JWT authentication, password hashing, and the document store.

### TCP sockets

| Function | Behavior |
| --- | --- |
| `tcp_listen(addr)` | Binds a `TcpListener` (`"127.0.0.1:0"` picks a port). |
| `tcp_accept(listener)` | `(stream, peer_addr)`, or nil when non-blocking with none pending. |
| `tcp_connect(addr)` | Connects within 5 seconds; host names resolve. |
| `tcp_local_addr(listener)` | The bound address. |
| `socket_read(stream, n)`, `socket_read_bytes(stream, n)` | Up to n bytes as text or a byte list; nil when non-blocking with nothing to read. |
| `socket_write(stream, data)` | Writes a string or byte list; returns the byte count. |
| `socket_write_http_chunk(stream, data)`, `socket_finish_http_chunks(stream)` | Chunked transfer framing. |
| `socket_read_chunk(stream, max)` | Decodes the next chunk; nil after the final one. |
| `socket_close(handle)`, `socket_set_nonblocking(handle, flag)` | For streams and listeners. |

## Hashing, tokens, and passwords

SHA-256, HMAC, PBKDF2, base64, and HS256 JWTs are built in with no
dependencies and checked against their RFC test vectors. Randomness comes from
the operating system. Inputs are strings (UTF-8) or byte lists.

| Function | Behavior |
| --- | --- |
| `sha256(data)`, `hmac_sha256(key, data)`, `hex_encode(data)` | Lowercase hex digests. |
| `base64_encode(data)`, `base64url_encode(data)` | Padded standard / unpadded URL-safe alphabets. |
| `base64_decode(text)`, `base64url_decode(text)`, `base64_decode_bytes(text)` | Either alphabet, padding optional; text (UTF-8) or bytes. |
| `to_bytes(text)`, `from_bytes(bytes)` | UTF-8 conversions. |
| `random_bytes(n)`, `random_token(n)`, `random_int(lo, hi)`, `uuid()` | OS randomness: bytes, an n-byte URL-safe token, an unbiased integer in `lo..hi`, a v4 UUID. |
| `password_hash(password)`, `password_verify(password, stored)` | Salted PBKDF2-HMAC-SHA256 (100,000 rounds) in a self-describing `$pbkdf2-sha256$...` string; constant-time comparison. |
| `jwt_sign(claims, secret)`, `jwt_verify(token, secret)` | HS256 tokens; verify returns `Ok(claims)` or `Err(reason)`, checking `exp`/`nbf` (seconds). |
| `secure_compare(a, b)` | Constant-time equality. |

## Time and text

`now()` returns Unix milliseconds. `iso_time(ms)` formats RFC 3339 UTC with
milliseconds (`2026-10-17T09:41:00.327Z`); `parse_time(text)` reads RFC 3339
with `Z` or `±HH:MM` offsets, or a bare date, and rejects impossible dates.
`slugify(text)` lowercases ASCII words and joins them with hyphens.

## Document store

`store_open(path)` opens a store of named collections of documents keyed by
integer or string ids (`":memory:"` keeps it in memory). Writes append JSON
Lines to the file before returning; opening replays the log, skips a torn final
line, and compacts logs that outgrow their data. A store belongs to the program
(fiber group) that opened it.

| Function | Behavior |
| --- | --- |
| `store_put(db, collection, id, doc)` | Insert or replace; returns doc. |
| `store_get(db, collection, id)` | The document, or nil. |
| `store_delete(db, collection, id)` | Whether a document was removed. |
| `store_all(db, collection)`, `store_count(db, collection)` | Documents in id order (integers first, numerically), and their count. |
| `store_find(db, collection, fields)` | Documents whose fields equal every given value. |
| `store_next_id(db, collection)` | A persistent per-collection integer sequence. |
| `store_close(db)` | Flushes to disk and releases the handle. |

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

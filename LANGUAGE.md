# Rynd language and Rust overlay contract

Rynd provides dynamic, expression-oriented application logic compiled to Rust.
Its native backend emits direct Rust expressions and closures with checked Rynd
value operations. Meso is the bytecode VM used by interactive and embedded runs.
The native target does not interpret bytecode. Cargo projects use Rust's normal
crate system, linker, release profiles, testing tools and native dependencies.

## Ruby, Elixir and CoffeeScript influences

The influences are semantic choices, not a promise of source compatibility:

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
  Groovy-style `value ?: fallback` uses truthiness and short-circuits. It is not
  a nil-only coalescer and does not unwrap `Some` or `Ok`.
- Semicolons are optional; blocks and conditionals return values. Parenthesized
  calls and braced blocks are explicit, so indentation does not change parsing.

Truthiness is intentionally the existing Rynd/Groovy-like rule: nil, false, zero,
NaN, empty strings/lists/maps, `None`, and `Err` are falsey; `Some(x)` follows x.
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
Lexical captures retain declaration-time values. Top-level functions resolve
module globals at call time and can call functions declared later in the module.
`let` creates an immutable binding; a new `let` may shadow it. `let mut` is rejected.
Explicit `return` is valid inside functions. Ending the final expression with a
semicolon makes a block return nil.

Patterns support literals, `_`, variables, nested tuples/lists, and tagged
`Some`, `None`, `Ok`, `Err`. Match guards run after matching and binding. Arm
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
Rust callbacks are registered with explicit arity. `register_fn` accepts function
pointers; `register_closure` accepts owned closures and captured application state.
A Rust adapter validates/unpacks values, calls any ordinary Rust crate API, and
returns values or errors. Rust consumers call compiled Rynd exports directly
through the generated library API. Integration tests exercise both directions
and an additional Rust crate dependency.

This is an explicit dynamic ABI, not transparent access to every Rust type,
trait, macro, borrow or generic from Rynd syntax. Implement strongly typed,
concurrent, asynchronous or performance-critical components in Rust adapters.
A Cargo workspace must share one runtime dependency for a common `Value` type.
The scaffold pins a portable source snapshot because Rynd is not published to a
registry by this repository; publishing a crate or claiming registry availability
is not part of creating a local project.

The standalone compiler needs `rustc` at build time; Cargo projects need Cargo
and a compatible Rust toolchain. Built executables need neither installed.
Scaffolds use no third-party dependencies until you add some. Standard Cargo
lockfiles/profiles govern dependencies and release builds.

## Explicit limits

Collections and input helpers materialize in memory, and dynamic dispatch has
measurable cost; this is not a zero-overhead static Rust dialect. Values use `Rc`
and do not cross threads; create independent runtimes or convert to Rust data.
Script call depth is bounded at 256. The VM has no untrusted-code sandbox,
CPU timeout, or memory quota. Native adapters have normal Rust process authority.
There is no Elixir actor/OTP runtime, distributed scheduler, debugger, automatic
Rust binding generator, or built-in full CSV implementation. Those are not
implied by the syntax influences. Rust crates provide such application services
through the tested adapter boundary.

See README for executable workflows and verification;
BENCHMARKS records measured performance rather than a machine-independent claim.

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
`None`, `Ok(x)`, and `Err(x)` construct tagged values for pattern matching.

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
| `filter_map(xs, f)` | One pass; keeps the payload of `Some(value)`, skips `None`. Preserves falsey payloads such as zero and nil. |
| `flat_map(xs, f)` | Concatenates the lists returned by f, in input order. |

These operations use eager lists. `find`, `any`, and `all` avoid scanning the tail
once their answer is known. `filter_map` combines selection and transformation
into a single traversal and output list; tagged-value allocation and callbacks
still have a cost. `sort_by` caches keys before its O(n log n) stable sort. Each
unary callback is arity-checked even for an empty input.

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

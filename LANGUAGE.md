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
Rust binding generator, or built-in full CSV/JSON implementation. Those are not
implied by the syntax influences. Rust crates provide such application services
through the tested adapter boundary.

See README for the builtin reference, executable workflows and verification;
BENCHMARKS records measured performance rather than a machine-independent claim.

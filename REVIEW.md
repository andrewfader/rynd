# Implementation review

Reviewed 2026-09-26: syntax, module loading, bytecode compilation and execution,
shared runtime, Rust generation, Cargo scaffolds, CLI/REPL, embedding, examples,
conformance checks, and benchmark harness.

## Supported workflows

Rynd supports functional application logic that integrates with Rust and ships
through Cargo. Immutable values, closures, pipelines, comprehensions, pattern
matching, modules, source locations, reusable compiled scripts, and host callbacks
work in the VM and compiled applications. The data helpers include JSON,
sorting, grouping, collection search, and explicit runtime error recovery.

[REQUIREMENTS.md](REQUIREMENTS.md) maps workflows to tests.
[LANGUAGE.md](LANGUAGE.md) specifies syntax and execution semantics.

## Changes from this review

Both compilers now use shared lexical free-name analysis to select closure
captures. A closure retains the outer locals referenced by its body, including
references needed by nested closures. Parameters, local declarations, pattern
bindings, and comprehension variables shadow outer names at their lexical scope.
Initializers see the preceding bindings. Recursive functions retain their self
reference, and globals resolve through the runtime.

This releases unused host values when a factory function returns and avoids
cloning unrelated local values while creating native closures. Tests check host
reference lifetimes and execute the same scope-sensitive programs in the VM and
optimized native binaries. Cargo scaffolds include the shared analysis module.

Typed Rust conversions now cover scalar values, borrowed strings, and inbound
lists/maps. Adapters use `TryFrom` to report type errors through normal runtime
recovery; exact scalar matching preserves integer precision. The embedding
example exercises conversion from script values into Rust booleans.

The conformance runner preserves executable checks for numeric behavior,
destructuring, captures, short-circuit evaluation, Unicode indexing, error exits,
REPL recovery, embedding, examples, and benchmark compiler failures. It writes
results to `target/conformance/results.json`.

The scripting toolkit adds immutable batch and zip operations, text filters and
replacement, JSON Lines, byte/file helpers, and explicit process execution. A
streaming CLI reuses compiled expressions per record. Single-quoted strings,
conditional statement modifiers, word operators, and destructured parameters
support these workflows.

The source debugger supports conditional breakpoints, stepping, exception
inspection, lexical values, frame selection, multiline expressions, and timing.
A separate debug loop keeps hooks out of normal bytecode dispatch. REPL commands
add history/replay, loading, inspection, completion lists, and timing.

## Verification

```sh
sh scripts/check.sh
sh scripts/coverage.sh
cargo bench --offline --bench rynd_benchmarks
```

The check script runs formatting, Clippy with warnings denied, debug and release
tests, the embedding example, VM/native conformance checks, and independent
Python JSON comparisons. Coverage enforces a 90% source-line threshold and writes
its report to `target/coverage/coverage.json`.

Final validation: 150 tests pass in debug and release. Formatting and Clippy
with warnings denied pass. The conformance suite passes 37 VM/native programs,
three REPL sessions, embedding, and two benchmark-failure checks. Independent
JSON checks pass 124 inputs and 20 rejections on each target. Source-line
coverage is 92.35% (4,938/5,347), above the 90% gate.
See [BENCHMARKS.md](BENCHMARKS.md) for new workload timings and baseline comparisons.

## Development directions

Future development can extend editor integration, formatting with comment
preservation, lazy collection protocols, execution budgets, syntax nesting
limits, and richer typed Rust adapters. Release packaging and a project license
are distribution decisions.

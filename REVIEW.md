# Codebase review and adoption priorities

Reviewed 2026-09-26: lexer/parser/AST, module loading, bytecode compiler and VM,
shared values/runtime, Rust code generation, Cargo scaffolding/builds, CLI/REPL,
embedding APIs, test coverage, audit scripts, examples, and benchmark harness.

## What is already useful

Rynd has a coherent niche: small functional programs that integrate with Rust
and ship through Cargo. Immutable values, closures, pipelines, comprehensions,
pattern matching, modules, source locations, reusable compiled scripts, host
callbacks, and two execution targets are already implemented. Shared runtime
semantics and independent expected-output tests are particularly valuable.

The largest immediate gap was everyday data work: scripts could read text but
needed host adapters for JSON, sorting, grouping, and recoverable parsing errors.
That makes a first useful program harder to write than the language syntax suggests.

## Research and decisions

Sources were checked through web searches targeting Reddit and GitHub, plus
primary project documentation. Direct access to Google's search page failed;
Google-specific results are not claimed. Community comments are qualitative
signals, not a survey or proof that a feature will increase adoption.

| Signal | Decision |
| --- | --- |
| [Rust scripting comparison discussion](https://www.reddit.com/r/rust/comments/1u0wbrh/scripting_languages_for_rust_comparison/) asks about integration, performance, and iteration. | Extend the existing Rust/VM workflow and measure specific workloads. |
| [Ad Astra's author announcement](https://www.reddit.com/r/rust/comments/1f389rz/) emphasizes API exports and language tooling. | Keep interop central; prioritize editor tooling next. |
| [Koto's repository](https://github.com/koto-lang/koto) presents embedding, standalone scripting, examples, and a playground together. | Add a complete data-processing example usable on both targets. |
| [Rhai's JSON documentation](https://rhai.rs/book/language/json.html) distinguishes its script-based convenience parser from strict serialization. | Implement a dedicated data parser with [RFC 8259](https://www.rfc-editor.org/rfc/rfc8259) grammar, explicit numeric/Unicode limits, and independent interoperability checks. |
| [Rhai's operation-limit API](https://rhai.rs/book/safety/max-operations.html) gives embedders control over script work. | Prototype a VM instruction budget, then defer it after measurements showed regressions in existing workloads. |

## Implemented

- JSON parsing/encoding, including Unicode surrogate pairs, exact i64 integers,
  sorted output keys, depth limits, and useful errors. The dependency-free runtime
  remains portable in generated standalone Rust and pinned Cargo scaffolds.
- Fifteen collection operations: sorting, cached sort keys, grouping, map views,
  slicing, enumeration, zip, short-circuit predicates/search, filter-map, flat-map.
- `attempt(f, args)` for runtime recovery using the existing `Ok`/`Err` patterns.
- Benchmarks comparing first-match search and fused collection processing with
  existing eager pipelines; expected results are checked before reporting success.

## Review findings addressed

1. **Nested VM failures left frames/operands behind.** Previously the outer engine
   discarded them only when the entire evaluation ended. Catching a callback error
   inside a running script would therefore corrupt execution. Calls now restore
   the caller's frame/stack boundary before returning an error. Tests cover locals,
   pending operands, nested `map`, recursion failure, and subsequent host calls.
2. **File-mode `run` read the entry file twice.** The CLI read source and then the
   module loader read it again. File runs now use the loader directly; stdin still
   uses the source reader.
3. **Collection search required full materialization.** `find`/`any`/`all` stop
   immediately when the result is known; callback-count tests establish that
   property independently of timing.
4. **Failures had one policy: abort evaluation.** `attempt` makes malformed records
   recoverable at an explicit function-call boundary.

## Next priorities and remaining tradeoffs

| Priority | Work | Reason |
| --- | --- | --- |
| Next | Editor highlighting, formatter, then an LSP using reusable diagnostics | Improve the edit/check cycle; establish formatting and comment-preservation rules before implementing a formatter. |
| Next | Release packaging, CI across operating systems, explicit project license | Make installation and redistribution straightforward. License selection belongs to the project owner. |
| Next | Lexically precise closure capture analysis | Both backends currently capture all visible locals, retaining values a closure never uses. Requires shadowing/transitive-capture tests. |
| Later | Lazy iterators and streaming JSON Lines | Eager inputs, ranges, and collections currently materialize in memory. A lazy protocol needs clear ordering/error/lifetime semantics. |
| Next | Low-overhead execution budgets | The initial implementation slowed existing VM workloads; preserve default throughput before shipping limits. |
| Later | Parser/compiler nesting limits and broader fuzzing | Module, function-call, and JSON depth limits exist; arbitrary syntax/AST recursion still needs an explicit limit. |
| Later | Typed Rust adapters and richer value conversions | Reduce adapter boilerplate while preserving explicit errors and crate identity. |

Scripts and host callbacks run with process authority; this remains a
trusted-script runtime. Engine sessions retain compiled chunks for live function
values; use compiled handles for repeated work and reset at session boundaries.
`rynd check` validates syntax and compilation; dynamic names/types resolve at run
time. Adding a JIT, async runtime, or package registry would require substantially
more design and maintenance than the useful workflows delivered here.

Validation commands and measured performance are recorded in [BENCHMARKS.md](BENCHMARKS.md).
The executable contracts live in [LANGUAGE.md](LANGUAGE.md),
[tests/data_workflows.rs](tests/data_workflows.rs), and the CLI/native/project suites.

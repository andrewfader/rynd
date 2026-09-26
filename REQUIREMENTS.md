# Rynd delivery contract

This checklist maps the original request in the Agy transcript and the later
Rynd/overlay/semantics requests to implemented behavior. It describes the current
source tree, not the original `506d6eb` implementation. Historical transcript
claims and their corrections remain in CLAIM_AUDIT.md.

| Requested outcome | Delivered behavior | Evidence / entry points |
| --- | --- | --- |
| Overall name Rynd; Meso may name a component | `rynd` crate/CLI, `.rynd` sources, `RyndEngine`, `RyndError`, `RyndResult`; Meso identifies the VM/runtime | Cargo.toml, CLI version/help, all current examples/tests |
| A functional language layer over Rust | Expression-valued blocks, first-class functions, captures, immutable data, pipelines, structural patterns, guards, tagged values, comprehensions | Parser/VM tests, BDD scenarios, optimized native E2E tests |
| Applications and libraries, beyond scripting | `rynd new`, `new --lib`, Cargo build/run/test/check, generated library exports | tests/project_test.rs builds and moves an application, builds a library, and consumes it from an independent Rust crate |
| Reusable modules | Relative imports, explicit `pub` exports, isolated private names, canonical dependency deduplication, cycle diagnostics | tests/modules_test.rs; examples/orders |
| Rust ecosystem interoperability | Rust crate dependencies and native adapters callable from Rynd; Rust calls compiled Rynd exports using shared values/errors | Generated scaffold tests and project test using a separate Rust dependency |
| Genuine compilation to Rust | Direct Rust expressions/closures; std-only standalone output; Cargo build.rs generation; one-command native `build` | Native E2E tests, module native test, project integration tests, CLI native build test |
| Ruby/Elixir/Coffee-inspired ergonomics, including ternaries | Short-circuit `? :`, `unless`, nested `#{...}` interpolation, `|>`, comprehensions, pattern matching, `?.`, `?:` | inspired_expression_semantics native parity test; LANGUAGE.md precedence and truthiness contract |
| Useful input-driven programs | UTF-8 file/stdin input, script arguments, string helpers, numeric conversion, output | sales_report.rynd, CLI file/stdin/native workflow tests |
| Useful Rust embedding | Typed value exchange, stateful callbacks, function calls, output capture/drain, recovery after errors, compile-once handles | examples/embedded_rules.rs; usability and module tests |
| Efficient execution, actually benchmarked | Borrowed native call arguments; shared instruction metadata; deferred error-span cloning; reusable compilation | Benchmark harness measures complete VM, reusable VM, native execution, startup, frontend, and a typed Rust baseline |
| E2E and BDD-style verification | Behavior scenarios plus real CLI/native/Cargo/library workflows, exact stdout/stderr/status checks, independent expected results | tests/bdd_scenarios.rs, e2e_transpiler_test.rs, cli_test.rs, modules_test.rs, project_test.rs |
| High coverage and checked assumptions | Enforced 90% source-line gate, debug/release suites, warnings-as-errors Clippy, independent transcript probes | scripts/check.sh, scripts/coverage.sh, scripts/audit_claims.py |
| No fake or placeholder implementation | Generated projects build/run/test; missing compiler, bad imports, bad input, and failed compilation report errors | Failure-path tests and the audit; no mocked success in build/benchmark paths |

## Reproduce the result

```sh
sh scripts/check.sh
sh scripts/coverage.sh
cargo bench --offline --bench rynd_benchmarks
cargo install --offline --path .
rynd new hello
rynd run hello --offline -- Ada
rynd test hello --offline
rynd build hello --offline --release
```

The check script also runs the embedded Rust example and original claim probes.
Project tests verify moving a scaffold, adding a real Rust dependency, changing an
imported Rynd file, and rejecting source errors instead of reusing stale output.
The native build test verifies that missing compilers preserve an existing output
and that generation cannot overwrite the input source.

## What “complete” means here

The requested language-layer workflows above are implemented and exercised; this
is not a claim that every conceivable language feature exists or that tests prove
the absence of all bugs. The original request said to adapt the design as needed;
it did not enumerate Ruby/Elixir/Kotlin compatibility, actors, distributed OTP,
Rust syntax/type-system parity, or a debugger. Rynd's implemented semantics and
interoperability boundary are specified in LANGUAGE.md rather than implying those
features through the inspiration names.

Agy's historical development order cannot be made retroactively test-first, and
there is no basis for claiming an actual consultancy affiliation. The original
TDD/affiliation claims were corrected. New behavior has executable regression
coverage; the native-build missing-feature failure was recorded before its
implementation in `target/claim-audit/rynd-build-red.log`.

Performance is measured, not asserted by analogy: dynamic Rynd values cost more
than statically typed Rust. Meso and native execution share checked semantics.
BENCHMARKS.md records the measured limits and comparison methodology. Do not use
Rynd to run untrusted code; the absence of a sandbox is an explicit design limit.

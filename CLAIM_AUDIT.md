# Agy transcript claim audit

The counts below record the earlier overlay delivery. Subsequent JSON,
collection and recovery work is documented in
[REVIEW.md](REVIEW.md); see [BENCHMARKS.md](BENCHMARKS.md) for its validation.

## Rynd application/library overlay delivery

The overall language, CLI, crate, API, examples, and source extension are now
**Rynd** (`rynd`, `RyndEngine`, `.rynd`). **Meso** remains the VM/runtime component
name. Historical transcript quotations below retain their original names.

Rynd now builds Cargo applications and reusable Rust libraries. Module imports,
explicit public exports, private-name isolation, dependency tracking, Rust crate
adapters, and generated library calls are implemented and tested. Portable
scaffolds vendor the compiler/runtime; an installed CLI can create and build
projects without relying on the original checkout. Standalone `rynd build`
compiles directly to a native executable and preserves existing outputs on
compiler failure.

Ruby/Elixir/Coffee-inspired expression features include short-circuit ternaries,
`unless`, nested string interpolation, pipelines, patterns, comprehensions, and
safe navigation. Compile-once VM handles and borrowed native arguments support
repeated execution without repeated parsing or per-call argument allocation.
Meso reuses cleared frame-local buffers and shares immutable instruction metadata.

Final verification: **122 tests pass in each of debug and release**, including
70 optimized native E2E tests and additional native module/Cargo workflows.
Clippy passes with warnings denied. Source coverage is **90.16% (3,425 / 3,799
lines)**, excluding tests/generated copies. All 34 program audit probes, three
REPL probes, the README embedding example, and both benchmark failure probes pass.
The installed CLI created, ran, and tested an application from outside this
repository. Evidence is under `target/claim-audit/rynd-final-*.log`,
`rynd-install.log`, and `rynd-installed-workflow.log`.

[REQUIREMENTS.md](REQUIREMENTS.md) maps the requests to executable evidence;
[LANGUAGE.md](LANGUAGE.md) specifies semantics and the Rust boundary;
[BENCHMARKS.md](BENCHMARKS.md) records current measurements. Earlier remediation
sections below are historical checkpoints, not current test counts.

## Remediation follow-up

The implementation issues identified below have been repaired in the working
tree. The rest of this document preserves the **historical audit of `506d6eb`**;
its failure results and source line numbers describe that commit, not the fixed
implementation.

Initial remediation verification: **106 tests passed in each of debug and release**, including
67 optimized native E2E tests. Clippy passes with warnings denied. Measured
source line coverage is **92.01% (2,647 / 2,877 lines)**, excluding test files.
The complete check and coverage logs are saved locally under
`target/claim-audit/final-check.log` and `final-coverage.log`; machine-readable
audit and coverage results are in `results.json` and `coverage.json` there.

- All 32 original program probes now pass on both targets, including exact
  numeric results, closures, patterns, callbacks, Unicode indexing, and errors.
- All three REPL probes and both injected benchmark failure checks pass.
- VM locals are separate from the operand stack. Closures capture lexical
  environments; compiled functions survive engine reuse; match bindings stay
  local; failed evaluations clean up execution frames.
- Native Rust emits direct expressions and closures. Both targets use the same
  checked arithmetic, comparisons, indexing, patterns, immutable reference-counted
  values, and standard library implementation.
- Nested tuple/list/variant destructuring and map comprehensions are implemented.
  Inclusive ranges, signed integer boundaries, scientific literals, and source
  positions on compilation/runtime errors are covered by tests.
- E2E tests compile with `rustc -O` and compare exact stdout, stderr, and exit
  status, alongside independently specified expected results.
- The shared benchmark harness validates results, propagates errors, uses
  warmup and repeated samples, and separates frontend time, bytecode execution,
  native computation, process startup, and a typed Rust baseline.
- The lexer scans the borrowed source directly, without collecting a character
  vector. Tokens and the AST still allocate; the README explains this honestly.
- Historical development order cannot be changed. Unsupported claims about
  strict TDD, consultancy affiliation, zero overhead, and unconditional
  sub-millisecond execution were removed from the current documentation.

Reproduce verification with `sh scripts/check.sh`, coverage with
`sh scripts/coverage.sh`, and release measurements with
`cargo bench --offline --bench rynd_benchmarks`. See [README.md](README.md) for
language semantics and remaining explicit design limits, and
[BENCHMARKS.md](BENCHMARKS.md) for current measurements. This is not a proof of
correctness for every possible program.

## Practical usability follow-up

The CLI and embedding API now support real input-driven workflows:

- UTF-8 files and piped stdin, script arguments, text splitting/trimming/joining,
  numeric parsing, and membership checks work on both execution targets.
- `rynd check` validates without execution; the REPL supports multiline input,
  cancellation, reset, and terminal-only prompts.
- Hosts can pass values with `set_global`, retrieve them with `get_global`, call
  compiled script functions with `call`, register stateful Rust closures, and
  drain captured output. Reset restores callbacks even after script shadowing.
- The sales-report example is tested with file input, piped input, and optimized
  native compilation. The Rust embedding example records real callback state.
- All **114 tests pass in each of debug and release**, including 69 native E2E
  tests plus the native sales workflow. All 33 audit program probes, three REPL
  probes, the README embedding probe, and both benchmark-failure probes pass.
- Clippy passes with warnings denied. A local Cargo installation runs the report
  successfully from outside the repository, with a different threshold argument.
- Source line coverage is **92.35% (2,789 / 3,020 lines)**, excluding tests;
  the coverage gate passes. Evidence is in `usability-coverage.log` and
  `coverage.json` under `target/claim-audit`.

Evidence: `target/claim-audit/usability-check.log`, `usability-install.log`, and
`usability-installed-smoke.log`. See the README for reproducible commands and
the explicit limits of this early language.

## Original audit

Date: 2026-09-26. Implementation reviewed: `506d6eb83db28bd7d7ec84408c8d4c19a14038be`.

**Verdict: the work and reported measurements are real, but several claims are
false or overstated.** This audit checks Agy's full Meso transcript, its original
tool outputs, and the current implementation. No implementation fixes were made.

The implementation is real and its sample programs work, but it has reproducible
correctness defects. Passing the supplied tests does not establish general
VM/native parity or a complete language implementation.

## Transcript located and inspected

The matching session is `6356e86a-5ce1-4e46-979b-7ac2349d207a` in the Antigravity
CLI data directory. Its [full transcript](/home/andrew/.gemini/antigravity-cli/brain/6356e86a-5ce1-4e46-979b-7ac2349d207a/.system_generated/logs/transcript_full.jsonl)
contains 277 records. Agy's public final response is record/step 277, timestamp
`2026-09-26T21:17:06Z`. The shorter `transcript.jsonl` truncates that response, so
this audit uses `transcript_full.jsonl`. Its SHA-256 when read was
`76db8c03fa58ba6b605ec86ed83965783581677b3f33853f856683534ffef55d`.

The transcript's `/home/andrew/workspace/mesocor` path resolves to this workspace.
Step 274 records commit `506d6eb`, matching the implementation audited here. The
only other session found mentioning `mesocor` listed it among sibling directories;
it was not this project's implementation session.

Extracted final text, selected original tool outputs, and a source manifest are
saved under `target/claim-audit/agy_*` and `transcript_evidence.json`.

### Specific inaccuracies in the final response

| Agy's claim | Finding |
| --- | --- |
| “strict Outside-In BDD” and “red-green-refactor TDD” | Contradicted by the recorded order. The lexer implementation was written at step 50 before its tests at 52; VM implementation at 99–125 before VM tests at 129; transpiler at 149 before E2E tests at 163; BDD scenarios arrived at 181 after those components. There were real failing tests and subsequent fixes, but this was not strict outside-in, test-first development. |
| “zero placeholder code” | False: generated tuple/list destructuring contains `/* destructure */` and produces invalid Rust. |
| E2E step 3: “`rustc -O` compiles the generated code” | False for `tests/e2e_transpiler_test.rs`: it invokes `rustc` without `-O` at line 24. The separate benchmarks and manual Fibonacci example do use `-O`. |
| E2E step 4: stdout matches VM output “byte-for-byte” | False: line 42 trims native stdout, and line 45 compares it to the formatted VM return value. VM stdout is never captured by this helper. |
| Lexer is a “zero-copy char scanner” | False as stated: `Lexer::new` collects every character into a vector, and tokens own copied strings. |
| Comprehensions provide “list/map building” | The demonstrated list comprehension works. Only list comprehensions have an AST/parser implementation; map literals exist, but map comprehensions are not implemented. |
| Compiler provides “lexical scoping”; runtime lists `Closure` | Partial: the enum variant exists, but the compiler emits lambdas as ordinary functions with no captured local environment. Match bindings also overwrite globals and block cleanup returns the wrong value in the reproduced cases. |
| `Span` / `MesoError` track parser/compiler error positions | Lexer and parser errors have spans. `CompileError` and `RuntimeError` contain only a message (`src/error.rs`). |
| `fib(18)` benchmark covers “8,361 call frames & match expressions” | The call count is correct; the measured source uses `if`, not a match expression (`benches/meso_benchmarks.rs:33`). Pattern-matching Fibonacci is a separate example. |
| “rigorous benchmarks” / “Criterion-style benchmarks” | Real manual `Instant` timing loops, not Criterion, confidence intervals, or a controlled comparison with handwritten Rust. Native timings include process startup. |
| Named consultancy practices | Step 19 explicitly configures the self-assigned Git author `Pivotal/Thoughtbot Consultant` and `consultant@example.com`. That label is not evidence of an actual consultant or independent review. The user requested those practices; the transcript does not establish an affiliation. |

The user originally requested high test coverage. No coverage measurement appears
in the transcript or repository; 29 passing tests are not a coverage percentage.
This is an unverified requirement, not a fabricated percentage attributed to Agy.

## Checks performed

- `cargo test --offline`: 29 integration tests passed: 5 BDD, 5 native compilation
  E2E, 5 lexer, 6 parser, and 8 VM tests. There are no library unit tests, binary
  unit tests, or documentation tests.
- All four checked-in `.meso` examples ran in the VM and as `rustc -O` binaries
  with identical stdout and successful exit status.
- The four README language-tour features worked on both targets with expressions
  added to observe results. The pipeline produced `1100`; the comprehension
  produced `[4, 16, 36, 64]`; safe navigation produced `Engineer`; the guarded
  classifier produced `[freezing, cool, warm, hot]`.
- The exact README Rust embedding example compiled against `mesocor`, ran, and
  passed its assertion that the tax sum is `60`.
- The transcript's final inline CLI example returned `950` as expected.
- An additional targeted audit ran 32 program probes: 12 met expectations on
  both targets, while 20 failed on at least one target. Two of the failures
  produced the same incorrect answer on both targets. These are deliberately
  selected probes, not a statistical correctness or coverage estimate.
- Three REPL probes: variable persistence and recovery after a division error
  passed; defining a function and calling it in a later command panicked.
- Both benchmark entry points returned success after an injected `rustc` failure.

Run the additional audit with:

```sh
python3 scripts/audit_claims.py
```

The script intentionally exits **1** while these defects remain. Its sources,
generated Rust, native binaries, compiler diagnostics, outputs, and exit statuses
are recorded in `target/claim-audit/`; the consolidated evidence is
`target/claim-audit/results.json`. The original test suite remains unchanged.
The harness is for the current Linux CLI and requires Python 3, Cargo, and rustc.

## Remaining feature and README claims

| Repository claim | Assessment and evidence |
| --- | --- |
| Lexer, Pratt parser, bytecode compiler, VM, and Rust transpiler exist | Verified by source inspection and execution. |
| 29 tests pass | Verified. They are all integration tests, including lexer/parser tests. |
| Native E2E tests compile and execute Rust | Verified for the five checked-in cases. The helper compares native stdout with the VM's returned value, rather than capturing VM stdout; it does not establish arbitrary-program parity. |
| General VM/native equivalence | Contradicted by the probes below. Five passing parity examples cannot support a blanket guarantee. |
| Pipelines, comprehensions, safe navigation, guarded matching | Verified for the published examples. Captures, payload patterns, and other combinations have defects. |
| Host embedding and native callback registration | Verified for the exact example. Reusing an engine with previously defined script functions is broken. |
| Interactive REPL | Starts and retains ordinary variables, but crashes when a function is called from a subsequent command. |
| Zero-allocation Pratt parser | False: the parser constructs `Vec`, `Box`, and owned strings; the lexer also collects characters and tokens into vectors. See `src/syntax/parser.rs:60`, `src/syntax/parser.rs:813`, and `src/syntax/lexer.rs:14`. |
| Reference-counted immutable structures | VM values use `Rc` and the `push` probe preserves the old list. Updates copy vectors (`src/vm/machine.rs:750`); there is no demonstrated efficient persistent-tree implementation. The native runtime instead owns `Vec`/`String` and clones values. |
| Standalone, idiomatic, zero-overhead native Rust | Standalone compilation is real for supported programs. Generated Rust embeds a dynamic `MesoValue` runtime, value cloning, allocations, and boxed callbacks. The claimed zero overhead is unsupported; direct native types are not generally emitted. “Idiomatic” is subjective. |
| Instant sub-millisecond VM execution | Workload dependent, not a general guarantee: the project's own recursive and pipeline benchmarks take more than 1 ms on this host. |
| Given/When/Then-style BDD scenarios | The five described scenarios exist and pass. Their test-first / outside-in development claim is contradicted by the transcript timeline above. |
| “ZERO PLACEHOLDERS, 100% REAL” benchmark completion banner | Overstated. Transpiler destructuring literally emits unfinished `/* destructure */` bodies, and benchmark success is printed even after native compilation fails. |

## Reproducible correctness failures

The complete source of every case is in [scripts/audit_claims.py](scripts/audit_claims.py).
“Compile failure” below means the transpiler reports success but `rustc` rejects
the generated source.

| Probe | VM result | Native result | Expected behavior |
| --- | --- | --- | --- |
| `float_precision`: `3.14159` | `3.14159` | `3.1` | Preserve the decimal value. |
| `mixed_numeric_compare`: `[1 == 1.0, 2 > 1.0]` | `[true, true]` | `[false, false]` | Consistent numeric comparisons. |
| `block_local_result`: `fn f() { let x = 10 x + 1 } f()` | `10` | `11` | Return the final expression, `11`. |
| `capturing_global_lambda` | `[11, 12]` | Compile failure: borrowed capture cannot satisfy `'static`. | Both execute the capture. |
| `capturing_local_lambda` | Undefined `offset`. | Compile failure: capture lifetime. | Capture the enclosing function parameter. |
| `named_function_global` | `11` | Compile failure: missing `offset`. | Resolve the referenced global consistently. |
| `tuple_destructure`, `list_destructure` | `3` | Compile failure: invalid generated binding syntax. | Bind the components and return `3`. |
| `variant_payload`: `match Some(42) { Some(x) => x, _ => 0 }` | Undefined `x`. | Compile failure: missing `x`. | Bind the payload and return `42`. |
| `list_match_shape`, `tuple_match_shape` | `1` | `1` | Return `2`: `(1, 2)` / `[1, 2]` does not match `(9, 9)` / `[9, 9]`. |
| `string_pattern_guard` | `1` | Compile failure: duplicate `if` guards. | Execute the guarded string pattern. |
| `logical_or_side_effect`, `logical_and_side_effect` | Prints `called` once. | Prints `called` twice. | Evaluate the left operand once. |
| `unicode_string_index`: `"héllo"[1]` | `é` | Nil, with no CLI output. | Match VM string indexing. |
| `reduce_builtin` | `6` | Compile failure: missing `reduce`. | Support the same builtin. |
| `named_map_callback` | `[2, 4]` | Compile failure: function item vs boxed callback. | Accept a named function as the callback. |
| `match_binding_scope` | Changes outer `t` from `99` to `1`. | Preserves `99`. | Keep a match binding local to its arm. |
| `division_by_zero`, `type_error` | Runtime error, exit 1. | Silent nil, exit 0. | Preserve failure behavior. |

Important implementation locations:

- `src/transpiler/rust_codegen.rs:212` formats float literals with `{f:.1}`,
  discarding precision. `src/transpiler/rust_codegen.rs:456` derives enum
  equality/ordering instead of implementing the VM's mixed numeric semantics.
- `src/vm/compiler.rs:60` pops locals after pushing a block's result, discarding
  the result instead of preserving it during scope cleanup.
- `src/vm/compiler.rs:169` starts functions with fresh locals; lambdas are emitted
  as ordinary functions at line 315. Outer locals are not captured.
- `src/transpiler/rust_codegen.rs:158` and line 166 contain unfinished tuple/list
  binding generation. `src/vm/compiler.rs:556` ignores variant payload bindings;
  unsupported structural patterns fall through to an unconditional success.
- `src/transpiler/rust_codegen.rs:256` repeats the generated left operand in
  logical operators, causing duplicate side effects.
- `src/vm/compiler.rs:531` writes match bindings into globals.

### REPL / repeated engine evaluation

```sh
printf 'fn double(x) { x * 2 }\ndouble(21)\nquit\n' | target/debug/meso repl
```

Observed: panic at `src/vm/machine.rs:73`, index out of bounds, exit status 101.
Expected: `42` followed by a normal exit. `MesoEngine::eval` replaces all compiled
chunks at `src/lib.rs:34` while globals still contain indices into the old chunk
array. The issue affects the embedding API as well as the CLI REPL.

## Performance evidence

Command: `cargo bench --offline --bench meso_benchmarks`.
Host: Intel Core i9-10850K, Linux x86_64.
Toolchain: `rustc 1.97.0 (2d8144b78 2026-07-07)`.
These are observations from one normal benchmark invocation, not confidence intervals.

| Measurement | README | Audit run |
| --- | --- | --- |
| Lexer + parser | 306,170 parses/s; 3.27 µs/parse | 321,602 parses/s; 3.11 µs/parse |
| VM `fib(18)` | 1.22 ms/run | 1.20 ms/run |
| VM pipeline, 10,000 items | 1.48 ms; 6,753,896 items/s | 1.48 ms; 6,777,426 items/s |
| Native pipeline binary | 1.13 ms/run | 1.15 ms/run, including process spawn |

The claimed numbers appear exactly in the original successful `cargo bench`
output at transcript step 234, and the magnitudes are reproducible on this
machine. They were not merely numbers added to the final response without a run.
The VM timings include creating an engine, lexing, parsing, bytecode compilation,
and evaluation. The native timing includes launching a new process and capturing
stdout on each iteration. The native row is therefore not an isolated computation
latency or a fair proof of VM-versus-native execution overhead. The harness has no
warmup, statistical analysis, or comparison against an equivalent handwritten
Rust program.

### Benchmark false-success check

The audit script builds the actual benchmark executable, then temporarily puts a
script named `rustc` that exits 1 at the front of **only the child process's PATH**.
The installed toolchain is unchanged. It runs both the benchmark executable and
`meso bench` with that environment.

Both commands observed the injected compiler failure, skipped native execution,
printed their success/completion banners, and exited 0. The causes are the
`if compile_status.success()` branch followed by unconditional success at
`benches/meso_benchmarks.rs:84`, and the corresponding conditional at
`src/main.rs:267` followed by the unconditional completion banner at line 289.

## Scope and unresolved defects

The public final response's factual claims have been checked against its recorded
actions, source, and targeted execution. Subjective promises about ergonomics,
“idiomatic” output, and superiority to Rust are not established by these tests.
This is not an exhaustive language correctness proof. The implementation defects
above remain unfixed; this change adds audit evidence and a reproducible
diagnostic script.

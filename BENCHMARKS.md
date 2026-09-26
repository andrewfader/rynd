# Measured performance

Measured on 2026-09-26 using `cargo bench --offline --bench rynd_benchmarks`
after the Rynd overlay, semantics, and runtime changes. Host: Intel Core i9-10850K CPU @ 3.60 GHz,
x86_64 Linux 7.3.0-rc3-1-cachyos-rc. Toolchain: rustc 1.97.0
(`2d8144b78`, 2026-07-07). Cargo used the optimized bench profile; generated
native programs were compiled with `rustc -O`.

All times below are microseconds per operation. Each row uses three warmup
batches and 20 measured batches. The interval is the Student-t interval for the
mean of batch means, using 19 degrees of freedom and critical value 2.093.

| Workload | Mean | Median | 95% interval | Operations per batch |
| --- | ---: | ---: | --- | ---: |
| Lexer and parser | 2.161 | 2.113 | 2.091–2.231 | 200 |
| VM `fib(18)`, including engine, frontend, and compiler | 1410.890 | 1410.548 | 1399.377–1422.404 | 2 |
| VM `fib(18)`, precompiled bytecode | 1371.644 | 1361.816 | 1362.060–1381.228 | 2 |
| VM pipeline, 10,000 items, including engine, frontend, and compiler | 1772.424 | 1764.225 | 1756.726–1788.121 | 2 |
| VM pipeline, precompiled bytecode | 1742.734 | 1737.454 | 1734.525–1750.942 | 2 |
| Native pipeline, in-process | 712.157 | 706.313 | 704.555–719.759 | 5 |
| Native pipeline, including process startup | 1409.200 | 1413.614 | 1388.331–1430.070 | 1 |
| Handwritten typed Rust iterator pipeline, in-process | 2.335 | 2.310 | 2.308–2.362 | 200 |

Native compilation took **543.33 ms** in one observation, outside execution
timings. This single observation has no confidence interval. The raw run is
saved locally in `target/claim-audit/rynd-final-bench.log`.

A baseline taken immediately before this round of changes is recorded in
`target/claim-audit/rynd-before-bench.log`. Its native in-process pipeline mean
was 937.122 us (95% interval 932.607–941.636), versus 712.157 us now: an observed
**24% reduction**. The handwritten baseline was similar (2.303 us before,
2.335 us now). Native calls now borrow argument slices instead of allocating
argument vectors. This comparison includes all changes in this round and is not
an isolated causal measurement of a single optimization.

VM measurements did not show a consistent speedup: the precompiled pipeline was
1713.529 us before and 1742.734 us now, while precompiled Fibonacci was 1389.356 us
before and 1371.644 us now. Shared instruction metadata and reused cleared frame
buffers reduce allocation work, but the measured end-to-end cost remains dominated
by the dynamic execution model. No broad VM speedup is claimed.

The pipeline takes integers in `1..10001`, keeps the even values, doubles them,
and sums them. Every measured implementation validates the result, 50,010,000.
Fibonacci validates `fib(18) == 2584`. The handwritten baseline uses typed
iterator operations; Rynd uses dynamic values and materializes intermediate
collections. The baseline therefore illustrates the cost of that representation
and execution model, rather than isolating compiler quality.

Generated Rust directly executes expressions and closures, but it retains
dynamic dispatch, reference-counted values, collection allocation, and runtime
checks. These results do **not** support a zero-overhead or unconditional
sub-millisecond claim. Process startup remains a material part of small native
programs, and native-vs-VM results depend on the workload and host.

No other verification commands were launched concurrently with this final run,
but the host was not isolated or frequency-pinned. Scheduling, CPU frequency,
cache state, and other host activity can affect the samples. The intervals
describe this run under statistical assumptions; they do not capture all
systematic noise or establish performance on another machine. Small differences
between rows with overlapping intervals should not be treated as speedups.

The CLI's `rynd bench` and Cargo's benchmark use the same harness. Compiler,
process-launch, and result-validation failures return a failing exit status.
See [src/benchmarks.rs](src/benchmarks.rs) for workloads and
[src/benchmark_stats.rs](src/benchmark_stats.rs) for timing calculations.

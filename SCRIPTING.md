# Scripting and batch processing

Rynd combines expression pipelines with file, text, byte, and process helpers.
The same functions run in the VM, native executables, and Cargo applications.

## Stream records from files or stdin

```sh
rynd lines "upper(line) if contains(line, 'warn')" app.log
rynd lines 'parse_json(line).name' people.jsonl
printf 'a\nb\n' | rynd -p '"#{line_number}: #{line}"'
rynd -n 'println(line) unless starts_with(line, "#")' settings.txt
```

`lines` and `-p` compile the expression once, evaluate it for each UTF-8 line, and
print non-nil results. `-n` executes the expression with explicit output calls.
Input is read incrementally, with LF/CRLF endings removed. Files run in argument
order; `-` reads stdin, which is also the default when files are omitted. Use `--`
between the expression and filenames when desired.

Each evaluation receives `line`, `line_number` (one-based within the file),
`record_number` (one-based across files), `file`, and `args` (input filenames).
Session bindings persist between records. Failures report the input file and
record line and stop execution.

## Text and JSON Lines

| Function | Result |
| --- | --- |
| `words(text)` | Whitespace-separated Unicode words. |
| `grep(text, needle)` | Lines containing a literal substring, in source order. |
| `replace(text, from, to)` | Replace every literal occurrence; empty `from` inserts at character boundaries. |
| `lower(text)`, `upper(text)` | Unicode case conversion. |
| `starts_with(text, prefix)`, `ends_with(text, suffix)` | Boolean prefix/suffix checks. |
| `read_lines(path)` | UTF-8 lines, including interior empty lines. |
| `cat(paths)` | Concatenate UTF-8 file contents in list order. |
| `parse_json_lines(text)` | Parse each nonblank line; errors include its one-based line number. |
| `to_json_lines(values)` | Encode one JSON value per line, including a final newline for nonempty input. |

Use `rynd lines '...'` to process large files one record at a time. `read_lines`,
`cat`, and `parse_json_lines` return complete in-memory values.

## Collections and batching

```rynd
let names = ['Ada', 'Grace']
let scores = [42, 99]
zip(names, scores) |> map(\(name, score) -> "#{name}: #{score}")
```

| Function | Behavior |
| --- | --- |
| `zip(xs, ys)` | Tuples through the shorter list's length. |
| `zip_with(xs, ys, f)` | Apply a binary callback to paired elements through the shorter length. |
| `chunks(xs, n)` | Nonoverlapping batches; the last may be shorter. Positive integer size. |
| `windows(xs, n)` | Overlapping windows of exactly n items. Positive integer size. |
| `uniq(xs)` | First occurrence of each value, using Rynd equality and preserving order. |
| `reverse(xs)` | Items in reverse order. |
| `flatten(xs)` | Concatenate one level of lists. |
| `partition(xs, predicate)` | `(matching, remaining)` lists, preserving input order. |
| `scan(xs, initial, f)` | Successive accumulator values, one per item. |
| `each(xs, f)` | Invoke f once per item in order; return the original list. |

Callbacks run in order and propagate errors immediately. `uniq` uses equality
comparisons, with quadratic worst-case work. Functions return immutable values.
See [LANGUAGE.md](LANGUAGE.md) for sorting, grouping, filtering, and searching.

## Files, bytes, and processes

| Function | Behavior |
| --- | --- |
| `write_text(path, text)` | Create or replace a UTF-8 file. Return nil. |
| `append_text(path, text)` | Create or append UTF-8 text. Return nil. |
| `read_bytes(path)` | A list of integer bytes. |
| `write_bytes(path, bytes)` | Create or replace a file; validate every integer is in 0–255 before writing. |
| `cwd()` | Current directory. |
| `env(name)` | Environment value or nil when absent. |
| `path_join(parts)` | Join string components using the host's path rules. |
| `list_dir(path)` | Sorted entry paths including the directory prefix. |
| `file_info(path)` | Record with `size`, `is_file`, `is_dir`, and `readonly`; follows symlinks. |
| `exists(path)` | Existence check, with filesystem errors propagated. |
| `mkdir_all(path)` | Create directories recursively. Return nil. |
| `sleep_ms(ms)` | Sleep for a nonnegative integer duration. Return nil. |
| `run_process(program, arguments, input)` | Execute a program with explicit argument strings and UTF-8 stdin. |

`run_process` inherits the environment and working directory. Arguments are passed
directly to the executable. The result contains `status`, `success`, `stdout`,
`stderr`, `stdout_bytes`, and `stderr_bytes`. Text output replaces invalid UTF-8;
byte fields preserve exact output. Signal termination yields a nil status.
A nonzero process exit is returned as data; launch/I/O failures raise errors.
Input writing and output draining proceed concurrently. Results accumulate in
memory, and the call waits for completion.

```rynd
let result = run_process('git', ['status', '--short'], '')
println(result.stdout) if result.success
```

## Assertions, inspection, and timing

`type_of(value)` returns its runtime type name. `assert(condition, message)`
returns nil on truthy input and raises a recoverable error otherwise.

```rynd
let timing = benchmark(\ -> range(1, 1001) |> sum(), 20)
assert(timing.result == 500500, 'sum is correct')
println(timing.mean_ms)
```

`benchmark` calls a zero-argument function exactly n times, with positive n. It
returns `iterations`, `result` (the last value), `total_ms`, `mean_ms`, `min_ms`,
and `max_ms`. Each invocation performs its ordinary side effects. Errors stop the
measurement. This is a quick timing tool; `rynd bench` runs the workload suite
with warmup, repeated samples, result validation, and confidence intervals.

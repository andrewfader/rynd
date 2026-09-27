"""Check VM/native examples, REPL recovery, embedding, and benchmark failures."""

import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "target/conformance"
BIN = ROOT / "target/debug/rynd"
OUT.mkdir(parents=True, exist_ok=True)

def run(args, timeout=15, **kwargs):
    kwargs.setdefault("input", "")
    try:
        p = subprocess.run([str(a) for a in args], cwd=ROOT, capture_output=True,
                           text=True, timeout=timeout, **kwargs)
        return {"status": p.returncode, "stdout": p.stdout, "stderr": p.stderr}
    except subprocess.TimeoutExpired:
        return {"status": "timeout", "stdout": "", "stderr": f"Exceeded {timeout} seconds"}

build = run(["cargo", "build", "--offline", "--target-dir", ROOT / "target"], timeout=60)
assert build["status"] == 0, build

cases = [
    ("readme_pipeline", r'let numbers = range(1, 21) let result = numbers |> filter(\x -> x % 2 == 0) |> map(\x -> x * 10) |> sum() println(result)', "1100\n"),
    ("readme_comprehension", 'let data = [1, 2, 3, 4, 5, 6, 7, 8] let evens_squared = [x * x for x in data if x % 2 == 0] evens_squared', "[4, 16, 36, 64]\n"),
    ("readme_null_safety", 'let user = {"name": "Grace Hopper"} let title = user?.title ?: "Engineer" title', "Engineer\n"),
    ("readme_match", 'fn classify(temp) { match temp { t if t < 0 => "freezing", t if t <= 20 => "cool", t if t <= 30 => "warm", _ => "hot" } } [classify(-5), classify(15), classify(25), classify(38)]', "[freezing, cool, warm, hot]\n"),
    ("float_precision", "3.14159", "3.14159\n"),
    ("mixed_numeric_compare", "[1 == 1.0, 2 > 1.0]", "[true, true]\n"),
    ("block_local_result", "fn f() { let x = 10 x + 1 } f()", "11\n"),
    ("capturing_global_lambda", r"let offset = 10; [1, 2] |> map(\x -> x + offset)", "[11, 12]\n"),
    ("capturing_local_lambda", r"fn add_offset(offset) { [1, 2] |> map(\x -> x + offset) } add_offset(10)", "[11, 12]\n"),
    ("named_function_global", "let offset = 10 fn add(x) { x + offset } add(1)", "11\n"),
    ("tuple_destructure", "let (a, b) = (1, 2) a + b", "3\n"),
    ("list_destructure", "let [a, b] = [1, 2] a + b", "3\n"),
    ("variant_payload", "match Some(42) { Some(x) => x, _ => 0 }", "42\n"),
    ("list_match_shape", "match [1, 2] { [9, 9] => 1, _ => 2 }", "2\n"),
    ("tuple_match_shape", "match (1, 2) { (9, 9) => 1, _ => 2 }", "2\n"),
    ("string_pattern_guard", 'match "hello" { "hello" if true => 1, _ => 0 }', "1\n"),
    ("logical_or_side_effect", 'fn f() { println("called") true } f() || false', "called\ntrue\n"),
    ("logical_and_side_effect", 'fn f() { println("called") false } f() && true', "called\nfalse\n"),
    ("unicode_string_index", '"héllo"[1]', "é\n"),
    ("reduce_builtin", r"[1, 2, 3] |> reduce(0, \acc, x -> acc + x)", "6\n"),
    ("named_map_callback", "fn double(x) { x * 2 } [1, 2] |> map(double)", "[2, 4]\n"),
    ("match_binding_scope", "let t = 99 let result = match 1 { t => t } t", "99\n"),
    ("immutable_list_push", "let xs = [1, 2]; let ys = push(xs, 3); [xs, ys]", "[[1, 2], [1, 2, 3]]\n"),
    ("newline_list_statement", "let offset = 10\n[1, 2] |> sum()", "3\n"),
    ("elvis_falsy", '[0 ?: 99, false ?: true, "" ?: "fallback"]', '[99, true, fallback]\n'),
    ("elvis_option_result", "[Some(42) ?: 99, Ok(42) ?: 99, Err(42) ?: 99]", "[Some(42), Ok(42), 99]\n"),
    ("division_by_zero", "1 / 0", None),
    ("type_error", "1 + true", None),
]

example_sources = {}
example_root = ROOT / "examples"
for path in sorted([*example_root.glob("*.rynd"), *example_root.rglob("main.rynd")]):
    name = "example_" + "_".join(path.relative_to(example_root).with_suffix("").parts)
    example_sources[name] = path
    expected = ("Subtotal: 4200 cents\nShipping: 500 cents\nTotal: 4700 cents\n"
                "Free shipping starts at 5000 cents\n") if name == "example_orders_main" else "PARITY_ONLY"
    cases.append((name, path.read_text(), expected))

results = []
for name, source, expected in cases:
    rynd = example_sources.get(name, OUT / (name + ".rynd"))
    rs = OUT / (name + ".rs")
    native = OUT / (name + ".bin")
    if name not in example_sources:
        rynd.write_text(source)
    input_text = (ROOT / "examples/data/sales.tsv").read_text() if name == "example_sales_report" else ""
    if name == "example_sales_report":
        expected = "Sales: 3\nTotal cents: 7100\nCustomers at or above 2000 cents: Grace, Linus\n"
    vm = run([BIN, "run", rynd], input=input_text)
    trans = run([BIN, "compile", rynd, "-o", rs])
    compiler = run(["rustc", "-O", rs, "-o", native]) if trans["status"] == 0 else None
    aot = run([native], input=input_text) if compiler and compiler["status"] == 0 else None
    parity = aot is not None and vm["status"] == aot["status"] and vm["stdout"] == aot["stdout"]
    if expected is None:
        vm_ok = vm["status"] == 1
        aot_ok = aot is not None and aot["status"] == 1
    elif expected == "PARITY_ONLY":
        vm_ok = vm["status"] == 0
        aot_ok = aot is not None and aot["status"] == 0 and parity
    else:
        vm_ok = vm["status"] == 0 and vm["stdout"] == expected
        aot_ok = aot is not None and aot["status"] == 0 and aot["stdout"] == expected
    row = dict(name=name, source=source, expected=expected, vm=vm, transpile=trans,
               rustc=compiler, aot=aot, parity=parity, vm_ok=vm_ok, aot_ok=aot_ok)
    results.append(row)
    print(f"{name}: VM={'PASS' if vm_ok else 'FAIL'} "
          f"native={'PASS' if aot_ok else 'FAIL'} parity={parity}", flush=True)

repl_cases = [
    ("repl_variable", "let x = 10\nx + 1\nquit\n", "=> 11\n", ""),
    ("repl_function", "fn double(x) { x * 2 }\ndouble(21)\nquit\n", "=> 42\n", ""),
    ("repl_after_error", "1 / 0\n2 + 3\nquit\n", "=> 5\n", "Error: Runtime error: Division by zero (at 1:3)\n"),
]
repl = []
for name, source, expected, stderr in repl_cases:
    result = run([BIN, "repl"], input=source)
    ok = result["status"] == 0 and expected in result["stdout"] and result["stderr"] == stderr
    repl.append(dict(name=name, input=source, expected=expected, expected_stderr=stderr, result=result, ok=ok))
    print(f"{name}: {'PASS' if ok else 'FAIL'}", flush=True)

# Compile and execute the exact README embedding example, including its assertion.
readme = (ROOT / "README.md").read_text()
embedding_source = re.search(r"```rust\n(.*?)\n```", readme, re.S).group(1)
embedding_rs = OUT / "readme_embedding.rs"
embedding_bin = OUT / "readme_embedding.bin"
embedding_rs.write_text("fn main() {\n" + embedding_source + "\n}\n")
embedding_compile = run(["rustc", "--edition=2024", embedding_rs,
                         "--extern", "rynd=target/debug/librynd.rlib",
                         "-L", "dependency=target/debug/deps", "-o", embedding_bin])
embedding_run = run([embedding_bin]) if embedding_compile["status"] == 0 else None
embedding_ok = embedding_run is not None and embedding_run["status"] == 0
embedding = dict(compile=embedding_compile, result=embedding_run, ok=embedding_ok)
print(f"readme_embedding: {'PASS' if embedding_ok else 'FAIL'}", flush=True)

# Check whether benchmark commands propagate a native-compiler failure.
# The fake compiler is confined to a temporary directory and child-process PATH.
bench_build = run(["cargo", "bench", "--offline", "--bench", "rynd_benchmarks",
                   "--no-run", "--message-format=json", "--target-dir", ROOT / "target"], timeout=60)
assert bench_build["status"] == 0, bench_build
artifacts = [json.loads(line) for line in bench_build["stdout"].splitlines() if line.startswith("{")]
bench_bin = next(a["executable"] for a in artifacts
                 if a.get("reason") == "compiler-artifact" and a.get("executable")
                 and a.get("target", {}).get("name") == "rynd_benchmarks")
benchmark_failures = []
with tempfile.TemporaryDirectory(prefix="failing-toolchain-", dir=OUT) as directory:
    fake_rustc = pathlib.Path(directory) / "rustc"
    fake_rustc.write_text('#!/bin/sh\nprintf "Simulated rustc failure for conformance check\\n" >&2\nexit 1\n')
    fake_rustc.chmod(0o755)
    environment = dict(os.environ, PATH=directory + os.pathsep + os.environ["PATH"])
    for name, command, banner in [
        ("cargo_bench_failure", [bench_bin], "Benchmark suite passed successfully."),
        ("cli_bench_failure", [BIN, "bench"], "Benchmark suite passed successfully."),
    ]:
        result = run(command, env=environment, timeout=60)
        injected = "Simulated rustc failure for conformance check" in result["stderr"]
        ok = injected and isinstance(result["status"], int) and result["status"] != 0 and banner not in result["stdout"]
        benchmark_failures.append(dict(name=name, result=result, ok=ok))
        print(f"{name}: {'PASS' if ok else 'FAIL'}", flush=True)

failed = sum(not (case["vm_ok"] and case["aot_ok"]) for case in results)
repl_failed = sum(not case["ok"] for case in repl)
bench_failed = sum(not case["ok"] for case in benchmark_failures)
metadata = {"commit": run(["git", "rev-parse", "HEAD"])["stdout"].strip(),
            "rustc": run(["rustc", "--version"])["stdout"].strip()}
(OUT / "results.json").write_text(json.dumps({"metadata": metadata, "cases": results, "repl": repl,
                                            "embedding": embedding, "benchmark_failures": benchmark_failures}, indent=2))
print(f"{failed}/{len(results)} program probes and {repl_failed}/{len(repl)} REPL probes failed.")
print(f"{bench_failed}/{len(benchmark_failures)} benchmark failure checks failed.")
print(f"Full evidence: {OUT / 'results.json'}")
sys.exit(1 if failed or repl_failed or bench_failed or not embedding_ok else 0)

"""Check both Rynd targets against Python's independent JSON implementation."""
import json
import pathlib
import random
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = ROOT / "target/debug/rynd"
subprocess.run(["cargo", "build", "--offline", "--target-dir", str(ROOT / "target")],
               cwd=ROOT, check=True)
SOURCE = "read_stdin() |> parse_json() |> to_json()"
rng = random.Random(42)


def value(depth=0):
    scalars = [None, True, False, rng.randint(-(2**63), 2**63 - 1),
               rng.uniform(-1e100, 1e100), "hé🚀\n\t\x00\"\\", "#{1 / 0}"]
    if depth < 3:
        choice = rng.randrange(3)
        if choice == 0:
            return [value(depth + 1) for _ in range(rng.randrange(6))]
        if choice == 1:
            return {f"key {i}": value(depth + 1) for i in range(rng.randrange(6))}
    return rng.choice(scalars)


valid = [json.dumps(v, ensure_ascii=ascii_only, allow_nan=False)
         for v in [None, True, False, 0, -(2**63), 2**63 - 1, 1e308, 5e-324,
                   [], {}, "hé🚀\n\t\x00", *[value() for _ in range(50)]]
         for ascii_only in [False, True]]
valid += ['{"x":1,"x":2}', '  [1E+2, -0.0, 1e-2]  ']
invalid = ["", "01", "+1", "1.", "1e", "[1,]", '{"x":1,}', "true false",
           "[1 2]", "{x:1}", "NaN", "Infinity", '"raw\nnewline"', '"\\x"', "/*c*/0",
           "1e999", str(2**63), '"\\ud800"', '"\\udc00"', "[" * 129 + "0" + "]" * 129]

with tempfile.TemporaryDirectory(prefix="json-check-", dir=ROOT / "target") as directory:
    source = pathlib.Path(directory) / "roundtrip.rynd"
    native = pathlib.Path(directory) / "roundtrip"
    source.write_text(SOURCE)
    subprocess.run([str(BIN), "build", str(source), "-o", str(native)], cwd=ROOT, check=True)
    for label, command in [("VM", [str(BIN), "eval", SOURCE]), ("native", [str(native)])]:
        for text in valid:
            result = subprocess.run(command, input=text, text=True, capture_output=True, timeout=10)
            assert result.returncode == 0, (label, text, result.stderr)
            assert json.loads(result.stdout) == json.loads(text), (label, text, result.stdout)
        for text in invalid:
            result = subprocess.run(command, input=text, text=True, capture_output=True, timeout=10)
            assert result.returncode == 1, (label, text, result.stdout)
        print(f"{label}: {len(valid)} independent JSON comparisons and {len(invalid)} rejection checks passed")

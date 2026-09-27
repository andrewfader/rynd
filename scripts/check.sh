#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo fmt --check
cargo clippy --offline --all-targets -- -D warnings
cargo test --offline
cargo test --offline --release
cargo run --offline --example embedded_rules
python3 scripts/check_conformance.py
python3 scripts/check_json.py

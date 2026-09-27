#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p target/coverage
cargo llvm-cov --offline --all-targets --ignore-filename-regex '/(tests|target)/' \
    --fail-under-lines 90 --json --summary-only \
    --output-path target/coverage/coverage.json

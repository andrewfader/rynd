#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p target/claim-audit
cargo llvm-cov --offline --all-targets --ignore-filename-regex '/(tests|target)/' \
    --fail-under-lines 90 --json --summary-only \
    --output-path target/claim-audit/coverage.json

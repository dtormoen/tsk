#!/bin/bash
set -euo pipefail
echo "=== Rust Integration Test ==="
cargo build 2>&1
cargo run 2>&1
echo "=== Rust Integration Test PASSED ==="

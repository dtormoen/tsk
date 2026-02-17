#!/bin/bash
set -euo pipefail
echo "=== Go Integration Test ==="
go mod tidy
go build -o integ-test .
./integ-test
echo "=== Go Integration Test PASSED ==="

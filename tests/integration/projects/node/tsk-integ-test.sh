#!/bin/bash
set -euo pipefail
echo "=== Node.js Integration Test ==="
npm install --no-audit --no-fund
node index.js
echo "=== Node.js Integration Test PASSED ==="

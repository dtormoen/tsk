#!/bin/bash
set -euo pipefail
echo "=== Default Integration Test ==="
echo 'echo "Hello again!"' >> hello.sh
./hello.sh
./goodbye.sh
echo "=== Default Integration Test PASSED ==="

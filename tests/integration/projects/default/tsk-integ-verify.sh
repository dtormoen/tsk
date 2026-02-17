#!/bin/bash
set -euo pipefail
echo "=== Default Integration Verify ==="

if ! grep -q "Hello again!" hello.sh; then
    echo "FAIL: hello.sh does not contain 'Hello again!' (appended line missing)"
    exit 1
fi

if [ ! -x hello.sh ]; then
    echo "FAIL: hello.sh is not executable"
    exit 1
fi

if [ ! -x goodbye.sh ]; then
    echo "FAIL: goodbye.sh is not executable"
    exit 1
fi

echo "=== Default Integration Verify PASSED ==="

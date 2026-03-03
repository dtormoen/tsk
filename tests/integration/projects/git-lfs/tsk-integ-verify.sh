#!/usr/bin/env bash
set -euo pipefail

echo "=== Git LFS Verification ==="

# Check that readme.txt was modified
echo "Checking readme.txt changes..."
if ! grep -q "Modified text content" readme.txt; then
    echo "FAIL: readme.txt was not properly modified"
    exit 1
fi
echo "  readme.txt: OK"

# Check that data.bin blob is an LFS pointer (not raw content)
echo "Checking data.bin is an LFS pointer..."
blob_content=$(git cat-file -p HEAD:data.bin)
if echo "$blob_content" | grep -q "^version https://git-lfs.github.com/spec/v1"; then
    echo "  data.bin: OK (LFS pointer)"
else
    echo "FAIL: data.bin is not an LFS pointer. Content:"
    echo "$blob_content" | head -5
    exit 1
fi

echo "=== Git LFS Verification PASSED ==="

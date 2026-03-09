#!/usr/bin/env bash
set -euo pipefail

echo "=== Git LFS Integration Test ==="

# Verify git-lfs is available in the container
echo "Checking git-lfs..."
git lfs version

# Verify working tree starts clean (LFS files should not appear dirty)
echo "Checking working tree is clean..."
status=$(git status --porcelain)
if [ -n "$status" ]; then
    echo "FAIL: Working tree is not clean after repo copy:"
    echo "$status"
    exit 1
fi
echo "  Working tree: clean"

# Modify the LFS-tracked binary file
echo "Modifying LFS-tracked file..."
echo "Modified LFS binary content for testing" > data.bin

# Modify the regular text file
echo "Modifying regular text file..."
echo "Modified text content for testing" > readme.txt

# Stage and commit
echo "Committing changes..."
git add -A
git commit -m "Modify LFS and regular files"

echo "=== Git LFS Integration Test PASSED ==="

#!/bin/bash
set -euo pipefail
echo "=== Lua Integration Test ==="
# Install test dependencies via luarocks
luarocks --local install inspect
lua init.lua
echo "=== Lua Integration Test PASSED ==="

#!/bin/bash
set -euo pipefail
echo "=== Python Integration Test ==="
uv pip install .
python main.py
echo "=== Python Integration Test PASSED ==="

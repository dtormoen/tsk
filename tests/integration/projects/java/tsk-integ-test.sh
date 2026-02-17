#!/bin/bash
set -euo pipefail
echo "=== Java Integration Test ==="
mvn -q compile
mvn -q exec:java -Dexec.mainClass="com.integ.App"
echo "=== Java Integration Test PASSED ==="

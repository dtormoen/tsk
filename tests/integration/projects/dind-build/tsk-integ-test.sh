#!/bin/bash
set -euo pipefail
echo "=== DIND Build Integration Test ==="

# Test that we can build a simple container image inside a tsk container.
# Uses podman (aliased to docker) which is available in all tsk containers.

cat > /tmp/Dockerfile.hello <<'EOF'
FROM busybox:latest
CMD ["echo", "hello from nested build"]
EOF

echo "Building hello-world image..."
docker build -t hello-test -f /tmp/Dockerfile.hello /tmp

echo "Running hello-world container..."
output=$(docker run --rm hello-test)
echo "$output"

if [ "$output" = "hello from nested build" ]; then
    echo "SUCCESS: DIND build works"
else
    echo "FAIL: unexpected output: $output"
    exit 1
fi

echo "=== DIND Build Integration Test PASSED ==="

#!/usr/bin/env bash
set -euo pipefail

# Integration test runner for TSK stack layers
#
# Runs stack integration tests using both Docker and Podman container engines,
# plus nested integration tests (podman-in-docker, podman-in-podman).
#
# When running inside a TSK container (TSK_CONTAINER=1):
#   - Docker engine tests are skipped (Docker unavailable)
#   - Nested tests are skipped (prevents infinite recursion)
#   - Podman engine stack tests still run

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TSK_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECTS_DIR="$TSK_ROOT/tests/integration/projects"
MANIFEST="$TSK_ROOT/Cargo.toml"
LOG_DIR="$TSK_ROOT/tests/integration/logs"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Create log directory for capturing test output
mkdir -p "$LOG_DIR"

# Global result tracking
total_passed=0
total_failed=0
all_failed=()

# Track temp dirs for cleanup
cleanup_dirs=()
cleanup() {
    for d in "${cleanup_dirs[@]}"; do
        rm -rf "$d"
    done
}
trap cleanup EXIT

# When running inside a TSK container with Podman, kill any stale API service
# to ensure a clean state. A stale Podman API service causes "conmon failed:
# signal: broken pipe" errors when starting containers. Build failures can
# crash the Podman API service; TSK will automatically start a new one on the
# next API call. Note: with parallel execution, a mid-run crash may affect
# other running tasks.
reset_podman_service() {
    if [ "${TSK_CONTAINER:-}" = "1" ]; then
        pkill -f "podman system service" 2>/dev/null || true
        # Wait for process to fully terminate and release the socket
        sleep 2
        rm -f /tmp/podman-run-$(id -u)/podman/podman.sock 2>/dev/null || true
    fi
}

# Creates isolated TSK data/runtime/config dirs and exports the env vars.
# Each test phase gets its own isolation to prevent task DB collisions.
# Note: exports must happen in the caller's scope, not inside $(), so
# this function echoes the dir path and the caller sets the env vars.
# Outputs: the base temp dir path (add to cleanup_dirs)
create_tsk_isolation() {
    local iso_dir
    iso_dir=$(mktemp -d)
    mkdir -p "$iso_dir/data" "$iso_dir/runtime" "$iso_dir/config"
    echo "$iso_dir"
}

# Runs all stack integration tests for a given container engine.
# Queues tasks via `tsk add`, runs them via `tsk server start`, checks results.
# Updates global counters: total_passed, total_failed, all_failed
# Arguments:
#   $1 - engine: "docker" or "podman"
run_stack_tests() {
    local engine="$1"

    reset_podman_service

    local iso_dir
    iso_dir=$(create_tsk_isolation)
    cleanup_dirs+=("$iso_dir")
    export TSK_DATA_HOME="$iso_dir/data"
    export TSK_RUNTIME_DIR="$iso_dir/runtime"
    export TSK_CONFIG_HOME="$iso_dir/config"

    # Collect stack names and their work directories
    local stacks=()
    declare -A stack_work_dirs
    declare -A stack_project_dirs

    # Queue all tasks via tsk add
    echo "--------------------------------------------"
    echo "  Queueing integration test tasks (${engine})"
    echo "--------------------------------------------"
    for project_dir in "$PROJECTS_DIR"/*/; do
        local stack
        stack=$(basename "$project_dir")
        stacks+=("$stack")
        stack_project_dirs[$stack]="$project_dir"

        # Create a temporary git repo with the project files
        local work_dir
        work_dir=$(mktemp -d)
        cleanup_dirs+=("$work_dir")
        stack_work_dirs[$stack]="$work_dir"

        git init -q -b main "$work_dir"
        git -C "$work_dir" config user.email "integ-test@tsk.dev"
        git -C "$work_dir" config user.name "TSK Integration Test"

        # Copy project files (including hidden files, excluding . and ..)
        cp -r "$project_dir". "$work_dir/"
        git -C "$work_dir" add -A
        git -C "$work_dir" commit -q -m "Initial commit for $stack integration test"

        echo -e "  Queueing: ${YELLOW}${stack}${NC}"
        if ! cargo run --manifest-path "$MANIFEST" -- add \
            --agent integ \
            --stack "$stack" \
            --name "integ-$stack" \
            --type feat \
            --prompt "Integration test for $stack stack" \
            --repo "$work_dir" \
            < /dev/null 2>&1 | tee "$LOG_DIR/${stack}-${engine}-add.log"; then
            echo -e "  ${RED}ERROR${NC}: Failed to queue $stack"
            exit 1
        fi
    done
    echo ""

    # Verify all tasks are queued
    echo "--------------------------------------------"
    echo "  Verifying queued tasks (${engine})"
    echo "--------------------------------------------"
    local list_before
    list_before=$(cargo run --manifest-path "$MANIFEST" -- list < /dev/null 2>&1)
    echo "$list_before" | tee "$LOG_DIR/list-before-${engine}.log"
    local queued_count expected_count
    queued_count=$(echo "$list_before" | grep -c "QUEUED" || true)
    expected_count="${#stacks[@]}"
    if [ "$queued_count" -ne "$expected_count" ]; then
        echo -e "${RED}ERROR${NC}: Expected $expected_count queued tasks, found $queued_count"
        exit 1
    fi
    echo -e "  ${GREEN}All $expected_count tasks queued${NC}"
    echo ""

    # Run all tasks in parallel via tsk server
    echo "--------------------------------------------"
    echo "  Starting server (${engine}, workers=4, quit-when-done)"
    echo "--------------------------------------------"
    cargo run --manifest-path "$MANIFEST" -- server start -q -w 4 \
        --container-engine "$engine" \
        < /dev/null 2>&1 | tee "$LOG_DIR/server-${engine}.log"
    echo ""

    # Check results
    echo "--------------------------------------------"
    echo "  Checking results (${engine})"
    echo "--------------------------------------------"
    local list_output
    list_output=$(cargo run --manifest-path "$MANIFEST" -- list < /dev/null 2>&1)
    echo "$list_output" | tee "$LOG_DIR/list-after-${engine}.log"
    echo ""

    local passed=0
    local failed=0
    local failed_stacks=()

    for stack in "${stacks[@]}"; do
        # Look for a line containing integ-<stack> and check its status.
        # Trailing space prevents partial matches (e.g., integ-go matching integ-golang).
        local task_line
        task_line=$(echo "$list_output" | grep "integ-${stack} " || true)
        if [ -z "$task_line" ]; then
            echo -e "${RED}FAILED${NC}: $stack ($engine) (task not found in list output)"
            failed=$((failed + 1))
            failed_stacks+=("$stack")
        elif echo "$task_line" | grep -q "COMPLETE"; then
            echo -e "${GREEN}PASSED${NC}: $stack ($engine)"
            passed=$((passed + 1))
        else
            echo -e "${RED}FAILED${NC}: $stack ($engine)"
            failed=$((failed + 1))
            failed_stacks+=("$stack")
        fi
    done
    echo ""

    # Run verification scripts on task branches
    echo "--------------------------------------------"
    echo "  Verifying task branches (${engine})"
    echo "--------------------------------------------"
    for stack in "${stacks[@]}"; do
        local project_dir="${stack_project_dirs[$stack]}"
        local verify_script="$project_dir/tsk-integ-verify.sh"

        # Skip stacks without a verify script
        if [ ! -f "$verify_script" ]; then
            continue
        fi

        # Skip stacks that already failed
        local task_line
        task_line=$(echo "$list_output" | grep "integ-${stack} " || true)
        if ! echo "$task_line" | grep -q "COMPLETE"; then
            continue
        fi

        local work_dir="${stack_work_dirs[$stack]}"

        # Find the task branch (pattern: tsk/feat/integ-<stack>/*)
        local branch
        branch=$(git -C "$work_dir" branch --list "tsk/feat/integ-${stack}/*" | head -1 | sed 's/^[* ]*//' || true)
        if [ -z "$branch" ]; then
            echo -e "  ${RED}VERIFY FAILED${NC}: $stack ($engine) (no task branch found)"
            failed=$((failed + 1))
            failed_stacks+=("$stack (verify: no branch)")
            passed=$((passed - 1))
            continue
        fi

        echo -e "  Verifying: ${YELLOW}${stack}${NC} (branch: $branch)"
        git -C "$work_dir" checkout -q "$branch"

        if (cd "$work_dir" && bash "$verify_script") 2>&1 | tee "$LOG_DIR/${stack}-${engine}-verify.log"; then
            echo -e "  ${GREEN}VERIFIED${NC}: $stack ($engine)"
        else
            echo -e "  ${RED}VERIFY FAILED${NC}: $stack ($engine)"
            failed=$((failed + 1))
            failed_stacks+=("$stack (verify)")
            passed=$((passed - 1))
        fi

        git -C "$work_dir" checkout -q main
    done
    echo ""

    # Accumulate into global counters
    total_passed=$((total_passed + passed))
    total_failed=$((total_failed + failed))
    for s in "${failed_stacks[@]:-}"; do
        if [ -n "$s" ]; then
            all_failed+=("$s ($engine)")
        fi
    done
}

# Runs a nested integration test for the given container engine.
# Starts a TSK container with the tsk repo and runs `just integration-test`
# inside it, testing podman-in-docker or podman-in-podman.
# Arguments:
#   $1 - engine: "docker" or "podman"
run_nested_test() {
    local engine="$1"

    reset_podman_service

    local iso_dir
    iso_dir=$(create_tsk_isolation)
    cleanup_dirs+=("$iso_dir")
    export TSK_DATA_HOME="$iso_dir/data"
    export TSK_RUNTIME_DIR="$iso_dir/runtime"
    export TSK_CONFIG_HOME="$iso_dir/config"

    # The nested container builds all 7 stack images from scratch in parallel.
    # The default 12 GB memory limit is too low — tmpfs-backed Podman storage
    # alone can exceed 10 GB during concurrent java + rust image builds,
    # triggering OOM kills. 30 GB provides sufficient headroom.
    mkdir -p "$iso_dir/config/tsk"
    cat > "$iso_dir/config/tsk/tsk.toml" << EOF
[defaults]
memory_gb = 30.0

[project.tsk]
volumes = [
    { name = "integ-podman-storage-${engine}", container = "/home/agent/.local/share/containers/storage" },
]
EOF

    echo -e "  Running: ${YELLOW}nested-${engine}${NC}"

    if cargo run --manifest-path "$MANIFEST" -- run \
        --agent integ \
        --dind \
        --container-engine "$engine" \
        --stack rust \
        --name "nested-${engine}" \
        --type feat \
        --prompt "Nested integration test for ${engine}" \
        --repo "$TSK_ROOT" \
        < /dev/null 2>&1 | tee "$LOG_DIR/nested-${engine}.log"; then
        echo -e "  ${GREEN}PASSED${NC}: nested-${engine}"
        total_passed=$((total_passed + 1))
    else
        echo -e "  ${RED}FAILED${NC}: nested-${engine}"
        total_failed=$((total_failed + 1))
        all_failed+=("nested-${engine}")
    fi
    echo ""
}

# ==========================================================================
#  Main
# ==========================================================================

echo "============================================"
echo "  TSK Stack Layer Integration Tests"
echo "============================================"
echo ""

# Build tsk first
echo "Building tsk..."
if ! cargo build --manifest-path "$MANIFEST" 2>&1; then
    echo "ERROR: Failed to build tsk"
    exit 1
fi
echo ""

# --- Nested integration tests (skipped inside TSK containers) ---
if [ "${TSK_CONTAINER:-}" != "1" ]; then
    echo "============================================"
    echo "  Nested Integration Tests"
    echo "============================================"
    echo ""

    echo "--------------------------------------------"
    echo "  podman-in-docker"
    echo "--------------------------------------------"
    run_nested_test "docker"

    echo "--------------------------------------------"
    echo "  podman-in-podman"
    echo "--------------------------------------------"
    run_nested_test "podman"
else
    echo "Skipping nested tests (inside TSK container)"
    echo ""
fi

# --- Docker engine tests (skipped inside TSK containers) ---
if [ "${TSK_CONTAINER:-}" != "1" ]; then
    echo "============================================"
    echo "  Docker Engine Stack Tests"
    echo "============================================"
    echo ""
    run_stack_tests "docker"
else
    echo "Skipping Docker engine tests (inside TSK container)"
    echo ""
fi

# --- Podman engine stack tests (always run) ---
echo "============================================"
echo "  Podman Engine Stack Tests"
echo "============================================"
echo ""
run_stack_tests "podman"

# --- Network isolation tests (only inside TSK containers) ---
if [ "${TSK_CONTAINER:-}" = "1" ]; then
    echo "============================================"
    echo "  Network Isolation Tests"
    echo "============================================"
    echo ""
    if "$SCRIPT_DIR/network-isolation-test.sh" 2>&1 | tee "$LOG_DIR/network-isolation.log"; then
        echo -e "  ${GREEN}PASSED${NC}: network-isolation"
        total_passed=$((total_passed + 1))
    else
        echo -e "  ${RED}FAILED${NC}: network-isolation"
        total_failed=$((total_failed + 1))
        all_failed+=("network-isolation")
    fi
    echo ""
else
    echo "Skipping network isolation tests (not in TSK container)"
    echo ""
fi

# --- Summary ---
echo "============================================"
echo "  Integration Test Summary"
echo "============================================"
echo -e "  ${GREEN}Passed${NC}: $total_passed"
echo -e "  ${RED}Failed${NC}: $total_failed"
if [ ${#all_failed[@]} -gt 0 ]; then
    echo ""
    echo "  Failed:"
    for s in "${all_failed[@]}"; do
        echo -e "    - ${RED}${s}${NC}"
    done
fi
echo "============================================"

if [ "$total_failed" -gt 0 ]; then
    exit 1
fi

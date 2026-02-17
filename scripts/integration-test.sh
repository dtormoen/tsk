#!/usr/bin/env bash
set -euo pipefail

# Integration test runner for TSK stack layers
# Queues all stack tests via `tsk add`, then runs them in parallel via `tsk server start -q -w 4`

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

# TSK isolation: use TSK-specific env vars to isolate data, runtime, and config
# directories without affecting other XDG-aware software (e.g., Podman).
# Config is also isolated so user-level dockerfiles/templates don't interfere
# with tests that should exercise the built-in defaults.
TEST_TSK_DIR=$(mktemp -d)
export TSK_DATA_HOME="$TEST_TSK_DIR/data"
export TSK_RUNTIME_DIR="$TEST_TSK_DIR/runtime"
export TSK_CONFIG_HOME="$TEST_TSK_DIR/config"
mkdir -p "$TSK_DATA_HOME" "$TSK_RUNTIME_DIR" "$TSK_CONFIG_HOME"

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

# Track temp dirs for cleanup
cleanup_dirs=("$TEST_TSK_DIR")
cleanup() {
    for d in "${cleanup_dirs[@]}"; do
        rm -rf "$d"
    done
}
trap cleanup EXIT

reset_podman_service

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

# Collect stack names and their work directories for later result checking
stacks=()
declare -A stack_work_dirs
declare -A stack_project_dirs

# Phase 1: Queue all tasks via tsk add
echo "--------------------------------------------"
echo "  Queueing integration test tasks"
echo "--------------------------------------------"
for project_dir in "$PROJECTS_DIR"/*/; do
    stack=$(basename "$project_dir")
    stacks+=("$stack")
    stack_project_dirs[$stack]="$project_dir"

    # Create a temporary git repo with the project files
    WORK_DIR=$(mktemp -d)
    cleanup_dirs+=("$WORK_DIR")
    stack_work_dirs[$stack]="$WORK_DIR"

    git init -q -b main "$WORK_DIR"
    git -C "$WORK_DIR" config user.email "integ-test@tsk.dev"
    git -C "$WORK_DIR" config user.name "TSK Integration Test"

    # Copy project files (including hidden files, excluding . and ..)
    cp -r "$project_dir". "$WORK_DIR/"
    git -C "$WORK_DIR" add -A
    git -C "$WORK_DIR" commit -q -m "Initial commit for $stack integration test"

    echo -e "  Queueing: ${YELLOW}${stack}${NC}"
    if ! cargo run --manifest-path "$MANIFEST" -- add \
        --agent integ \
        --stack "$stack" \
        --name "integ-$stack" \
        --type feat \
        --description "Integration test for $stack stack" \
        --repo "$WORK_DIR" \
        < /dev/null 2>&1 | tee "$LOG_DIR/${stack}-add.log"; then
        echo -e "  ${RED}ERROR${NC}: Failed to queue $stack"
        exit 1
    fi
done
echo ""

# Phase 2: Verify all tasks are queued
echo "--------------------------------------------"
echo "  Verifying queued tasks"
echo "--------------------------------------------"
LIST_BEFORE=$(cargo run --manifest-path "$MANIFEST" -- list < /dev/null 2>&1)
echo "$LIST_BEFORE" | tee "$LOG_DIR/list-before.log"
QUEUED_COUNT=$(echo "$LIST_BEFORE" | grep -c "QUEUED" || true)
EXPECTED_COUNT="${#stacks[@]}"
if [ "$QUEUED_COUNT" -ne "$EXPECTED_COUNT" ]; then
    echo -e "${RED}ERROR${NC}: Expected $EXPECTED_COUNT queued tasks, found $QUEUED_COUNT"
    exit 1
fi
echo -e "  ${GREEN}All $EXPECTED_COUNT tasks queued${NC}"
echo ""

# Phase 3: Run all tasks in parallel via tsk server
echo "--------------------------------------------"
echo "  Starting server (workers=4, quit-when-done)"
echo "--------------------------------------------"
cargo run --manifest-path "$MANIFEST" -- server start -q -w 4 \
    < /dev/null 2>&1 | tee "$LOG_DIR/server.log"
echo ""

# Phase 4: Check results
echo "--------------------------------------------"
echo "  Checking results"
echo "--------------------------------------------"
LIST_OUTPUT=$(cargo run --manifest-path "$MANIFEST" -- list < /dev/null 2>&1)
echo "$LIST_OUTPUT" | tee "$LOG_DIR/list-after.log"
echo ""

# Parse results per stack
passed=0
failed=0
failed_stacks=()

for stack in "${stacks[@]}"; do
    # Look for a line containing integ-<stack> and check its status.
    # The tsk list output has columns: ID, Name, Type, Status, ...
    # Trailing space prevents partial matches (e.g., integ-go matching integ-golang).
    task_line=$(echo "$LIST_OUTPUT" | grep "integ-${stack} " || true)
    if [ -z "$task_line" ]; then
        echo -e "${RED}FAILED${NC}: $stack (task not found in list output)"
        failed=$((failed + 1))
        failed_stacks+=("$stack")
    elif echo "$task_line" | grep -q "COMPLETE"; then
        echo -e "${GREEN}PASSED${NC}: $stack"
        passed=$((passed + 1))
    else
        echo -e "${RED}FAILED${NC}: $stack"
        failed=$((failed + 1))
        failed_stacks+=("$stack")
    fi
done
echo ""

# Phase 5: Run verification scripts on task branches
# For completed tasks that have a tsk-integ-verify.sh, check out the task branch
# and run the verification script to validate the final state of the repository.
echo "--------------------------------------------"
echo "  Verifying task branches"
echo "--------------------------------------------"
for stack in "${stacks[@]}"; do
    project_dir="${stack_project_dirs[$stack]}"
    verify_script="$project_dir/tsk-integ-verify.sh"

    # Skip stacks without a verify script
    if [ ! -f "$verify_script" ]; then
        continue
    fi

    # Skip stacks that already failed
    task_line=$(echo "$LIST_OUTPUT" | grep "integ-${stack} " || true)
    if ! echo "$task_line" | grep -q "COMPLETE"; then
        continue
    fi

    work_dir="${stack_work_dirs[$stack]}"

    # Find the task branch (pattern: tsk/feat/integ-<stack>/*)
    branch=$(git -C "$work_dir" branch --list "tsk/feat/integ-${stack}/*" | head -1 | sed 's/^[* ]*//' || true)
    if [ -z "$branch" ]; then
        echo -e "  ${RED}VERIFY FAILED${NC}: $stack (no task branch found)"
        failed=$((failed + 1))
        failed_stacks+=("$stack (verify: no branch)")
        # Undo the pass count from phase 4
        passed=$((passed - 1))
        continue
    fi

    echo -e "  Verifying: ${YELLOW}${stack}${NC} (branch: $branch)"
    git -C "$work_dir" checkout -q "$branch"

    if (cd "$work_dir" && bash "$verify_script") 2>&1 | tee "$LOG_DIR/${stack}-verify.log"; then
        echo -e "  ${GREEN}VERIFIED${NC}: $stack"
    else
        echo -e "  ${RED}VERIFY FAILED${NC}: $stack"
        failed=$((failed + 1))
        failed_stacks+=("$stack (verify)")
        # Undo the pass count from phase 4
        passed=$((passed - 1))
    fi

    # Return to main branch
    git -C "$work_dir" checkout -q main
done
echo ""

# Summary
echo "============================================"
echo "  Integration Test Summary"
echo "============================================"
echo -e "  ${GREEN}Passed${NC}: $passed"
echo -e "  ${RED}Failed${NC}: $failed"
if [ ${#failed_stacks[@]} -gt 0 ]; then
    echo ""
    echo "  Failed stacks:"
    for s in "${failed_stacks[@]}"; do
        echo -e "    - ${RED}${s}${NC}"
    done
fi
echo "============================================"

if [ "$failed" -gt 0 ]; then
    exit 1
fi

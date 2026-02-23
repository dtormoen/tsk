# Install cargo-binstall and dev tools (cargo-edit for `cargo upgrade`, cargo-outdated)
setup:
    cargo install cargo-binstall
    cargo binstall cargo-edit cargo-outdated -y

# Build and install the tsk binary from the local workspace
install:
    cargo install --path . --locked --force

# Run the test suite in quiet mode
test:
    cargo test -q

# Auto-format all Rust source files
format:
    cargo fmt

# Run clippy lints on all targets, treating warnings as errors
lint:
    cargo clippy --all-targets -- -D warnings
    cargo clippy -- -D warnings

# Format, lint, test, verify the binary starts, and run network isolation tests in TSK containers (run before committing)
precommit: format lint test
    # Check that the command starts
    cargo run -- --help > /dev/null
    # Run network isolation tests if inside a TSK container
    if [ "${TSK_CONTAINER:-}" = "1" ]; then \
        echo "Running integration tests"; \
        just integration-test; \
    else \
        echo "Skipping network isolation tests (not in TSK container)"; \
    fi

# Upgrade all dependencies in cargo.toml
upgrade-deps:
    cargo upgrade

# Update all dependencies in cargo.lock
update-deps:
    cargo update

# Run network isolation tests (must be run inside a TSK container)
network-isolation-test:
    ./scripts/network-isolation-test.sh

# Smoke test for Podman engine support
podman-smoke-test:
    #!/usr/bin/env bash
    set -euo pipefail
    MANIFEST="$(pwd)/Cargo.toml"
    WORK_DIR=$(mktemp -d)
    trap 'rm -rf "$WORK_DIR"' EXIT
    cd "$WORK_DIR"
    git init
    git commit --allow-empty -m "init"
    cargo run --manifest-path "$MANIFEST" -- run --container-engine podman --agent no-op --type feat --name podman-test --stack default --description "Podman smoke test"

# Run integration tests for all stack layers
integration-test:
    ./scripts/integration-test.sh

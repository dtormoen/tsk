setup:
    cargo install cargo-binstall
    cargo binstall cargo-edit cargo-outdated -y

install:
    cargo install --path . --locked --force

test:
    cargo test -q

format:
    cargo fmt

lint:
    cargo clippy --all-targets -- -D warnings
    cargo clippy -- -D warnings

precommit: format lint test
    # Check that the command starts
    cargo run -- --help > /dev/null

# Upgrade all dependencies in cargo.toml
upgrade-deps:
    cargo upgrade

# Update all dependencies in cargo.lock
update-deps:
    cargo update

# Run network isolation tests (must be run inside a TSK container)
network-isolation-test:
    ./scripts/network-isolation-test.sh

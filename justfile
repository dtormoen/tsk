build:
    cargo build

install:
    cargo install --path . --locked --force

test:
    cargo test -q

format:
    cargo fmt
    cargo clippy --all-targets --fix --allow-dirty -- -D warnings

precommit:
    cargo fmt
    cargo clippy --all-targets --fix --allow-dirty -- -D warnings
    cargo test -q
    # Check that the command starts
    cargo run -- --help > /dev/null

build:
    cargo build

install:
    cargo install --path . --locked --force

test:
    cargo test -q

format:
    cargo clippy --all-targets --fix --allow-dirty -- -D warnings
    cargo fmt

precommit:
    cargo clippy --all-targets --fix --allow-dirty -- -D warnings
    cargo fmt
    cargo test -q
    # Check that the command starts
    cargo run -- --help > /dev/null

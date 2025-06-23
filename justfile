build:
    cargo build

install:
    cargo install --path .

test:
    cargo test

format:
    cargo fmt
    cargo clippy -- -D warnings

precommit:
    cargo fmt
    cargo clippy -- -D warnings
    cargo test
    # Check that the command starts
    cargo run -- --help

docker-build:
    cargo run -- docker-build --no-cache

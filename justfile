build:
    cargo build

install:
    cargo install --path .

test:
    cargo test -- --test-threads=1

format:
    cargo fmt
    cargo clippy

precommit:
    cargo fmt
    cargo clippy -- -D warnings
    cargo test
    # Check that the command will starts
    cargo run -- --help

docker-build:
    cargo run -- docker-build --no-cache


build:
    cargo build

test:
    cargo test

format:
    cargo fmt
    cargo clippy

precommit:
    cargo fmt
    cargo clippy
    cargo test


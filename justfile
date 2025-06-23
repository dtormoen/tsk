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
    cargo run -- docker-build

nuke-tsk:
    rm -rf .tsk
    git for-each-ref --format="%(refname:short)" refs/heads/tsk/\* | xargs git branch -D

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
    # Check that the command will starts
    cargo run -- --help

docker-build:
    cd dockerfiles/tsk-base && docker build -t tsk/base .
    cd dockerfiles/tsk-proxy && docker build -t tsk/proxy .

nuke-tsk:
    rm -rf .tsk
    git for-each-ref --format="%(refname:short)" refs/heads/tsk/\* | xargs git branch -D

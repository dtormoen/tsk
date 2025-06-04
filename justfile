build:
    cargo build

test:
    cargo test -- --test-threads=1

format:
    cargo fmt
    cargo clippy

precommit:
    cargo fmt
    cargo clippy -- -D warnings
    cargo test -- --test-threads=1
    # Check that the command will starts
    cargo run -- --help

docker-build:
    cd dockerfiles/tsk-base && docker build \
        --build-arg GIT_USER_NAME="$(git config --global user.name)" \
        --build-arg GIT_USER_EMAIL="$(git config --global user.email)" \
        -t tsk/base .
    cd dockerfiles/tsk-proxy && docker build -t tsk/proxy .

nuke-tsk:
    rm -rf .tsk
    git for-each-ref --format="%(refname:short)" refs/heads/tsk/\* | xargs git branch -D

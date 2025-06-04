# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

TSK is a Rust-based CLI tool for delegating development tasks to AI agents running in sandboxed Docker environments. The project enables a "lead engineer + AI team" workflow where tasks are executed autonomously in isolated containers and produce reviewable git branches.

## Development Commands

```bash
# Build and development
cargo build
cargo test -- --test-threads=1    # Tests must run serially
RUST_LOG=debug cargo run

# Code quality
cargo fmt -- --check
cargo clippy -- -D warnings

# Justfile commands (recommended)
just build                         # Build project
just test                          # Run tests with proper threading
just format                        # Format and lint code
just precommit                     # Full CI checks (fmt, clippy, test, help)
```

## Architecture Overview

TSK implements a command pattern with dependency injection for testability. The core workflow: queue tasks → execute in Docker → create git branches for review.

### Key Components

**CLI Commands** (`src/commands/`)
- `add`: Queue tasks with descriptions and templates
- `run`: Execute all queued tasks in parallel Docker containers
- `quick`: Immediately execute single tasks
- `list`: Display task status and results
- `debug`: Launch interactive containers for troubleshooting
- `tasks`: Manage task queue (delete/clean operations)

**Task Management** (`src/task.rs`, `src/task_manager.rs`)
- JSON-based persistence in `.tsk/tasks.json`
- Task status: Queued → Running → Complete/Failed
- Branch naming: `tsk/{timestamp}-{task-name}`

**Docker Integration** (`src/docker.rs`)
- Security-first containers with dropped capabilities
- Network isolation via proxy (Squid) for API-only access
- Resource limits: 2GB memory, 1 CPU core
- Volume mounting for repository copies and Claude config

**Git Operations** (`src/git.rs`)
- Repository copying with `.tsk` exclusion
- Isolated branch creation and result integration
- Automatic commit and fetch operations

**Dependency Injection** (`src/context/`)
- `AppContext` provides centralized resource management with builder pattern
- Trait abstractions enable comprehensive testing with mocks
- Factory pattern prevents accidental operations in tests
- `FileSystemOperations` trait abstracts all file system operations for testability

### Branch and Task Conventions

Tasks create timestamped branches: `tsk/2024-06-01-1430-add-auth-feature`
Template-based task descriptions encourage structured problem statements.

### Docker Infrastructure

**Base Image** (`dockerfiles/tsk-base/`): Ubuntu 22.04 with Claude Code, Rust toolchain, Node.js
**Proxy Image** (`dockerfiles/tsk-proxy/`): Squid proxy for controlled network access
Git configuration inherited via Docker build args from host user.

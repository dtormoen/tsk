# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

TSK is a Rust-based CLI tool for delegating development tasks to AI agents running in sandboxed Docker environments. The project enables a "lead engineer + AI team" workflow where tasks are executed autonomously in isolated containers and produce reviewable git branches.

## Development Commands

```bash
# Build and development
cargo build
cargo test
RUST_LOG=debug cargo run

# Code quality
cargo fmt -- --check
cargo clippy -- -D warnings

# Justfile commands (recommended)
just build                         # Build project
just install                       # Install TSK binary to system
just test                          # Run tests with proper threading
just format                        # Format and lint code
just precommit                     # Full CI checks (fmt, clippy, test, help)
just docker-build                  # Build Docker images using embedded assets
```

## Architecture Overview

TSK implements a command pattern with dependency injection for testability. The core workflow: queue tasks → execute in Docker → create git branches for review. TSK can run in server mode for continuous task processing across multiple repositories.

### Key Components

**CLI Commands** (`src/commands/`)
- `add`: Queue tasks with descriptions and templates
- `run`: Execute all queued tasks (or start server with `--server` flag)
- `quick`: Immediately execute single tasks
- `list`: Display task status and results
- `debug`: Launch interactive containers for troubleshooting
- `tasks`: Manage task queue (delete/clean operations)
- `stop-server`: Stop the running TSK server

**Task Management** (`src/task.rs`, `src/task_storage.rs`, `src/task_manager.rs`)
- `TaskBuilder` provides consistent task creation with builder pattern
- `TaskStorage` trait abstracts storage with JSON-based implementation
- Centralized JSON persistence in XDG data directory (`$XDG_DATA_HOME/tsk/tasks.json`)
- Task status: Queued → Running → Complete/Failed
- Branch naming: `tsk/{task-id}` (where task-id is `{timestamp}-{task-type}-{task-name}`)

**Docker Integration** (`src/docker.rs`)
- Security-first containers with dropped capabilities
- Network isolation via proxy (Squid) for API-only access
- Resource limits: 4GB memory, 4 CPU cores
- Volume mounting for repository copies and Claude config

**Storage** (`src/storage/`)
- `XdgDirectories`: Manages XDG-compliant directory structure
- Centralized task storage across all repositories
- Runtime directory for server socket and PID file

**Server Mode** (`src/server/`, `src/client/`)
- `TskServer`: Continuous task execution daemon
- `TskClient`: Client for communicating with server
- Unix socket-based IPC protocol
- Sequential task execution (one at a time)

**Git Operations** (`src/git.rs`)
- Repository copying to centralized task directories
- Isolated branch creation and result integration
- Automatic commit and fetch operations

**Dependency Injection** (`src/context/`)
- `AppContext` provides centralized resource management with builder pattern
- Traits in the `AppContext` should be accessed via the `AppContext`
- Factory pattern prevents accidental operations in tests
- `FileSystemOperations` trait abstracts all file system operations for testability
- `GitOperations` trait abstracts all git operations for improved testability and separation of concerns
- `XdgDirectories` provides XDG-compliant directory paths for centralized storage

### Testing Conventions

- Avoid mocks, especially for traits that are not in the `AppContext`
- Avoid tests with side effects like modifying resources in the `AppContext` or changing directories
- Make tests thread safe so they can be run in parallel
- Keep tests simple and concise while still testing core functionality
- Avoid the use of `#[cfg(test)]` directives in code

### Branch and Task Conventions

Tasks create timestamped branches: `tsk/2024-06-01-1430-feat-add-auth`
Template-based task descriptions encourage structured problem statements.

### Docker Infrastructure

**Base Image** (`dockerfiles/tsk-base/`): Ubuntu 22.04 with Claude Code, Rust toolchain, Node.js
**Proxy Image** (`dockerfiles/tsk-proxy/`): Squid proxy for controlled network access
Git configuration inherited via Docker build args from host user.

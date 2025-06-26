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
- `debug`: Launch interactive containers for troubleshooting (uses unified task execution with no-op agent)
- `tasks`: Manage task queue (delete/clean operations)
- `templates`: Manage task type templates
- `docker-build`: Build required docker images
- `stop-server`: Stop the running TSK server
- `stop-proxy`: Stop the running TSK proxy

**Task Management** (`src/task.rs`, `src/task_storage.rs`, `src/task_manager.rs`)
- `TaskBuilder` provides consistent task creation with builder pattern
- Repository is copied at task creation time (including all tracked files, untracked non-ignored files, and dirty files), ensuring all tasks have a valid repository copy that matches `git status`
- `TaskStorage` trait abstracts storage with JSON-based implementation
- Centralized JSON persistence in XDG data directory (`$XDG_DATA_HOME/tsk/tasks.json`)
- Task status: Queued → Running → Complete/Failed
- Branch naming: `tsk/{task-id}` (where task-id is `{timestamp}-{task-type}-{task-name}`)

**Docker Integration** (`src/docker/`)
- `DockerImageManager`: Centralized Docker image management with intelligent layering
- Security-first containers with dropped capabilities
- Network isolation via proxy (Squid) for API-only access
- Volume mounting for repository copies and agent config
- Layered image system: base → tech-stack → agent → project
- Automatic fallback to default project layer when specific layer is missing

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
- Repository copying to centralized task directories (includes .tsk directory for Docker configurations)
- Isolated branch creation and result integration
- Automatic commit and fetch operations

**Dependency Injection** (`src/context/`)
- `AppContext` provides centralized resource management with builder pattern
- Traits in the `AppContext` should be accessed via the `AppContext`
- Factory pattern prevents accidental operations in tests
- `FileSystemOperations` trait abstracts all file system operations for testability
- `GitOperations` trait abstracts all git operations for improved testability and separation of concerns
- `RepositoryContext` trait provides auto-detection of tech stack and project name from repository files
- `XdgDirectories` provides XDG-compliant directory paths for centralized storage

**Auto-Detection** (`src/context/repository_context.rs`)
- Automatic detection of technology stack based on repository files:
  - Rust: `Cargo.toml` → "rust"
  - Python: `pyproject.toml`, `requirements.txt`, `setup.py` → "python"
  - Node.js: `package.json` → "node"
  - Go: `go.mod` → "go"
  - Java: `pom.xml`, `build.gradle`, `build.gradle.kts` → "java"
  - Default: "default" (when no specific files found)
- Automatic project name detection from repository directory name with cleaning for Docker compatibility
- Used by `TaskBuilder`, `DockerBuildCommand`, and `DebugCommand` when `--tech-stack` and `--project` flags are not provided
- Provides user feedback when auto-detection is used vs. explicit flags

### Development Conventions

- Avoid the use of `#[cfg(test)]` and `#[allow(dead_code)]` directives in code
- Always keep documentation up to date following rustdoc best practices
- Keep CLAUDE.md file simple, but up to date

### Testing Conventions

- Avoid mocks, especially for traits that are not in the `AppContext`
- Avoid tests with side effects like modifying resources in the `AppContext` or changing directories
- Make tests thread safe so they can be run in parallel
- Keep tests simple and concise while still testing core functionality
- Avoid the use of `#[cfg(test)]` and `#[allow(dead_code)]` directives in code

### Branch and Task Conventions

- Tasks create timestamped branches: `tsk/2024-06-01-1430-feat-add-auth`
- Template-based task descriptions encourage structured problem statements.

### Docker Infrastructure

- **Layered Images**: Four-layer system for flexible customization
  - Base layer: Ubuntu 22.04 base OS and common tools
  - Tech-stack layer: Language-specific toolchains (default, rust, python, etc.)
  - Agent layer: AI agent installations (claude, etc.)
  - Project layer: Project-specific dependencies (optional, falls back to default)
- **Custom Project Dockerfiles**: Place project-specific Dockerfiles in `.tsk/dockerfiles/project/{project-name}/Dockerfile`
- **Proxy Image** (`dockerfiles/tsk-proxy/`): Squid proxy for controlled network access
- Git configuration inherited via Docker build args from host user
- Automatic image rebuilding when missing during task execution

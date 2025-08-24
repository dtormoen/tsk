# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

TSK is a Rust-based CLI tool for delegating development tasks to AI agents running in sandboxed Docker environments. The project enables a "lead engineer + AI team" workflow where tasks are executed autonomously in isolated containers and produce reviewable git branches. TSK supports parallel task execution with configurable worker counts for improved throughput.

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
just docker-build                  # Build Docker images using embedded assets (wraps 'tsk docker build')
```

## Architecture Overview

TSK implements a command pattern with dependency injection for testability. The core workflow: queue tasks → execute in Docker → create git branches for review. TSK can run in server mode for continuous task processing across multiple repositories.

### Key Components

**CLI Commands** (`src/commands/`)

*Task Commands (implicit "task" noun):*
- `add`: Queue tasks with descriptions and templates
- `run`: Execute all queued tasks
- `quick`: Immediately execute single tasks
- `list`: Display task status and results
- `debug`: Launch interactive containers for troubleshooting (accepts same arguments as `quick` but always runs interactively)
- `clean`: Delete all completed tasks
- `delete <task-id>`: Delete a specific task
- `retry <task-id>`: Retry a previous task

*Subcommand Groups:*
- `server run`: Start the TSK server daemon
- `server stop`: Stop the running TSK server
- `docker build`: Build required docker images
- `proxy stop`: Stop the running TSK proxy
- `template list`: List available task type templates

**Task Management** (`src/task.rs`, `src/task_storage.rs`, `src/task_manager.rs`)
- `TaskBuilder` provides consistent task creation with builder pattern
- Repository is copied at task creation time (including all tracked files, untracked non-ignored files, and dirty files), ensuring all tasks have a valid repository copy that matches `git status`
- `TaskStorage` trait abstracts storage with JSON-based implementation
  - Thread-safe with mutex locking for file access
  - Optimized `update_task_status` method for atomic status updates
- Centralized JSON persistence in XDG data directory (`$XDG_DATA_HOME/tsk/tasks.json`)
- Task status: Queued → Running → Complete/Failed
- Branch naming: `tsk/{task-type}/{task-name}/{task-id}` (human-readable format with task type, sanitized task name, and 8-character unique identifier)

**Docker Integration** (`src/docker/`)
- `DockerImageManager`: Centralized Docker image management with intelligent layering
- Security-first containers with dropped capabilities
- Network isolation via proxy (Squid) for API-only access
- Volume mounting for repository copies and agent config
- Layered image system: base → tech-stack → agent → project
- Automatic fallback to default project layer when specific layer is missing

**Storage** (`src/storage/`)
- `TskConfig`: Manages TSK configuration and XDG-compliant directory structure
- Centralized task storage across all repositories
- Runtime directory for server socket and PID file

**Server Mode** (`src/server/`, `src/client/`)
- `TskServer`: Continuous task execution daemon
- `TskClient`: Client for communicating with server
- Unix socket-based IPC protocol
- Parallel task execution with configurable workers (default: 1)
- `TaskExecutor`: Manages parallel execution with semaphore-based worker pool
  - Automatic retry for agent warmup failures with 1-hour wait period
  - Tasks that fail during warmup are reset to queued status and retried after wait

**Git Operations** (`src/git.rs`, `src/git_sync.rs`)
- Repository copying to centralized task directories (includes .tsk directory for Docker configurations)
- Isolated branch creation and result integration
- Automatic commit and fetch operations
- `GitSyncManager`: Repository-level synchronization for concurrent git operations
  - Prevents concurrent fetch operations to the same repository
  - Uses repository path-based locking mechanism

**Dependency Injection** (`src/context/`)
- `AppContext` provides centralized resource management with builder pattern
- Traits in the `AppContext` should be accessed via the `AppContext`
- Factory pattern prevents accidental operations in tests
- `Config` struct centralizes environment variable access for thread-safe testing
  - Provides overrides for `HOME/.claude`, `EDITOR`, and `TERM` environment variables
  - Eliminates unsafe `env::set_var` operations in tests
  - Accessed via `AppContext::config()` method
  - Passed to agents via `AgentProvider::get_agent()` for proper configuration
- `FileSystemOperations` trait abstracts all file system operations for testability
- `GitOperations` trait abstracts all git operations for improved testability and separation of concerns
- `TskConfig` provides TSK configuration and XDG-compliant directory paths for centralized storage

**Agents** (`src/agent/`)
- `Agent` trait defines the interface for AI agents that execute tasks
  - `build_command()`: Returns the command to execute for normal task processing
  - `build_interactive_command()`: Returns the command for interactive debugging sessions (shows instructions and normal command, then provides shell)
  - `validate()`: Checks agent configuration (e.g., Claude credentials)
  - `warmup()`: Performs pre-execution setup (e.g., refreshing OAuth tokens)
- Available agents:
  - `claude-code`: Claude Code AI agent (default)
  - `no-op`: Simple agent for testing that displays instructions
- Interactive debugging uses `build_interactive_command()` to show what would run normally before providing interactive access

**Auto-Detection** (`src/repository.rs`)
- Automatic detection of technology stack based on repository files:
  - Rust: `Cargo.toml` → "rust"
  - Python: `pyproject.toml`, `requirements.txt`, `setup.py` → "python"
  - Node.js: `package.json` → "node"
  - Go: `go.mod` → "go"
  - Java: `pom.xml`, `build.gradle`, `build.gradle.kts` → "java"
  - Lua: `rockspec`, `.luacheckrc`, `init.lua` → "lua"
  - Default: "default" (when no specific files found)
- Automatic project name detection from repository directory name with cleaning for Docker compatibility
- Used by `TaskBuilder`, `DockerBuildCommand`, and `DebugCommand` when `--tech-stack` and `--project` flags are not provided
- Provides user feedback when auto-detection is used vs. explicit flags

### Development Conventions

- Avoid `#[allow(dead_code)]` directives
- Always keep documentation up to date following rustdoc best practices
- Keep CLAUDE.md file simple, but up to date
- Avoid `unsafe` blocks

### Testing Conventions

- Prefer integration tests using real implementations over mocks
- Use `TestGitRepository` from `test_utils::git_test_utils` for tests requiring git repositories
- Tests should use temporary directories that are automatically cleaned up
- Make tests thread safe so they can be run in parallel
- Always use `AppContext::builder()` for test setup rather than creating objects contained in the `AppContext` directly
  - Exception: Tests in `src/context/*` that are directly testing XdgConfig or TskConfig functionality
  - `AppContext::builder().build()` automatically sets up test-safe temporary directories and configurations
- Keep tests simple and concise while still testing core functionality. Improving existing tests is always prefered over adding new ones
- Avoid using `#[allow(dead_code)]`
- Limit `#[cfg(test)]` to tests and test utilities

### Branch and Task Conventions

- Tasks create branches with human-readable names: `tsk/{task-type}/{task-name}/{task-id}` (e.g., `tsk/feat/add-user-auth/a1b2c3d4`)
- Template-based task descriptions encourage structured problem statements.

### Docker Infrastructure

- **Layered Images**: Four-layer system for flexible customization
  - Base layer: Ubuntu 24.04 base OS and common tools
  - Tech-stack layer: Language-specific toolchains (default, rust, python, node, go, java, lua)
  - Agent layer: AI agent installations (claude, etc.)
  - Project layer: Project-specific dependencies (optional, falls back to default)
- **Custom Project Dockerfiles**: Place project-specific Dockerfiles in `.tsk/dockerfiles/project/{project-name}/Dockerfile`
- **Proxy Image** (`dockerfiles/tsk-proxy/`): Squid proxy for controlled network access
- Git configuration inherited via Docker build args from host user
- Automatic image rebuilding when missing during task execution

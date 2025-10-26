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
- `run`: Immediately execute single tasks (supports piped input via stdin for descriptions)
- `shell`: Launch sandbox container with agent for interactive use (supports piped input via stdin for descriptions)
- `add`: Queue tasks with descriptions and templates (supports piped input via stdin for descriptions)
- `list`: Display task status and results
- `clean`: Delete all completed tasks
- `delete <task-id>`: Delete a specific task
- `retry <task-id>`: Retry a previous task

*Subcommand Groups:*
- `server start`: Start the TSK server daemon
- `server stop`: Stop the running TSK server
- `docker build`: Build required docker images
- `proxy stop`: Stop the running TSK proxy
- `template list`: List available task type templates

**Task Management** (`src/task.rs`, `src/task_storage.rs`, `src/task_manager.rs`)
- `TaskBuilder` provides consistent task creation with builder pattern
- Repository is copied at task creation time, capturing the complete working directory state including:
  - All tracked files with their current working directory content (including unstaged changes)
  - All staged files (newly added files in the index)
  - All untracked non-ignored files
  - The `.git` directory for full repository state
  - The `.tsk` directory for project-specific configurations
  - This ensures all tasks have a valid repository copy that exactly matches what `git status` shows
- `TaskStorage` trait abstracts storage with JSON-based implementation
  - Thread-safe with mutex locking for file access
  - Optimized `update_task_status` method for atomic status updates
- Centralized JSON persistence in XDG data directory (`$XDG_DATA_HOME/tsk/tasks.json`)
- Task status: Queued → Running → Complete/Failed
- Branch naming: `tsk/{task-type}/{task-name}/{task-id}` (human-readable format with task type, sanitized task name, and 8-character unique identifier)

**Docker Integration** (`src/docker/`)
- `DockerImageManager`: Centralized Docker image management with intelligent layering
- `ProxyManager`: Dedicated proxy lifecycle management (build, start, health checks, stop)
- `DockerManager`: Container execution with unified support for interactive and non-interactive modes
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
- Quit-when-done mode (`-q/--quit`): Exits automatically when queue is empty
- `TaskScheduler`: Manages task scheduling and execution delegation
  - Polls for completed jobs from the worker pool
  - Schedules queued tasks when workers are available
  - Updates terminal title with active/total worker counts by querying the pool
  - Automatic retry for agent warmup failures with 1-hour wait period
  - Tasks that fail during warmup are reset to queued status and retried after wait
- `WorkerPool`: Generic async job execution pool with semaphore-based concurrency control
  - Tracks active jobs in JoinSet for efficient completion polling
  - Provides `poll_completed()` for retrieving finished job results
  - Provides `total_workers()`, `active_workers()`, and `available_workers()` for monitoring

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
- `FileSystemOperations` trait abstracts all file system operations for testability
- `GitOperations` trait abstracts all git operations for improved testability and separation of concerns
- `TskConfig` provides TSK configuration and XDG-compliant directory paths for centralized storage

**Agents** (`src/agent/`)
- `Agent` trait defines the interface for AI agents that execute tasks
  - `build_command(instruction_path, is_interactive)`: Returns the command to execute (handles both normal and interactive modes)
  - `validate()`: Checks agent configuration (e.g., Claude credentials)
  - `warmup()`: Performs pre-execution setup (e.g., refreshing OAuth tokens)
  - `version()`: Returns the agent's version string (used to trigger Docker rebuilds when agents are upgraded)
- Available agents:
  - `claude`: Claude AI agent (default) - automatically detects version from `claude --version`
  - `codex`: Codex AI agent - automatically detects version from `codex --version`
  - `no-op`: Simple agent for testing that displays instructions
- Interactive debugging mode shows task instructions and the normal command before providing shell access

**Auto-Detection** (`src/repository.rs`)
- Automatic detection of stack based on repository files:
  - Rust: `Cargo.toml` → "rust"
  - Lua: `rockspec`, `.luacheckrc`, `init.lua` → "lua"
  - Python: `pyproject.toml`, `requirements.txt`, `setup.py` → "python"
  - Node.js: `package.json` → "node"
  - Go: `go.mod` → "go"
  - Java: `pom.xml`, `build.gradle`, `build.gradle.kts` → "java"
  - Default: "default" (when no specific files found)
- Automatic project name detection from repository directory name with cleaning for Docker compatibility
- Used by `TaskBuilder`, `DockerBuildCommand`, and `ShellCommand` when `--stack` and `--project` flags are not provided
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
  - Use `let ctx = AppContext::builder().build();` to automatically set up test-safe temporary directories and configurations
- Keep tests simple and concise while still testing core functionality. Improving existing tests is always preferred over adding new ones
- Avoid using `#[allow(dead_code)]`
- Limit `#[cfg(test)]` to tests and test utilities

### Branch and Task Conventions

- Tasks create branches with human-readable names: `tsk/{task-type}/{task-name}/{task-id}` (e.g., `tsk/feat/add-user-auth/a1b2c3d4`)
- Template-based task descriptions encourage structured problem statements.

### Docker Infrastructure

- **Layered Images**: Four-layer system for flexible customization
  - Base layer: Ubuntu 24.04 base OS and common tools (stored as `dockerfiles/base/default.dockerfile`)
  - Stack layer: Language-specific toolchains (stored as `dockerfiles/stack/{name}.dockerfile` for rust, python, node, go, java, lua, etc.)
  - Agent layer: AI agent installations (stored as `dockerfiles/agent/{name}.dockerfile` for claude, codex, etc.)
  - Project layer: Project-specific dependencies (stored as `dockerfiles/project/{name}.dockerfile`, optional, falls back to default)
- **Custom Project Dockerfiles**: Place project-specific Dockerfiles in `.tsk/dockerfiles/project/{project-name}.dockerfile`
- **Proxy Image** (`dockerfiles/tsk-proxy/`): Squid proxy for controlled network access
  - Custom proxy configuration: Place a `squid.conf` file in the TSK config directory (`~/.config/tsk/squid.conf` by default) to override the default proxy configuration
  - The custom configuration will be used when building/rebuilding the proxy image
- Git configuration inherited via Docker build args from host user
- Automatic image rebuilding when missing during task execution
- Agent version tracking: Docker images are rebuilt when agent versions change (via `TSK_AGENT_VERSION` ARG)

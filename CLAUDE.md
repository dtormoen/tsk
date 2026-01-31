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
- `server start`: Start the TSK server daemon (supports `-w/--workers`, `-q/--quit`, `-s/--sound`)
- `server stop`: Stop the running TSK server
- `docker build`: Build required docker images (supports `--proxy-only` to build only the proxy)
- `proxy stop`: Stop the running TSK proxy
- `template list`: List available task type templates

**Task Management** (`src/task.rs`, `src/task_storage.rs`, `src/task_manager.rs`)
- `TaskBuilder` provides consistent task creation with builder pattern
- Repository is cloned at task creation time using git clone with optimized pack files, then overlaid with working directory state:
  - Git clone creates an optimized repository copy (1-2 pack files instead of 30+) with full history
  - Working directory files are overlaid to preserve uncommitted/unstaged changes
  - This captures all tracked files with current content, staged files, and untracked non-ignored files
  - The `.tsk` directory is copied separately for project-specific configurations
  - This ensures all tasks have a valid repository copy that exactly matches what `git status` shows
- `TaskStorage` trait abstracts storage with JSON-based implementation
  - Thread-safe with mutex locking for file access
  - Optimized `update_task_status` method for atomic status updates
- Centralized JSON persistence in XDG data directory (`$XDG_DATA_HOME/tsk/tasks.json`)
- Task status: Queued → Running → Complete/Failed
- Branch naming: `tsk/{task-type}/{task-name}/{task-id}` (human-readable format with task type, sanitized task name, and 8-character unique identifier)

**Docker Integration** (`src/docker/`)
- `DockerImageManager`: Centralized Docker image management with intelligent layering
- `ProxyManager`: Dedicated proxy lifecycle management with automatic cleanup and network isolation
  - Skips proxy build if proxy is already running (faster startup)
  - Config changes picked up when proxy stops and restarts
  - Automatically stops proxy when no agents are connected and no tasks are queued
  - Uses Docker network inspection to count connected agent containers
- `DockerManager`: Container execution with unified support for interactive and non-interactive modes
- Security-first containers with dropped capabilities
- **Per-container network isolation**: Each agent runs in an isolated internal network that can only communicate with the proxy
- Proxy-based URL filtering (Squid) for API-only access with domain whitelist
- Host service access via `host.docker.internal` (automatically added to NO_PROXY)
- Volume mounting for repository copies and agent config
- Layered image system: base → tech-stack → agent → project
- Automatic fallback to default project layer when specific layer is missing

**Storage** (`src/context/`)
- `TskEnv`: Manages XDG-compliant directory paths (data_dir, runtime_dir, config_dir) and runtime environment settings (editor, terminal type)
- `TskConfig`: User configuration loaded from tsk.toml (docker limits, project-specific settings)
- Centralized task storage across all repositories
- Runtime directory for server socket and PID file

**Configuration File** (`$XDG_CONFIG_HOME/tsk/tsk.toml`)
- Optional TOML configuration file for user-configurable options
- Loaded at startup and accessible via `AppContext::tsk_config()`
- Missing file or invalid TOML uses defaults (fail-open with warnings)
- Supports Docker container resource limits and project-specific settings:
  ```toml
  [docker]
  memory_limit_gb = 12.0  # gigabytes (default: 12.0)
  cpu_limit = 8           # number of CPUs (default: 8)

  [git_town]
  enabled = true  # Enable git-town parent branch tracking (default: false)


  # Project-specific configuration (matches project name from --project or auto-detection)
  [project.my-go-project]
  agent = "claude"            # Default agent for this project
  stack = "go"                # Default stack for this project
  volumes = [
      # Bind mount: map host path to container path (supports ~ expansion)
      { host = "~/.cache/go-mod", container = "/go/pkg/mod" },
      # Named volume: Docker-managed volume (prefixed with "tsk-")
      { name = "go-build-cache", container = "/home/agent/.cache/go-build" },
      # Read-only mount
      { host = "/etc/ssl/certs", container = "/etc/ssl/certs", readonly = true }
  ]
  ```
- **Priority order**: CLI flags > project config > auto-detection > defaults

**Server Mode** (`src/server/`, `src/client/`)
- `TskServer`: Continuous task execution daemon
- `TskClient`: Client for communicating with server
- Unix socket-based IPC protocol
- Parallel task execution with configurable workers (default: 1)
- Quit-when-done mode (`-q/--quit`): Exits automatically when queue is empty
- Sound notifications (`-s/--sound`): Play sound on task completion (platform-specific)
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

**Git Operations** (`src/git.rs`, `src/git_sync.rs`, `src/context/git_operations.rs`)
- Repository cloning to centralized task directories using `CloneLocal::NoLinks` for optimized pack files
- Working directory overlay preserves uncommitted/unstaged changes from source
- Isolated branch creation and result integration
- Automatic commit and fetch operations
- `GitSyncManager`: Repository-level synchronization for concurrent git operations
  - Prevents concurrent fetch operations to the same repository
  - Uses repository path-based locking mechanism
- **Submodule Support**: Full support for repositories with git submodules
  - Copies `.git/modules/` directory to preserve submodule git data without network access
  - Fixes gitdir paths in submodule `.git` files to point to correct locations
  - Agents can work on files across superproject and all submodules
  - Commits made in submodules are fetched back with the same branch name as the superproject task
  - Only repositories (base and submodules) with actual changes get branches created
  - Graceful fallback: if submodule setup fails, contents are treated as regular files
- **Git-Town Integration**: Optional parent branch tracking for git-town users
  - Enable with `git_town.enabled = true` in tsk.toml
  - When enabled, task branches automatically record their parent branch
  - Parent is the branch checked out when the task was created
  - Uses git config: `git-town-branch.<branch>.parent`
  - Graceful failure: if parent cannot be set, logs warning and continues

**Dependency Injection** (`src/context/`)
- `AppContext` provides centralized resource management with builder pattern
- Traits in the `AppContext` should be accessed via the `AppContext`
- Factory pattern prevents accidental operations in tests
- `FileSystemOperations` trait abstracts all file system operations for testability
- `GitOperations` trait abstracts all git operations for improved testability and separation of concerns
- `TskEnv` provides XDG-compliant directory paths and runtime environment settings (editor, terminal type)
- `TskConfig` provides user configuration loaded from tsk.toml

**Agents** (`src/agent/`)
- `Agent` trait defines the interface for AI agents that execute tasks
  - `build_command(instruction_path, is_interactive)`: Returns the command to execute (handles both normal and interactive modes)
  - `validate()`: Checks agent configuration (e.g., Claude credentials)
  - `warmup()`: Performs pre-execution setup (e.g., refreshing OAuth tokens)
  - `version()`: Returns the agent's version string (used to trigger Docker rebuilds when agents are upgraded)
  - `files_to_copy()`: Returns files to copy into container before starting (used for agent config files)
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
  - Exception: Tests in `src/context/*` that are directly testing TskEnv or TskConfig functionality
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
  - Proxy is automatically rebuilt on each startup to pick up config changes (Docker layer caching makes unchanged rebuilds fast)
  - Proxy automatically stops when no agent containers are connected and no tasks are queued
- Git configuration resolved dynamically from the repository being built (respects per-repo author settings)
- Automatic image rebuilding when missing during task execution
- Agent version tracking: Docker images are rebuilt when agent versions change (via `TSK_AGENT_VERSION` ARG)

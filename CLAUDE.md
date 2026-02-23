# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Important files:
- @justfile - key development commands. These should be used when possible over raw cargo commands.
- @README.md - user facing documentation for the project. This could cover key user facing details without going into too much detail. Make sure it stays up to date with your changes.

## Architecture Overview

TSK implements a command pattern with dependency injection for testability. The core workflow: queue tasks → execute in containers (Docker or Podman) → create git branches for review. TSK can run in server mode for continuous task processing across multiple repositories.

### Key Components

**CLI Commands** (`src/commands/`)

*Task Commands (implicit "task" noun):*
- `run`: Immediately execute single tasks (tracked in DB, supports piped input via stdin for descriptions, supports `--no-network-isolation`, supports `--dind`)
- `shell`: Launch sandbox container with agent for interactive use (tracked in DB, supports piped input via stdin for descriptions, supports `--no-network-isolation`, supports `--dind`)
- `add`: Queue tasks with descriptions and templates (supports piped input via stdin for descriptions, supports `--parent <taskid>` for task chaining, supports `--no-network-isolation`, supports `--dind`)
- `list`: Display task status and results (shows parent task information)
- `clean`: Delete completed tasks (skips parents with queued/running children)
- `delete <task-id>`: Delete a specific task
- `retry <task-id>`: Retry a previous task

*Subcommand Groups:*
- `server start`: Start the TSK server daemon (supports `-w/--workers`, `-q/--quit`, `-s/--sound`)
- `server stop`: Stop the running TSK server
- `docker build`: Build required docker images (supports `--proxy-only` to build only the proxy)
- `template list`: List available task type templates

**Task Management** (`src/task.rs`, `src/context/task_storage.rs`, `src/task_manager.rs`, `src/task_runner.rs`)
- `TaskBuilder` provides consistent task creation with builder pattern
- Repository is cloned at task creation time using git clone with optimized pack files, then overlaid with working directory state:
  - Git clone creates an optimized repository copy (1-2 pack files instead of 30+) with full history
  - Working directory files are overlaid to preserve uncommitted/unstaged changes
  - This captures all tracked files with current content, staged files, and untracked non-ignored files
  - The `.tsk` directory is copied separately for project-specific configurations
  - This ensures all tasks have a valid repository copy that exactly matches what `git status` shows
- `TaskStorage` is a concrete SQLite-backed struct in `src/context/task_storage.rs` (`rusqlite` with bundled SQLite)
  - WAL mode and busy_timeout enabled for safe concurrent multi-process access
  - `tokio::task::spawn_blocking` bridges sync rusqlite into async
- Automatic migration from legacy `tasks.json` to SQLite on first run (renames to `tasks.json.bak`)
- Centralized SQLite persistence in XDG data directory (`$XDG_DATA_HOME/tsk/tasks.db`)
- **Config snapshotting**: At task creation, the fully-resolved `ResolvedConfig` is serialized to JSON and stored in the `resolved_config` column. At execution time, `docker::resolve_config_from_task()` deserializes the snapshot instead of re-resolving from config files. Falls back to live resolution if `resolved_config` is NULL (pre-migration tasks). Chained tasks inherit the parent's snapshot.
- Task status: Queued → Running → Complete/Failed (Waiting status shown in list for tasks awaiting parent completion)
- Two execution paths:
  - **Server-scheduled**: `add` stores tasks as `Queued`, server scheduler picks them up and transitions to `Running`
  - **Inline**: `run`/`shell` store tasks as `Running` (via `TaskRunner::store_and_execute_task`) and execute immediately; the `Running` status prevents the scheduler from picking them up
- Branch naming: `tsk/{task-type}/{task-name}/{task-id}` (human-readable format with task type, sanitized task name, and 8-character unique identifier)
- **Task Chaining**: Tasks can specify a parent task via `--parent <taskid>` (short: `-p`)
  - Database schema supports multiple parents (`parent_ids` stored as JSON array in TEXT column), but CLI currently accepts only one
  - Child tasks wait until their parent task completes before starting
  - Repository is copied from the completed parent task's folder (not user's working directory)
  - Child task's branch starts from parent task's final commit
  - Git-town parent is set to the parent task's branch (not the original user branch)
  - Chained tasks (A → B → C) are supported naturally
  - If parent task fails, all child tasks are marked as Failed (cascading failure)

**Docker Integration** (`src/docker/`)
- **Container engine**: Supports Docker (default) and Podman via `--container-engine` flag on container-related subcommands (`run`, `shell`, `retry`, `server start`, `docker build`) or top-level `container_engine` in tsk.toml. Podman uses host network mode for builds and case-insensitive error matching.
- `DockerImageManager`: Centralized Docker image management with intelligent layering
- `ProxyManager`: Dedicated proxy lifecycle management with automatic cleanup and network isolation
  - Per-configuration proxy instances: tasks with different proxy configs (host_ports, squid_conf) get separate proxy containers named `tsk-proxy-{fingerprint}`
  - Fingerprint derived from sorted host_ports + squid.conf content (SHA256, first 8 hex chars)
  - Custom squid.conf mounted at runtime via bind mount (not baked into image)
  - Skips proxy build if proxy is already running (faster startup)
  - Automatically stops proxy when no agents are connected
  - Uses Docker network inspection to count connected agent containers per proxy
- `DockerManager`: Container execution with unified support for interactive and non-interactive modes
- Security-first containers with dropped capabilities
- **Docker-in-Docker (DIND) support**: Opt-in via `--dind` flag or config (`dind = true` in `[defaults]` or `[project.<name>]`). When enabled, applies a custom seccomp profile allowing nested container operations, disables AppArmor confinement, and keeps SETUID/SETGID capabilities for rootless Podman user-namespace setup. When disabled (default), security_opt is left at Docker/Podman defaults and SETUID/SETGID are dropped. Resolution order: CLI flag > `[project.<name>]` > `[defaults]` > default (false).
- **Per-container network isolation**: Each agent runs in an isolated internal network that can only communicate with the proxy (see [Network Isolation Guide](docs/network-isolation.md)). Can be disabled per-task with `--no-network-isolation`
- Proxy-based URL filtering (Squid) for API-only access with domain allowlist
- Host service access via TCP port forwarding through the proxy container (configured via `host_ports` in `[defaults]` or `[project.<name>]`)
- **Container environment variables**: All task containers receive `TSK_CONTAINER=1` and `TSK_TASK_ID=<task-id>` for in-container detection. When `TSK_CONTAINER=1` is set, TSK auto-defaults to Podman and skips proxy/network isolation (handled by outer container).
- **Directory override environment variables**: `TSK_DATA_HOME`, `TSK_RUNTIME_DIR`, and `TSK_CONFIG_HOME` override the corresponding XDG base directories for TSK only (without affecting other XDG-aware software). Resolution priority: builder override > TSK env var > XDG env var > default fallback.
- Volume mounting for repository copies and agent config
- Layered image system: base → tech-stack → agent → project
- Automatic fallback to default project layer when specific layer is missing

**Storage** (`src/context/`)
- `TskEnv`: Manages directory paths (data_dir, runtime_dir, config_dir) and runtime environment settings (editor, terminal type). TSK-specific env vars (`TSK_DATA_HOME`, `TSK_RUNTIME_DIR`, `TSK_CONFIG_HOME`) take precedence over XDG vars, enabling isolated testing without affecting other XDG-aware software
- `TskConfig`: User configuration loaded from tsk.toml. Uses shared config shape with `[defaults]` and `[project.<name>]` sections. `TskConfig::resolve_config(project_name, project_config, project_root)` returns a `ResolvedConfig` with all layers merged: `user [project.<name>] > project .tsk/tsk.toml > user [defaults] > built-in`. Project-level config is loaded from `.tsk/tsk.toml` via `load_project_config()`. `ResolvedConfig::proxy_config()` extracts a `ResolvedProxyConfig` with host_ports and squid_conf for proxy fingerprinting.
- Centralized task storage across all repositories
- Runtime directory for PID file

**Configuration File** (`$XDG_CONFIG_HOME/tsk/tsk.toml` and `.tsk/tsk.toml`)
- User config loaded at startup from `~/.config/tsk/tsk.toml`, accessible via `AppContext::tsk_config()`
- Project config loaded from `.tsk/tsk.toml` in the project root at task creation/execution time via `tsk_config::load_project_config()`
- Missing file or invalid TOML uses defaults (fail-open with warnings)
- Old format (`[docker]`, `[proxy]`, `[git_town]` sections) is detected and prints migration error
- Uses shared config shape: `[defaults]` for all projects, `[project.<name>]` for per-project overrides, `.tsk/tsk.toml` for project-level defaults
- `TskConfig::resolve_config(project_name, project_config)` merges layers: `user [project.<name>] > project .tsk/tsk.toml > user [defaults] > built-in`
- See README.md for full configuration reference and examples

**Server Mode** (`src/server/`)
- `TskServer`: Continuous task execution daemon
- Signal-based lifecycle: SIGTERM/SIGINT for graceful shutdown (kills managed containers, marks tasks as Failed, stops idle proxy, cleans up PID file); second signal forces immediate exit
- Parallel task execution with configurable workers (default: 1)
- Quit-when-done mode (`-q/--quit`): Exits automatically when queue is empty
- Sound notifications (`-s/--sound`): Play sound on task completion (platform-specific)
- `TaskScheduler`: Manages task scheduling and execution delegation
  - Polls for completed jobs from the worker pool
  - Schedules queued tasks when workers are available
  - Updates terminal title with active/total worker counts by querying the pool
  - Proactive Claude OAuth token check: before scheduling Claude tasks, reads `~/.claude/.credentials.json` and skips scheduling if the token expires within 5 minutes. Re-checks every 1 minute. Sends a desktop notification (once per expiry episode) prompting the user to run `claude /login`
  - Automatic retry for agent warmup failures with 1-hour wait period
  - Tasks that fail during warmup are reset to queued status and retried after wait
  - Parent-aware scheduling: tasks with incomplete parents are skipped
  - Prepares child tasks by copying repository from parent task before scheduling
  - Handles cascading failures when parent tasks fail
  - Auto-cleans completed/failed tasks based on `[server]` config (default: enabled, 7 days, hourly check, runs on startup)
- `WorkerPool`: Generic async job execution pool with semaphore-based concurrency control
  - Tracks active jobs in JoinSet for efficient completion polling
  - Provides `poll_completed()` for retrieving finished job results
  - Provides `total_workers()`, `active_workers()`, and `available_workers()` for monitoring

**Git Operations** (`src/git.rs`, `src/git_sync.rs`, `src/git_operations.rs`)
- Repository cloning to centralized task directories using `CloneLocal::NoLinks` for optimized pack files
- Working directory overlay preserves uncommitted/unstaged changes from source
- Isolated branch creation and result integration
- Automatic commit and fetch operations
- `GitSyncManager`: Repository-level synchronization for concurrent git operations
  - Prevents concurrent fetch operations to the same repository
  - Uses dual-lock architecture: in-process `tokio::Mutex` + cross-process `flock(2)` file locks
  - Lock file at `<repo>/.git/tsk.lock` provides cross-process safety (multiple `tsk` processes on same repo)
- **Submodule Support**: Full support for repositories with git submodules
  - Copies `.git/modules/` directory to preserve submodule git data without network access
  - Fixes gitdir paths in submodule `.git` files to point to correct locations
  - Agents can work on files across superproject and all submodules
  - Commits made in submodules are fetched back with the same branch name as the superproject task
  - Only repositories (base and submodules) with actual changes get branches created
  - Graceful fallback: if submodule setup fails, contents are treated as regular files
- **Git-Town Integration**: Optional parent branch tracking for git-town users
  - Enable with `git_town = true` in `[defaults]` or `[project.<name>]` in tsk.toml
  - When enabled, task branches automatically record their parent branch
  - Parent is the branch checked out when the task was created
  - Uses git config: `git-town-branch.<branch>.parent`
  - Graceful failure: if parent cannot be set, logs warning and continues

**Dependency Injection** (`src/context/`)
- `AppContext` provides centralized resource management with builder pattern
- Docker client is NOT part of AppContext; it is constructed at command entry points and injected into `DockerManager`/`TaskRunner` via constructors. This ensures commands that don't need Docker (add, list, clean, delete) work without a Docker daemon
- Factory pattern prevents accidental operations in tests
- `TskEnv` provides XDG-compliant directory paths and runtime environment settings (editor, terminal type)
- `TskConfig` provides user configuration loaded from tsk.toml
- `TaskStorage` provides centralized task persistence (single shared `Arc<TaskStorage>` instance)

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
  - `integ`: Integration test agent that runs `tsk-integ-test.sh` from the project workspace
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

**Skills Marketplace** (`skills/`, `.claude-plugin/marketplace.json`)
- Claude Code skills following the [Agent Skills](https://agentskills.io) open standard
- Marketplace manifest at `.claude-plugin/marketplace.json` registers available plugins
- Each skill is a plugin directory: `skills/<name>/.claude-plugin/plugin.json` + `skills/<name>/skills/<name>/SKILL.md`
- Users install via `/plugin marketplace add dtormoen/tsk` then `/plugin install <name>@dtormoen/tsk`

### Development Conventions

- Avoid `#[allow(dead_code)]` directives
- Always keep documentation up to date following rustdoc best practices
- Keep CLAUDE.md file simple, but up to date
- Avoid `unsafe` blocks

### Commit Conventions

Commits use conventional commit prefixes. These map to changelog groups via `release-plz.toml`:

- `feat`: New user-facing functionality or capabilities (appears in release notes under "added")
- `fix`: Bug fixes to existing behavior (appears in release notes under "fixed")
- `docs`: Documentation-only changes (appears in release notes under "documentation")
- `test`: Adding or updating tests with no production code changes (excluded from release notes)
- `refactor`: Code restructuring with no behavior change (excluded from release notes)
- `chore`: Maintenance tasks like dependency updates, CI config, releases (excluded from release notes)

Use `refactor` when reorganizing code internals without changing what the software does. Use `chore` for non-code changes or tooling. Both are skipped in changelogs, so use `feat` or `fix` if the change is meaningful to users.

For user-facing breaking changes, add `!` after the type (e.g., `feat!:`, `fix!:`). The commit title must explain why the change is breaking so users understand the impact without reading the full diff (e.g., `feat!: rename --workers to --concurrency for clarity`).

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

### Integration Tests

- Stack layer integration tests live in `tests/integration/projects/<stack>/`
- Each project has a `tsk-integ-test.sh` script that validates the stack works
- Run with `just integration-test` (requires Docker/Podman and internet access for image builds)
- To add a new test: create a directory in `tests/integration/projects/` with project files and a `tsk-integ-test.sh`

### Branch and Task Conventions

- Tasks create branches with human-readable names: `tsk/{task-type}/{task-name}/{task-id}` (e.g., `tsk/feat/add-user-auth/a1b2c3d4`)
- Template-based task descriptions encourage structured problem statements.
- Templates support YAML-style frontmatter (`---` delimited) with a `description` field shown in `tsk template list`. Frontmatter is stripped before template content reaches agents.

### Docker Infrastructure

- See README.md for the layered image system, custom dockerfiles, and proxy configuration
- **Inline Docker layers**: `setup`, `stack_config[name].setup`, and `agent_config[name].setup` from `ResolvedConfig` inject Dockerfile content at the project, stack, and agent layer positions respectively. Config-defined layers take priority over embedded assets. This allows defining entirely new stacks/agents in TOML (e.g., `stack_config.scala.setup`).
- Git configuration resolved dynamically from the repository being built (respects per-repo author settings)
- Automatic image rebuilding when missing during task execution
- Agent version tracking: Docker images are rebuilt when agent versions change (via `TSK_AGENT_VERSION` ARG)

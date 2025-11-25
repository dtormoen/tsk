# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.1](https://github.com/dtormoen/tsk/compare/v0.6.0...v0.6.1) - 2025-11-25

### Added

- add uv to base image
- add quit-when-done mode to server start

### Fixed

- add line buffering to Docker log streaming to prevent parse errors
- resolve git fetch errors when copying repos from specific commits

### Other

- update deps
- *(docker)* simplify agent installation and environment setup

## [0.6.0](https://github.com/dtormoen/tsk/compare/v0.5.4...v0.6.0) - 2025-10-26

This release has a few large changes. First we've added
[codex](https://openai.com/codex/) support! You can now launch codex agents or even
launch multiple agents in parallel:

```bash
tsk add --type feat --name greeting --description "Add a greeting to all TSK commands" --agent claude,codex
```

This will create two tasks that are identical except for `claude` will work on one and
`codex` will work on the other. This is great for comparing the performance of different
agents.

We've also simplified TSK commands:
- `tsk debug` is now called `tsk shell`
- `tsk quick` is now called `tsk run`
- `tsk run` has been removed
- `--timeout` has been removed from a few commands as it was not actually implemented
- The `claude-code` agent has been renamed to `claude` throughout the codebase

### Added

- [**breaking**] simplify command structure for better UX
- add multi-agent support for task creation
- *(codex)* improve log processor output quality
- *(codex)* add JSON log processor with human-readable output
- add codex agent support

### Other

- reorganize and enhance README
- improve documentation
- rename claude-code agent to claude

## [0.5.4](https://github.com/dtormoen/tsk/compare/v0.5.3...v0.5.4) - 2025-10-26

### Added

- add pip and ty to python dockerfile

### Fixed

- handle terminal size and resize in debug command

### Other

- move agent layer after project layer since claude code updates frequently

## [0.5.3](https://github.com/dtormoen/tsk/compare/v0.5.2...v0.5.3) - 2025-10-21

### Fixed

- don't mount .claude.json to avoid corrupting it
- deduplicate consecutive parsing errors in Claude Code log processor
- detect lua before python

## [0.5.2](https://github.com/dtormoen/tsk/compare/v0.5.1...v0.5.2) - 2025-10-01

### Fixed

- handle empty git repositories gracefully

## [0.5.1](https://github.com/dtormoen/tsk/compare/v0.5.0...v0.5.1) - 2025-09-25

This release mainly adds a few new features:
- The outputs while agents are working are much more detailed. They include:
    - The model being used e.g. opus or sonnet
    - The sub-agent that is active
    - The full instructions and outputs of sub-agents
- Adds the ability to pipe in your instructions: `echo "Make a sweet app" | tsk add --type feat --name an-app
- Adds the ability to define a custom proxy configuration

### Added

- add stdin pipe support for task descriptions
- enhance claude-code log processor with task-aware multi-agent output tracking
- add output logging for claude-code agent debugging
- display model name in claude code log processor output
- add support for custom squid proxy configuration

### Fixed

- implement atomic file writes to prevent race conditions

### Other

- simplify Agent trait by consolidating build_command methods
- simplify DockerManager by integrating ProxyManager and using AppContext
- simplify TSK configuration by removing redundant XdgConfig struct
- simplify Docker image management interface
- remove unused additional_files functionality from Docker composer
- remove obsolete tsk-base dockerfile

## [0.5.0](https://github.com/dtormoen/tsk/compare/v0.4.1...v0.5.0) - 2025-09-08

This release should make Docker builds more reliable overall, but it does introduce a
breaking change where docker layer snippets for projects, agents, and stacks which
previously would be stored in e.g. `.tsk/dockerfiles/project/<project_name>/Dockerfile`
are now stored in `.tsk/dockerfiles/project/<project_name>.dockerfile`. See the `.tsk`
or `dockerfiles` directory in `tsk` for examples of how to structure Dockerfile snippets.

### Added

- add Docker build lock manager to prevent concurrent image builds
- implement agent version tracking for automatic Docker rebuilds
- increase CPU and memory limits to 8 CPUs and 12gb
- [**breaking**] simplify Docker image generation with Handlebars templating

### Fixed

- copy working directory versions of files including unstaged changes
- improve lua stack snippet
- allow dots and underscores in project names for Docker layer matching

### Other

- upgrade go version in docker snippet
- extract proxy management into dedicated ProxyManager module
- simplify Docker asset directory structure
- *(server)* replace TaskExecutor with TaskScheduler and WorkerPool

## [0.4.1](https://github.com/dtormoen/tsk/compare/v0.4.0...v0.4.1) - 2025-09-01

### Fixed

- handle non-JSON output gracefully in Claude Code log processor
- rework Python tech-stack to use UV with proper virtual environment
- add support for symlinks in git repository copy operations

## [0.4.0](https://github.com/dtormoen/tsk/compare/v0.3.2...v0.4.0) - 2025-08-25

This release is largely focused on refactoring which will help create a solid foundation
for future planned changes. Some notable improvements are:
- Fix that ensures proxy can be restarted (requires a `tsk docker build`)
- Fixed `tsk debug` so now you can start an interactive session with Claude in a docker 
  container or debug issues with the project docker containers you've created
- tsk checks if claude code is working before launching a container. If this step fails, 
  it typically means you've used up your subscription limits. `tsk` will now retry the 
  task once each hour rather than fail it outright. This can be used for example to 
  queue up a lot of work overnight and tsk will wait until your limit is refilled
- Unit tests are faster, no longer flaky, and are easier to properly isolate to run
  in parallel

### Added

- [**breaking**] fix debug command and add interactive agent support
- [**breaking**] add is_interactive field to Task for controlling execution mode
- add automatic retry for agent warmup failures in server mode

### Fixed

- flaky tests
- add delay to interactive commands to prevent missing initial output
- handle stale PID file in proxy container for clean restarts
- add health check to proxy container and wait for readiness
- correct Claude Code log parser to handle TodoWrite output format
- update Claude Code log processor for new JSON format
- clean up temporary instruction file when task creation is cancelled
- update justfile to better reflect CI linting behavior
- optimize warmup failure wait behavior test to run in <1 second
- remove flaky test

### Other

- enable ignored tests using AppContext and mock docker clients
- use Bollard for interactive containers instead of docker CLI
- use AppContext::builder() for test setup instead of XdgConfig::builder()
- simplify layered asset tests using DRY principles and AppContext
- prevent git config access in tests and use AppContext for test isolation
- update dependencies
- rename xdg variable names to better reflect TskConfig usage
- simplify RepoManager by using AppContext directly
- store AppContext in TaskRunner instead of rebuilding it
- simplify DockerImageManager using AppContext and DRY principles
- make executor tests deterministic and remove redundancy
- move git configuration from DockerImageManager to TskConfig
- extract TaskBuilder to separate module for better code organization
- rename XdgDirectories to TskConfig throughout codebase
- replace XdgConfig::with_paths with AppContext in all tests
- simplify test setup by leveraging AppContext's test defaults
- consolidate Config into XdgDirectories and add test-safe context creation
- replace MockAssetManager with FileSystemAssetManager in layered asset tests
- simplify debug command to use TaskManager for unified task execution
- simplify TaskRunner constructor to accept AppContext
- use copied_repo_path from Task struct to determine task directories
- consolidate TaskManager constructors into single new() method
- remove deprecated new() constructor from ClaudeCodeAgent
- pass Config through AgentProvider to eliminate direct env var access
- configure clippy to disallow env variable setting or removing
- add Config struct to centralize environment variable access
- convert RepositoryContext from trait to stateless functions
- add guidance on testing
- fix flaky warmup failure wait behavior test
- stronger wording around fixing just precommit issues
- remove MockFileSystem from file_system module
- replace MockFileSystem with temporary directories in tests
- replace MockFileSystem with temporary directories in tests
- replace MockFileSystem with temporary directories in tests
- remove repository dependency from tsk list command
- update to rust 1.89

## [0.3.2](https://github.com/dtormoen/tsk/compare/v0.3.1...v0.3.2) - 2025-08-05

### Added

- expand project Dockerfile search to include user-level config
- add bulk operations for retry and delete commands
- upgrade Docker images to Ubuntu 24.04 and resolve GID conflict

### Fixed

- ensure project layer detection uses copied repository in server mode

## [0.3.1](https://github.com/dtormoen/tsk/compare/v0.3.0...v0.3.1) - 2025-07-18

This release fixes an issue where Dockerfiles in projects could conflict with tsk's own 
Docker image generation and improves support for Go projects.

### Added

- expand proxy allowed domains for all supported language package managers

### Fixed

- prevent project Dockerfiles from overriding TSK images

## [0.3.0](https://github.com/dtormoen/tsk/compare/v0.2.0...v0.3.0) - 2025-07-09

### Added

- add logging for when container starts
- add --repo argument to add, quick, and debug commands
- make --description optional for templates without {{DESCRIPTION}} placeholder
- add "ack" task type for debugging
- implement parallel task execution with configurable workers

### Fixed

- resolve clippy warnings and improve code quality
- use task ID instead of timestamp for docker container names
- update dependency that causes server hangs
- force local installs to use the lock file
- remove duplicate task status updates causing server shutdown hang
- resolve TSK server worker management and task completion issues

### Other

- add a demo gif of tsk
- remove unused code and fix dead code warnings
- remove unused test utilities and fix dead code warnings
- remove lib target and consolidate binary-only structure
- implement human-readable branch naming format
- [**breaking**] replace timestamp-based task IDs with 8-character nanoid IDs
- rename -i/--instructions flag to -p/--prompt in CLI commands
- [**breaking**] remove log file saving feature from agents
- restructure CLI to follow noun-verb command pattern

## [0.2.0](https://github.com/dtormoen/tsk/compare/v0.1.0...v0.2.0) - 2025-07-07

### Added

- plan template includes success criteria and out of scope section
- ensure proxy image is built before task execution

### Fixed

- release storage mutex lock after task completion to prevent server deadlock
- remove rust specific language from default templates

### Other

- add comprehensive Docker builds documentation
- migrate task_manager tests from mocks to real implementations
- replace git mock implementations with integration tests
- replace git mock implementations with integration tests
- add repo_path parameter to is_git_repository method
- eliminate unsafe blocks in XdgDirectories tests
- add project specific task templates that mention rust best practices
- update install instructions in README.md

## [0.1.0](https://github.com/dtormoen/tsk/releases/tag/v0.1.0) - 2025-06-27

### Added

- add tech-stack Dockerfiles for Go, Python, Node, Java, and Lua
- improve ClaudeCodeLogProcessor with richer output
- extend copy_repo to include untracked non-ignored files
- add filesystem support for project-specific Dockerfiles
- implement streaming Docker build output
- always rebuild Docker images for tasks
- add project-specific Docker layer for tsk
- configure Rust builds to use external target directory
- optimize copy_repo to only copy git-tracked files
- add project-layer template for creating project-specific Docker layers
- add auto-detection for --tech-stack and --project flags
- [**breaking**] introduce DockerImageManager for centralized Docker image management
- [**breaking**] update branch naming scheme to include task type
- add --dry-run flag to docker-build command
- implement multi-level template configuration scheme for Docker images
- implement multi-level template configuration scheme
- transform TSK into installable binary with embedded assets
- *(docker-build)* add --no-cache flag to force fresh builds
- Add retry functionality to tasks command
- add docker-build command to automate Docker image building
- Add a more useful error message when it can't connect to Docker
- Add terminal window title updates during task execution
- Track source commit when tasks are created
- Update templates to use Conventional Commits spec

### Fixed

- resolve format string issues for new Rust version
- resolve all precommit issues and test failures
- use docker cache by default
- remove git repository requirement for tsk run command
- correct Docker layer composition for user switching and CMD placement
- Update Claude settings to avoid timeouts
- remove just target that's no longer needed
- Make it clear in plan template that no code should be written

### Other

- initial release
- add release-plz to manage versioning
- [**breaking**] upgrade project to Rust 2024 edition
- Create rust.yml
- make output of testing commands less verbose
- make Task::new require created_at and copied_repo_path parameters
- rename project_root to build_root in Docker image manager
- simplify Task creation by removing redundant constructor
- move repository copying from task execution to task creation
- [**breaking**] make Docker components repository-aware for multi-repo support
- [**breaking**] unify Docker container execution methods and improve interactive mode
- unify debug and task execution pathways
- use Bollard API for Docker image building instead of CLI commands
- consolidate unit tests according to Rust best practices
- [**breaking**] make Task struct fields required and remove Option types
- standardize agent naming to use 'claude-code' consistently
- update CLAUDE.md
- streamline README
- cleanup parts of README that are no longer accurate
- *(task)* remove unused repo_root parameter from write_instructions_content
- *(task)* remove instructions() method from TaskBuilder
- *(task)* separate instructions content from file path in TaskBuilder
- *(client)* integrate TskClient into AppContext dependency injection system
- Move terminal operations to AppContext
- Fix server EOF error when handling empty client connections
- Add agent warmup mechanism for pre-container setup
- Fix client EOF error when server closes connection without response
- Transform TSK into a centralized server-based system
- Refactor log processor to be specific to Claude Code agent
- Improve task duration display with human-readable format
- Add desktop notifications for task completion events
- Associate each task with repository root to support multi-repo operations
- Improve plan template
- Improve editor workflow by editing instruction files from repository root
- Avoid creating branches when no commits exist
- Add more detail to plan task
- Improve debug command to start interactive session directly
- Update templates slightly
- Make todo list display more compact using status emojis
- Refactor TSK to support multiple AI agents through extensible provider pattern
- Add TODO update display in log processor
- Unify task storage by removing separate quick-tasks directory
- Consolidate test MockDockerClient implementations into reusable test utilities
- Simplify the code a bit
- Add a planning task type
- Refactor task management with TaskBuilder pattern and extract TaskStorage
- Refactor task execution into dedicated TaskRunner
- WIP, refactoring task_manager
- WIP removing DockerManagerTrait
- Add ripgrep
- Fix unit tests in task_manager.rs to be thread-safe
- Use multiple threads for unit tests again
- Update Docker resource limits
- Refactor git operations to use git2-rs library
- Add some guidance on tests to CLAUDE.md
- Add a refactor template
- Add GitOperations trait to AppContext for improved testability
- Add FileSystemOperations trait and refactor AppContext to use builder pattern
- Update CLAUDE.md
- Clean up log parser issues
- Introduce AppContext for dependency injection
- Implement command pattern for CLI commands
- Add a doc template
- Fix task directory cleanup in clean subcommand
- Add interactive instruction editing with -e flag for add and quick commands
- Add tasks subcommand for managing task list
- Make --type flag optional and support any template-based task types
- Refactor TSK to always use instructions files and add template support
- Show only message type for non-assistant/result logs
- Improve log streaming output formatting
- Parse task result status from Claude Code JSON logs
- Implement real-time log streaming with JSON formatting for TSK
- TSK automated changes for task: fix-precommit
- Refactor shared task execution logic into TaskManager module
- Add list and run subcommands for task management
- Add git config to docker image
- Add 'add' subcommand for queuing tasks
- TSK automated changes for task: instructions-file
- Make it easier to view output
- TSK debug session changes for: stream-output
- Run squid as squid user
- Add command to stop proxy
- Add nginx proxy container
- Better way to manage rust
- Pass in the description of the command
- Add some convenience just commands and attempt to unblock cargo
- Add an entrypoint script
- Launch images with firewall script
- Add setup for firewall rules
- Fix to properly merge branches
- Switch from using worktrees to copying the repo
- Add .claude directory to version control
- Add claude config file
- allow networking
- Include claude directory in launched containers
- Make sure tests that effect docker and git don't have unintended side effects
- Refactor docker.rs to reuse code and pull out constants
- Add debug subcommand
- Claude Code now works in Docker container
- Add ability to launch Docker containers
- Add base Dockerfile
- Add tests for git commands
- Add basic command line functionality
- Add a CLAUDE file
- Add a README that explains the project
- Initial commit

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

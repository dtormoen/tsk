# TSK - AI Agent Task Manager

A Rust CLI tool that lets you delegate development tasks to AI agents running in sandboxed Docker environments. Get back git branches for human review.

![TSK Demo](./docs/images/tsk-demo.gif)

## What it does

TSK enables a "lead engineer + AI team" workflow:
1. **Assign tasks** to AI agents with natural language descriptions and task type templates to automate prompt boilerplate
2. **Agents work autonomously** in isolated Docker containers
3. **Get git branches** back with their changes for review
4. **Review and merge** using your normal git workflow

Think of it as having a team of engineers who work independently and submit pull requests for review.

## Installation

### Requirements

- [Rust](https://rustup.rs/) - Rust toolchain and Cargo
- [Docker](https://docs.docker.com/get-docker/) - Container runtime
- [Git](https://git-scm.com/downloads) - Version control system
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) - AI agent (help us support more agents!)

### Install TSK

```bash
# Install using cargo
cargo install tsk-ai

# Build Docker images (required)
tsk docker build
```

## Quick Start

```bash
# Add a task using the "feat" task template
tsk add --type "feat" --name "greeting" --description "Add a warm greeting to all CLI commands"

# Run all queued tasks
tsk run

# Check results
tsk list

# Review the changes
git checkout tsk/a1b2c3d4
git diff main...HEAD

# Merge if it looks good
git checkout main && git merge tsk/a1b2c3d4
```

### Server Mode

For continuous task processing across multiple repositories:

```bash
# Start server
tsk server run

# Add tasks from any repo
cd ~/project-a && tsk add --type "fix" --name "task1" --description "..."
cd ~/project-b && tsk add --type "feat" --name "task2" --description "..."

# Stop server
tsk server stop
```

The server includes automatic retry logic for agent warmup failures (e.g., API rate limits). When a task fails during the agent warmup phase, the server will:
- Wait 1 hour before attempting any new tasks
- Reset the failed task to queued status
- Retry the task after the wait period

### Parallel Execution

TSK supports parallel task execution for improved throughput:

```bash
# Run up to 4 tasks in parallel
tsk run --workers 4

# Server mode also supports parallel execution
tsk server run --workers 4
```

Each task runs in its own isolated Docker container, so parallel execution is safe and efficient.

## Commands

### Task Commands
- `tsk add` - Queue a task
- `tsk run` - Execute queued tasks
- `tsk list` - View task status and branches
- `tsk quick` - Execute a task immediately
- `tsk debug` - Start an interactive docker container
- `tsk clean` - Clean up completed tasks
- `tsk delete <task-id>...` - Delete one or more tasks
- `tsk retry <task-id>...` - Retry one or more tasks

### Server Commands
- `tsk server run` - Start the TSK server daemon
- `tsk server stop` - Stop the running TSK server

### Configuration Commands
- `tsk docker build` - Build required docker images
- `tsk proxy stop` - Stop the TSK proxy container
- `tsk template list` - View available task type templates

Run `tsk help` or `tsk help <command>` for detailed options.

## Documentation

- [Docker Builds Guide](docs/docker-builds.md) - Comprehensive guide to TSK's Docker infrastructure, customization, and debugging

## Contributing

This project uses:
- `cargo test` for running tests
- `just precommit` for full CI checks
- See [CLAUDE.md](CLAUDE.md) for development guidelines

## License

MIT License - see [LICENSE](LICENSE) file for details.

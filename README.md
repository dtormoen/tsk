# TSK - AI Agent Task Manager

⚠️⚠️ **EARLY DEVELOPMENT SOFTWARE** ⚠️⚠️

This project is in early development. Breaking changes are expected.

A Rust CLI tool that lets you delegate development tasks to AI agents running in sandboxed Docker environments. Get back git branches for human review.

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
tsk docker-build
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
git checkout tsk/2025-06-23-1430-feat-greeting
git diff main...HEAD

# Merge if it looks good
git checkout main && git merge tsk/2025-06-23-1430-feat-greeting
```

### Server Mode

For continuous task processing across multiple repositories:

```bash
# Start server
tsk run --server

# Add tasks from any repo
cd ~/project-a && tsk add --type "fix" --name "task1" --description "..."
cd ~/project-b && tsk add --type "feat" --name "task2" --description "..."

# Stop server
tsk stop-server
```

## Commands

- `tsk add` - Queue a task
- `tsk run` - Execute queued tasks (or `--server` for daemon mode)
- `tsk list` - View task status and branches
- `tsk templates` - View available task type templates
- `tsk quick` - Execute a task immediately
- `tsk debug` - Start an interactive docker container
- `tsk tasks --clean` - Clean up completed tasks
- `tsk docker-build` - Build required docker containers

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

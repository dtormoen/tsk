# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

TSK is a Rust-based CLI tool for delegating development tasks to AI agents running in sandboxed Docker environments. The project enables a "lead engineer + AI team" workflow where tasks are executed autonomously in isolated containers and produce reviewable git branches.

## Development Commands

Since this is a Rust project in early development, use these commands:

```bash
# Build the project
cargo build

# Run tests
cargo test

# Run with debug output
RUST_LOG=debug cargo run

# Check code formatting
cargo fmt -- --check

# Run linter
cargo clippy -- -D warnings

# Build release version
cargo build --release

# Run fmt, clippy, and tests
just precommit
```

## Architecture Guidelines

### Core Components (Planned)

1. **CLI Interface** (`src/cli/`)
   - Command parsing using clap or similar
   - Subcommands: add, run, list, quick, server
   - Configuration management

2. **Task Management** (`src/task/`)
   - Task queue implementation
   - Task types and metadata
   - Status tracking and persistence

3. **Agent Integration** (`src/agents/`)
   - Abstract agent interface
   - Claude Code integration
   - Aider integration
   - Agent selection logic

4. **Sandbox Execution** (`src/sandbox/`)
   - Docker container management
   - Git worktree creation
   - Network restrictions
   - Resource limits

5. **Git Operations** (`src/git/`)
   - Branch creation and management
   - Worktree operations
   - Commit packaging

### Key Design Decisions

- Tasks execute in isolated Docker containers with restricted network access
- Each task creates a dedicated git branch for review
- Agents run autonomously without interactive input
- All changes must pass through human review before merging

### Security Considerations

- Implement strict Docker container isolation
- Limit network access to AI API endpoints only
- No access to host filesystem outside worktree
- Resource limits on containers (memory, CPU, time)
- No access to SSH keys, Docker socket, or system files

### Error Handling

- Tasks should fail gracefully and always produce a branch
- Capture all agent output for debugging
- Implement comprehensive logging with RUST_LOG support
- Timeout handling for long-running tasks

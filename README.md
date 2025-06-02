# tsk - Autonomous AI Agent Task Manager

A Rust-based CLI tool for delegating development tasks to AI agents running in sandboxed environments. TSK enables you to work with AI coding tools like Aider and Claude Code as if you're managing a team of engineers - assign tasks, get back git branches, and maintain human oversight through familiar code review workflows.

## Overview

TSK provides a "lead engineer + AI team" workflow by:
- Delegating specific development tasks to autonomous AI agents
- Running agents in isolated Docker containers for safe execution
- Creating dedicated git branches for each task
- Enabling human review and approval through standard git workflows
- Supporting iterative refinement when tasks need improvement

Think of it as having a team of engineers who work autonomously but always submit their work for review before merging.

## Key Benefits

### Safe Autonomous Execution
- AI agents run in sandboxed Docker containers with network and filesystem restrictions
- Agents can modify code, run tests, and install dependencies without affecting your main environment
- Every task produces a reviewable git branch, regardless of success or failure

### Human-in-the-Loop Design
- You maintain control as the "lead engineer" reviewing all changes
- Failed or incomplete tasks become learning opportunities for better task descriptions
- Familiar git-based review workflow using tools you already know
- Natural iteration cycle: review → refine instructions → retry

### Agent Tool Integration
- Leverages AI coding tools (Aider, Claude Code)
- Supports headless/batch execution of existing tools
- Extensible architecture for adding new AI agents

## Quick Start

```bash
# Delegate a generic task (no type needed)
tsk add --name "auth-review" \
  --description "Review the authentication module for security vulnerabilities, focus on input validation and session management"

# Add a feature task (using the feature template)
tsk add --name "add-notifications" --type "feature" \
  --description "Add email notifications when users register"

# Run all queued tasks (each creates a branch)
tsk run

# Review the results
tsk list
# Output:
# ✓ auth-review → branch: tsk/auth-review-20240531-143022
# ⚠ extract-service → branch: tsk/extract-service-20240531-143055 (tests failed)

# Review the auth changes
git checkout tsk/auth-review-20240531-143022
git diff main...HEAD

# Looks good! Merge it
git checkout main
git merge tsk/auth-review-20240531-143022

# The refactoring needs work - refine and retry
tsk add --name "extract-service-v2" \
  --description "Extract user notification logic into NotificationService class. Keep existing public API unchanged and ensure all tests pass."

# Or create a task with interactive editing
tsk add --name "complex-feature" --type "feature" --edit
# This will open your $EDITOR to create detailed instructions
```

## Command Reference

### `tsk add`
Queues a task for autonomous execution by an AI agent.

```bash
tsk add --name <TASK_NAME> [--type <TASK_TYPE>] --description <DESCRIPTION>
```

**Options:**
- `--name, -n`: Unique identifier for the task
- `--type, -t`: Task type (optional, defaults to 'generic'). Must have a corresponding template in the templates/ folder
- `--description, -d`: Detailed description of what needs to be accomplished
- `--instructions, -i`: Path to instructions file to pass to the agent (alternative to --description)
- `--edit, -e`: Open the instructions file in $EDITOR after creation for interactive editing
- `--agent, -a`: Specific agent to use (aider, claude-code)
- `--timeout`: Task timeout in minutes (default: 30)

**Note:** If neither `--description` nor `--instructions` is provided, you must use `--edit` to create instructions interactively.

**Task Types:**
Task types are determined by available templates in the `templates/` folder. By default, the following template is provided:
- `feature`: Template for implementing new functionality

To add custom task types, create new template files in the `templates/` folder (e.g., `templates/bug-fix.md`)

### `tsk run`
Executes all queued tasks, creating git branches for review.

```bash
tsk run [OPTIONS]
```

**Options:**
- `--parallel, -p`: Number of concurrent tasks (default: 1)
- `--timeout, -t`: Task timeout in minutes (default: 30)
- `--dry-run`: Preview execution plan without running tasks

### `tsk list`
Displays all tasks with their current status and resulting branches.

```bash
tsk list [OPTIONS]
```

**Options:**
- `--status, -s`: Filter by status (pending|running|completed|failed)

**Output Example:**
```
Task Status Report
==================

✓ auth-review (completed 2m ago)
  Branch: tsk/auth-review-20240531-143022
  Agent: aider-gpt4
  Files: 3 modified, 1 added
  Tests: ✓ passing

⚠ extract-service (completed 5m ago)
  Branch: tsk/extract-service-20240531-143055
  Agent: aider-gpt4
  Files: 8 modified, 2 added
  Tests: ✗ 2 failing

⏳ add-logging (running for 3m)
  Agent: claude-code
  Progress: Analyzing codebase structure...

📋 optimize-queries (pending)
  Queued: 10m ago
```

### `tsk quick`
Immediately executes a task and creates a branch for review.

```bash
tsk quick [--type <TASK_TYPE>] --description <DESCRIPTION>
```

**Options:**
- `--name, -n`: Unique identifier for the task
- `--type, -t`: Task type (optional, defaults to 'generic'). Must have a corresponding template in the templates/ folder
- `--description, -d`: Task description
- `--instructions, -i`: Path to instructions file to pass to the agent (alternative to --description)
- `--edit, -e`: Open the instructions file in $EDITOR after creation for interactive editing
- `--agent, -a`: Specific agent to use (aider|claude-code)
- `--timeout`: Task timeout in minutes (default: 30)

**Note:** If neither `--description` nor `--instructions` is provided, you must use `--edit` to create instructions interactively.

### `tsk tasks`
Manages tasks in the task list, allowing deletion of specific tasks or cleanup of completed tasks.

```bash
tsk tasks [OPTIONS]
```

**Options:**
- `--delete, -d <TASK_ID>`: Delete a specific task by ID
- `--clean, -c`: Delete all completed tasks and all quick tasks

**Examples:**
```bash
# Delete a specific task
tsk tasks --delete 2024-06-01-1430-auth-review

# Clean up completed tasks and quick tasks
tsk tasks --clean
```

### `tsk server` (Planned)
Starts the TSK daemon for scheduled and background task execution.

```bash
tsk server [OPTIONS]
```

**Options:**
- `--port, -p`: Server port (default: 8080)
- `--config, -c`: Configuration file path
- `--workers, -w`: Number of worker processes

## Architecture

### Execution Flow

1. **Task Queuing**: User defines task with type and detailed description
2. **Environment Setup**: Copy repository and create isolated Docker container
3. **Agent Execution**: Selected AI agent (Aider/Claude Code) runs autonomously
4. **Result Capture**: All changes committed to a dedicated task branch
5. **Quality Checks**: Automated tests, linting, and compilation validation
6. **Human Review**: Developer reviews branch using standard git tools
7. **Integration**: Merge acceptable changes, refine and retry others

### Agent Integration

TSK acts as an orchestration layer for existing AI coding tools

### Sandboxing and Security

**Docker Isolation**
- Each task runs in a separate container with minimal privileges
- Network access restricted to AI API endpoints only
- File system access limited to the git worktree
- Resource limits prevent runaway processes

**Network Security**
```bash
# Example Docker network restrictions
docker run --network=ai-restricted \
  --cap-drop=ALL \
  --read-only \
  --tmpfs /tmp \
  --memory=2g \
  --cpus=1.0
```

**File System Boundaries**
- Agents cannot access host filesystem or other projects
- No access to Docker socket, SSH keys, or system files
- All modifications contained within the copied repository

### Branch Management

**Naming Convention**
```
task/{task-name}-{attempt}-{timestamp}
```

## Best Practices

### Writing Effective Task Descriptions

**Good Task Description:**
```bash
tsk add --name "add-rate-limiting" \
  --description "Add rate limiting to the login endpoint. Use a sliding window approach with Redis backend. Limit to 5 attempts per minute per IP address. Return 429 status code when limit exceeded. Add configuration options for limits and window size."
```

**Poor Task Description:**
```bash
tsk add --name "security" \
  --description "Make it more secure"
```

### Task Sizing Guidelines

**Good Task Size:**
- Single responsibility or feature
- Can be completed in 15-30 minutes
- Clear success criteria
- Isolated changes that don't affect multiple systems

**Too Large:**
- "Rewrite the entire authentication system"
- "Add comprehensive logging everywhere"
- "Refactor the database layer"

**Too Small:**
- "Fix typo in comment"
- "Add single line of logging"
- "Rename one variable"

### Review Workflow

1. **Quick Assessment**: Check `task-summary.md` for agent's self-evaluation
2. **Automated Checks**: Review `automated-checks.txt` for test/lint results
3. **Code Review**: Use standard git diff tools to examine changes
4. **Decision**: Merge, manually adjust, or create refined task

## Development Roadmap

### Phase 1: Core Implementation
- [ ] Basic CLI framework and task management
- [ ] Docker sandbox integration
- [ ] Git repository copying and management
- [ ] Claude Code integration for autonomous execution
- [ ] Branch creation and result packaging

### Phase 2: Enhanced Agent Support
- [ ] Improved error handling and recovery
- [ ] Parallel task execution
- [ ] Enhanced result validation

### Phase 3: Workflow Optimization
- [ ] Task templates and quick commands
- [ ] Branch cleanup automation
- [ ] Integration with git hooks
- [ ] Performance optimization
- [ ] Comprehensive logging and monitoring

### Phase 4: Server Mode
- [ ] Background daemon process
- [ ] Task scheduling and queuing
- [ ] Web dashboard for task monitoring
- [ ] API for external integrations
- [ ] Multi-project support

### Phase 5: Advanced Features
- [ ] Task dependency management
- [ ] Custom agent development
- [ ] Integration with CI/CD pipelines
- [ ] Team collaboration features
- [ ] Advanced security policies

## Contributing

### Development Setup
```bash
cargo build
cargo test
```


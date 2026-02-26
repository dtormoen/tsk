---
name: tsk-help
description: Use this skill when the user asks about tsk commands, delegating development tasks to AI agents, managing sandboxed task execution, or working with the tsk task queue and server.
allowed-tools:
  - Bash(tsk help)
---

# tsk

`tsk` delegates development tasks to AI agents (Claude Code, Codex) running in isolated Docker/Podman containers. Agents work autonomously and produce git branches for review.

## Core Commands

### Run a task immediately

```bash
tsk run --type feat --name add-auth --prompt "Add JWT authentication to the API"
```

Key flags:
- `--type` (`-t`): Task template (feat, fix, doc, refactor, or custom templates)
- `--name` (`-n`): Human-readable name used in the git branch
- `--prompt` (`-p`): Task prompt (replaces `{{PROMPT}}` in templates)
- `--agent`: Agent to use (claude, codex). Default: claude
- `--edit`: Open the full prompt in your editor before sending

### Interactive sandbox

```bash
tsk shell
```

Drops you into a container with your repo copy and agent installed. Work interactively, then exit to save changes as a branch.

### Queue tasks for background execution

```bash
# Start the server with parallel workers
tsk server start --workers 4

# Queue tasks
tsk add -t feat -n user-api -p "Add REST API for user management"
tsk add -t fix -n login-bug -p "Fix session timeout on login page"

# Check status
tsk list
```

### Chain tasks

Child tasks start from where the parent left off:

```bash
tsk add -t feat -n add-api -p "Add users API endpoint"
tsk list  # get the task ID
tsk add -t feat -n add-tests -p "Add tests for users API" --parent <taskid>
```

### Other commands

This is the output of `tsk help`

!`tsk help`

## How it works

1. `tsk` copies your repository (including uncommitted changes) into an isolated directory
2. A Docker/Podman container is started with your language stack and agent installed
3. Network access is restricted to approved API domains via a Squid proxy
4. The agent executes the task autonomously
5. Changes are fetched back as a new branch: `tsk/{type}/{name}/{id}`

## Multi-agent execution

Run the same task with multiple agents to compare results:

```bash
tsk add -t feat -n greeting --agent codex,claude -p "Add a greeting feature"
```

## Custom templates

Create templates in `~/.config/tsk/templates/` or `.tsk/templates/`:

```bash
mkdir -p ~/.config/tsk/templates
cat > ~/.config/tsk/templates/issue-bot.md << 'EOF'
Solve the GitHub issue below. Write tests and a descriptive commit message.

{{PROMPT}}
EOF

# Use it
gh issue view 42 | tsk add -t issue-bot -n fix-issue-42
```

## Configuration

`tsk` is configured via `~/.config/tsk/tsk.toml`. Key settings:

```toml
container_engine = "docker"  # or "podman"

[defaults]
memory_gb = 12.0
cpu = 8

[server]
auto_clean_enabled = true
auto_clean_age_days = 7.0

[project.my-project]
agent = "claude"
stack = "go"
```

See `tsk help` or `tsk help <command>` for full option details.

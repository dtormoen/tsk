# TSK - AI Agent Task Manager

A Rust CLI tool that lets you delegate development tasks to AI agents running in sandboxed Docker or Podman environments. Get back git branches for human review.

Currently Claude Code and Codex coding agents are supported.

![TSK Demo](./docs/images/tsk-demo.gif)

## Overview

TSK enables a "lead engineer + AI team" workflow:
1. **Assign tasks** to AI agents using task type templates to automate prompt boilerplate and enable powerful multi-agent workflows
2. **Agents work autonomously** in parallel isolated containers with file system and network isolation
3. **Get git branches** back with their changes for review
4. **Review and merge** using your normal git workflow

Think of it as having a team of engineers who work independently and submit pull requests for review.

## Installation

### Requirements

- [Rust](https://rustup.rs/) - Rust toolchain and Cargo
- [Docker](https://docs.docker.com/get-docker/) or [Podman](https://podman.io/) - Container runtime
- [Git](https://git-scm.com/downloads) - Version control system
- One of the supported coding agents:
  - [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
  - [Codex](https://openai.com/codex/)
  - Help us support more!

### Install TSK

```bash
# Install using cargo
cargo install tsk-ai
# Or build from source!
gh repo clone dtormoen/tsk
cd tsk
cargo install .
```

## Quick Start Guide

TSK can be used in multiple ways. Here are some of the main workflows to get started. Try testing these in the TSK repository!

### Interactive Sandboxes

Start up sandbox with an interactive shell so you can work interactively with a coding agent. This is similar to a git worktrees workflow, but provides stronger isolation. `claude` is the default coding agent, but you can also specify `--agent codex` to use `codex`.

```bash
tsk shell
```

The `tsk shell` command will:
- Make a copy of your repo
- Create a new git branch for you to work on
- Start a proxy to limit internet access
- Build and start a container with your stack (go, python, rust, etc.) and agent (default: claude) installed
- Drop you into an interactive shell

After you exit the interactive shell (ctrl-d or `exit`), TSK will save any work you've done as a new branch in your original repo.

This workflow is really powerful when used with terminal multiplexers like `tmux` or `zellij`. It allows you to start multiple agents that are working on completely isolated copies of your repository with no opportunity to interfere with each other or access resources outside of the container.

### One-off Fully Autonomous Agent Sandboxes

TSK has flags that help you avoid repetitive instructions like "make sure unit tests pass", "update documentation", or "write a descriptive commit message". Consider this command which immediately kicks off an autonomous agent in a sandbox to implement a new feature:

```bash
tsk run --type feat --name greeting --description "Add a greeting to all TSK commands."
```

Some important parts of the command:
- `--type` specifies the type of task the agent is working on. Using TSK built-in tasks or writing your own can save a lot of boilerplate. Check out [feat.md](./templates/feat.md) for the `feat` type and [templates](./templates) for all task types.
- `--name` will be used in the final git branch to help you remember what task the branch contains.
- `--description` is used to fill in the `{{description}}` placeholder in [feat.md](./templates/feat.md).

Similar to `tsk shell`, the agent will run in a sandbox so it will not interfere with any ongoing work and will create a new branch in your repository in the background once it is done working.

After you try this command out, try out these next steps:
- Add the `--edit` flag to edit the full prompt that is sent to the agent.
- Add a custom task type. Use `tsk template list` to see existing task templates and where you can add your own custom tasks.
  - See the [custom templates used by TSK](./.tsk/templates) for inspiration.

### Queuing Tasks for Parallel Execution

The TSK server allows you to have a single process that manages parallel task execution so you can easily background agents working. First, we start the server set up to handle up to 4 tasks in parallel:

```bash
tsk server start --workers 4
```

Now, in another terminal window, we can quickly queue up multiple tasks:

```bash
# Add a task. Notice the similarity to the `tsk run` command
tsk add --type doc --name tsk-architecture --description "Tell me how TSK works"

# Look at the task queue. Your task `tsk-architecture` should be present in the list
tsk list

# Add another task. Notice the short flag names
tsk add -t feat -n greeting -d "Add a silly robot greeting to every TSK command"

# Now there should be two running tasks
tsk list

# Wait for the tasks to finish. After they complete, look at the two new branches
git branch --format="%(refname:short) - %(subject) (%(committerdate:relative))"
```

After you try this command out, try these next steps:
- Add tasks from multiple repositories in parallel
- Start up multiple agents at once
  - Adding `--agent codex` will use `codex` to perform the task
  - Adding `--agent codex,claude` will have `codex` and `claude` do the task in parallel with the same environment and instructions so you can compare agent performance
  - Adding `--agent claude,claude` will have `claude` do the task twice. This can be useful for exploratory changes to get ideas quickly

### Task Chaining

Chain tasks together with `--parent` (`-p`) so a child task starts from where its parent left off:

```bash
# First task: set up the foundation
tsk add -t feat -n add-api -d "Add a REST API endpoint for users"

# Check the task list to get the task ID
tsk list

# Second task: chain it to the first (replace <taskid> with the parent's ID)
tsk add -t feat -n add-tests -d "Add integration tests for the users API" --parent <taskid>
```

Child tasks wait for their parent to complete, then start from the parent's final commit. `tsk list` shows these tasks as `WAITING`. If a parent fails, its children are automatically marked as `FAILED`. Chains of any length (A → B → C) are supported.

### Create a Simple Task Template

Let's create a very basic way to automate working on GitHub issues:

```bash
# First create the tsk template configuration directory
mkdir -p ~/.config/tsk/templates

# Create a very simple template. Notice the use of the "{{DESCRIPTION}}" placeholder
cat > ~/.config/tsk/templates/issue-bot.md << 'EOF'
Solve the GitHub issue below. Make sure it is tested and write a descriptive commit
message describing the changes after you are done.

{{DESCRIPTION}}
EOF

# Make sure tsk sees the new `issue-bot` task template
tsk template list

# Pipe in some input to start the task
# Piped input automatically replaces the {{DESCRIPTION}} placeholder
gh issue view <issue-number> | tsk add -t issue-bot -n fix-my-issue
```

Now it's easy to solve GitHub issues with a simple task template. Try this with code reviews as well to easily respond to feedback.

## Commands

### Task Commands

Create, manage, and monitor tasks assigned to AI agents.

- `tsk run` - Execute a task immediately
- `tsk shell` - Start a sandbox container with an interactive shell
- `tsk add` - Queue a task (supports `--parent <taskid>` for task chaining)
- `tsk list` - View task status and branches
- `tsk clean` - Clean up completed tasks
- `tsk delete <task-id>...` - Delete one or more tasks
- `tsk retry <task-id>...` - Retry one or more tasks

### Server Commands

Manage the TSK server daemon for parallel task execution. The server automatically cleans up completed and failed tasks older than 7 days.

- `tsk server start` - Start the TSK server daemon
- `tsk server stop` - Stop the running TSK server

### Configuration Commands

Build container images and manage task templates.

- `tsk docker build` - Build required container images
- `tsk template list` - View available task type templates and where they are installed

Run `tsk help` or `tsk help <command>` for detailed options.

## Configuring TSK

TSK has 3 levels of configuration in priority order:
- Project level in the `.tsk` folder local to your project
- User level in `~/.config/tsk`
- Built-in configurations

Each configuration directory can contain:
- `dockerfiles`: A folder containing dockerfiles and layers that are used to create sandboxes
- `templates`: A folder of task template markdown files which can be used via the `-t/--type` flag

### Configuration File

TSK can be configured via `~/.config/tsk/tsk.toml`. All settings are optional.

```toml
# Container engine and resource limits
[docker]
container_engine = "docker"  # "docker" (default) or "podman"
memory_limit_gb = 12.0       # Container memory limit (default: 12.0)
cpu_limit = 8                # Number of CPUs (default: 8)

# Proxy configuration
[proxy]
# Forward ports from containers to host services (agents connect to tsk-proxy:<port>)
# Default: [] (no port forwarding)
host_services = [5432, 6379]  # e.g., PostgreSQL, Redis

# Git-town integration (https://git-town.com/)
# When enabled, task branches automatically record their parent branch
[git_town]
enabled = true  # default: false

# Server daemon configuration
[server]
auto_clean_enabled = true   # Automatically clean old tasks (default: true)
auto_clean_age_days = 7.0   # Minimum age in days before cleanup (default: 7.0)

# Project-specific configuration (matches directory name)
[project.my-project]
agent = "claude"        # Default agent (claude or codex)
stack = "go"            # Default stack for auto-detection override
volumes = [
    # Bind mount: Share host directories with containers (supports ~ expansion)
    { host = "~/.cache/go-mod", container = "/go/pkg/mod" },
    # Named volume: Container-managed persistent storage (prefixed with tsk-)
    { name = "go-build-cache", container = "/home/agent/.cache/go-build" },
    # Read-only mount: Provide artifacts without modification risk
    { host = "~/debug-logs", container = "/debug-logs", readonly = true }
]
env = [
    # Environment variables passed to the container
    { name = "DATABASE_URL", value = "postgres://tsk-proxy:5432/mydb" },
    { name = "REDIS_URL", value = "redis://tsk-proxy:6379" },
]
```

Volume mounts are particularly useful for:
- **Build caches**: Share Go module cache (`/go/pkg/mod`) or Rust target directories to speed up builds
- **Persistent state**: Use named volumes for build caches that persist across tasks
- **Read-only artifacts**: Mount debugging artifacts, config files, or other resources without risk of modification

Environment variables (`env`) let you pass configuration to task containers, such as database URLs or API keys. Use `tsk-proxy:<port>` to connect to host services forwarded through the proxy.

The container engine can also be set per-command with the `--container-engine` flag (available on `run`, `shell`, `retry`, `server start`, and `docker build`).

The `[proxy]` section lets you expose host services to task containers. Agents connect to `tsk-proxy:<port>` to reach services running on your host machine (e.g., local databases or dev servers).

The `[git_town]` section enables integration with [git-town](https://www.git-town.com/), a tool for branch-based workflow automation. When enabled, TSK sets the parent branch metadata on task branches, allowing git-town commands like `git town sync` to work correctly with TSK-created branches.

Configuration priority: CLI flags > project config > auto-detection > defaults

### Customizing the TSK Sandbox Environment

Each TSK sandbox container image has 4 main parts:
- A [base dockerfile](./dockerfiles/base/default.dockerfile) that includes the OS and a set of basic development tools e.g. `git`
- A `stack` snippet that defines language specific build steps. See:
  - [default](./dockerfiles/stack/default.dockerfile) - minimal fallback stack
  - [go](./dockerfiles/stack/go.dockerfile)
  - [java](./dockerfiles/stack/java.dockerfile)
  - [lua](./dockerfiles/stack/lua.dockerfile)
  - [node](./dockerfiles/stack/node.dockerfile)
  - [python](./dockerfiles/stack/python.dockerfile)
  - [rust](./dockerfiles/stack/rust.dockerfile)
- An `agent` snippet that installs an agent, e.g. `claude` or `codex`.
- A `project` snippet that defines project specific build steps (applied last for project-specific customizations). This does nothing by default, but can be used to add extra build steps for your project.

It is very difficult to make these images general purpose enough to cover all repositories. You may need some special customization. See [dockerfiles](./dockerfiles) for the built-in dockerfiles as well as the [TSK custom project layer](./.tsk/dockerfiles/project/tsk.dockerfile) to see how you can integrate custom build steps into your project by creating a `.tsk/dockerfiles/project/<yourproject>.dockerfile` or `~/.config/tsk/dockerfiles/project/<yourproject>.dockerfile` snippet.

You can run `tsk docker build --dry-run` to see the dockerfile that `tsk` will dynamically generate for your repository. You can also run `tsk run --type tech-stack` or `tsk run --type project-layer` to try to generate a `stack` or `project` snippet for your project, but this has not been heavily tested.

See the [Docker Builds Guide](docs/docker-builds.md) for a more in-depth walk through, and the [Network Isolation Guide](docs/network-isolation.md) for details on how TSK secures agent network access.

I'm working on improving this part of `tsk` to be as seamless and easy to set up as possible, but it's still a work in progress. I welcome all feedback on how to make this easier and more intuitive!

### Creating Templates

Templates are simply markdown files that get passed to agents. TSK additionally adds a convenience `{{description}}` placeholder that will get replaced by anything you pipe into tsk or pass in via the `-d/--description` flag.

To create good templates, I would recommend thinking about repetitive tasks that you need agents to do within your codebase like "make sure the unit tests pass", "write a commit message", etc. and encode those in a template file. There are many great prompting guides out there so I'll spare the details here.

### Custom Proxy Configuration

TSK uses Squid as a forward proxy to control network access from task containers. If you want to customize the proxy configuration e.g. to allow access to a specific service or allow a URL for downloading specific dependencies of your project, you can create a `squid.conf` file in the user level configuration directory, usually `~/.config/tsk`. Look at the default [TSK squid.conf](./dockerfiles/tsk-proxy/squid.conf) as an example.

## TSK Data Directory

TSK uses the following directories for storing data while running tasks:
- **~/.local/share/tsk/tasks.db**: SQLite database for task queue and task definitions
- **~/.local/share/tsk/tasks/**: Task directories that get mounted into sandboxes when the agent runs. They contain:
  - **<taskid>/repo**: The repo copy that the agent operates on
  - **<taskid>/output**: Directory containing a log file with the agent's actions
  - **<taskid>/instructions.md**: The instructions that were passed to an agent

## Contributing

This project uses:
- `cargo test` for running tests
- `just precommit` for full CI checks
- See [CLAUDE.md](CLAUDE.md) for development guidelines

## License

MIT License - see [LICENSE](LICENSE) file for details.

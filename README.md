# TSK - AI Agent Task Manager

A Rust CLI tool that lets you delegate development tasks to AI agents running in sandboxed Docker environments. Get back git branches for human review.

Currently Claude Code and Codex coding agents are supported.

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

## Quick Start

TSK can be used in multiple ways. Here are some of the main workflows to get started.

### Interactive Sandboxes

```sh
tsk shell
```

After the shell starts, fire up your coding agent! `claude` is the default, but you can also use `codex` by adding `--agent codex`.

The `tsk shell` command will:
- Make a copy of your repo
- Create a new git branch for you to work on
- Start a proxy to limit internet access
- Build and start a docker container with your stack (go, python, rust, etc.) and agent (default: claude) installed
- Drop you into an interactive shell

After you exit the interactive shell (ctrl-d or `exit`), TSK will save any work you've done as a new branch in your original repo.

This workflow is really powerful when used with terminal multiplexers like `tmux` or `zellij`. It allows you to start multiple agents that are working on completely isolated copies of your repository with no opportunity to interfere with each other or access resources outside of the container.

### One-off Fully Autonomous Agent Sandboxes

```sh
tsk run --type feat --name my-feature --description "Implement a fancy new feature for TSK."
```

This allows you to quickly launch a fully autonomous agent to implement a new feature. Similar to `tsk shell`, the agent will run in a sandbox so it will not interfere with any ongoing work and will create a new branch in your repository in the background once it is done working. The command will run synchronously until the agent is done working.

Some important parts of the command:
- `--type` specifies the type of task the agent is working on. Using TSK built-in tasks or writing your own can save a lot of boilerplate. Check out [feat.md](./templates/feat.md) for the `feat` type and [templates](./templates) for all task types.
- `--name` will be used in the final git branch to help you remember what task the branch contains.
- `--description` is used to fill in a `{{description}}` placeholder in [feat.md](./templates/feat.md).

After you try this command out, try out these next steps:
- Add the `--edit` flag to edit the full prompt that is sent to the agent.
- Add a custom task type. Use `tsk template list` to see existing task templates and where you can add your own custom tasks.
  - See the [custom templates used by TSK](./.tsk/templates) for inspiration.

### Queuing Tasks for Parallel Execution

```sh
# In a separate terminal window, start the TSK server
tsk server start --workers 4
```

This is where TSK starts to get really powerful. This command starts a server that will handle up to 4 tasks in parallel. Now that the server is running, you can add tasks to it:

```sh
# Add a task. Notice the similarity to the `tsk run` command. The options discussed there work here too
tsk add --type feat --name my-feature --description "Implement a fancy new feature for TSK."

# Look at the task queue. Your task `my-feature` should be present in the list and RUNNING
tsk list

# Add another task. Notice the short flag names
tsk add -t feat -n greeting -d "Make TSK print a silly robot greeting every time a user runs a command."

# Now there should be two running tasks
tsk list

# Wait for the tasks to finish. After they complete, look at the two new branches
git branch --format="%(refname:short) - %(objectname:short) - %(subject) (%(committerdate:relative))"
```

After you try this command out, try these next steps:
- Add tasks from multiple repositories in parallel
- Start up multiple agents at once
  - Adding `--agent codex` will use `codex` to perform the task
  - Adding `--agent codex,claude` will have `codex` and `claude` do the task in parallel with the same environment and instructions so you can compare agent performance
  - Adding `--agent claude,claude` will have `claude` do the task twice. This can be useful for exploratory changes to get ideas quickly

### Create a Simple Task Template

Let's create a very basic way to automate working on GitHub issues:

```sh
# First create the tsk template configuration directory
mkdir -p .tsk/templates

# Create a very simple template. Notice the use of the "{{DESCRIPTION}}" placeholder
cat > .tsk/templates/issue-bot.md << 'EOF'
Solve the GitHub issue below. Make sure it is tested and write a descriptive commit message describing the changes after you are done.

{{DESCRIPTION}}
EOF

# Make sure tsk sees the new `issue-bot` task template
tsk template list

# Pipe in some input to start the task. Piped input automatically replaces the {{DESCRIPTION}} placeholder
gh issue view <some GitHub issue number> | tsk add -t issue-bot -n fix-my-issue
```

Now it's easy to solve GitHub issues with a simple task template. Try this with code reviews as well to easily respond to feedback.

## Configuring TSK

TSK has 3 levels of configuration in priority order:
- Project level in the `.tsk` folder local to your project
- User level in `~/.config/tsk`
- Built-in configurations

Each configuration directory can contain:
- `dockerfiles`: A folder containing dockerfiles and layers that are used to create sandboxes
- `templates`: A folder of task template markdown files which can be used via the `-t/--type` flag

### Customizing the TSK Sandbox Environment

Each TSK sandbox docker image has 4 main parts:
- A [base dockerfile](./dockerfiles/base/default.dockerfile) that includes the OS and a set of basic development tools e.g. `git`
- A `stack` snippet that defines language specific build steps. See:
  - [go](./dockerfiles/stack/go.dockerfile)
  - [java](./dockerfiles/stack/java.dockerfile)
  - [lua](./dockerfiles/stack/lua.dockerfile)
  - [node](./dockerfiles/stack/node.dockerfile)
  - [python](./dockerfiles/stack/python.dockerfile)
  - [rust](./dockerfiles/stack/rust.dockerfile)
- A `project` snippet that defines project specific build steps. This does nothing by default, but can be used to add extra build steps for your project.
- An `agent` snippet that installs an agent, e.g. `claude` or `codex`.

It is very difficult to make these images general purpose enough to cover all repositories. You may need some special customization. See [dockerfiles](./dockerfiles) for the built-in dockerfiles as well as the [TSK custom project layer](./.tsk/dockerfiles/project/tsk.dockerfile) to see how you can integrate custom build steps into your project by creating a `.tsk/dockerfiles/project/<yourproject>.dockerfile` or `~/.config/tsk/dockerfiles/project/<yourproject>.dockerfile` snippet.

You can run `tsk docker build --dry-run` to see the dockerfile that `tsk` will dynamically generate for your repository. You can also run `tsk run --type tech-stack` or `tsk run --type project-layer` to try to generate a `stack` or `project` snippet for your project, but this has not been heavily tested.

See the [Docker Builds Guide](docs/docker-builds.md) for a more in-depth walk through.

I'm working on improving this part of `tsk` to be as seamless and easy to set up as possible, but it's still a work in progress. I welcome all feedback on how to make this easier and more intuitive!

### Creating Templates

Templates are simply markdown files that get passed to agents. TSK additionally adds a convenience `{{description}}` placeholder that will get replaced by anything you pipe into tsk or pass in via the `-d/--description` flag.

To create good templates, I would recommend thinking about repetitive tasks that you need agents to do within your codebase like "make sure the unit tests pass", "write a commit message", etc. and encode those in a template file. There are many great prompting guides out there so I'll spare the details here.

## Commands

### Task Commands
- `tsk run` - Execute a task immediately
- `tsk shell` - Start an interactive sandbox container
- `tsk add` - Queue a task
- `tsk list` - View task status and branches
- `tsk clean` - Clean up completed tasks
- `tsk delete <task-id>...` - Delete one or more tasks
- `tsk retry <task-id>...` - Retry one or more tasks

### Server Commands
- `tsk server start` - Start the TSK server daemon
- `tsk server stop` - Stop the running TSK server

### Configuration Commands
- `tsk docker build` - Build required docker images
- `tsk proxy stop` - Stop the TSK proxy container
- `tsk template list` - View available task type templates and where they are installed

Run `tsk help` or `tsk help <command>` for detailed options.

## Contributing

This project uses:
- `cargo test` for running tests
- `just precommit` for full CI checks
- See [CLAUDE.md](CLAUDE.md) for development guidelines

## License

MIT License - see [LICENSE](LICENSE) file for details.

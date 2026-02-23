---
name: tsk-config
description: Use this skill when the user wants to set up or configure TSK Docker container images, customize their tsk.toml for Docker builds, configure stack/agent/project layers, or troubleshoot TSK container build issues.
user-invocable: true
disable-model-invocation: true
allowed-tools: Bash Read Write Edit Glob Grep AskUserQuestion
---

# TSK Docker Configuration Guide

You are helping a user configure TSK Docker container images for their project. Follow these steps in order.

## Step 1: Gather Project Information

Detect the project's technology stack by checking for these files in the project root. These are TSK's built-in stacks with auto-detection — custom stacks for any language can be defined via `stack_config` in `tsk.toml` (covered in Step 5).

| File | Stack |
|------|-------|
| `Cargo.toml` | rust |
| `go.mod` | go |
| `package.json` | node |
| `pyproject.toml`, `requirements.txt`, `setup.py` | python |
| `pom.xml`, `build.gradle`, `build.gradle.kts` | java |
| `rockspec`, `.luacheckrc`, `init.lua` | lua |
| None of the above | default |

Also determine the project name from the current directory name.

Tell the user what you detected and ask them to confirm or override. If their stack isn't listed above, let them know they can define a custom stack.

## Step 2: Check for Deprecated Dockerfiles

Check if `.tsk/dockerfiles/` exists. If it does, warn the user:

> **Deprecated**: TSK no longer supports filesystem-based dockerfiles in `.tsk/dockerfiles/`. Docker customization is now done via `setup` fields in `tsk.toml`. This guide will help you migrate to the new format.

List any files found in `.tsk/dockerfiles/` and note their contents — you will use them to populate the new config.

## Step 3: Choose Configuration Location

Ask the user where to put the configuration:

- **Project-level** (`.tsk/tsk.toml`): Checked into version control, shared with the team. Best for project-specific dependencies that all contributors need.
- **User-level** (`~/.config/tsk/tsk.toml`): Personal settings, not shared. Best for machine-specific paths, personal preferences, or settings across multiple projects.

If the user picks user-level, config goes under `[project.<project-name>]` in `~/.config/tsk/tsk.toml`. If project-level, config goes at the top level of `.tsk/tsk.toml`.

## Step 4: View the Current Docker Build

Run this command and show the output to the user:

```bash
tsk docker build --dry-run
```

Explain to the user: this shows the complete Dockerfile that TSK generates with all layers resolved. Look for comments like `# Stack layer`, `# Project layer`, and `# Agent layer` to see where each `setup` field injects content. The next step will help them add the right customizations.

## Step 5: Populate Configuration

Based on the project analysis, write the `tsk.toml` configuration. Use the layer reference below to decide what goes where.

### TSK Docker Layer Architecture

TSK builds container images using 4 layers, assembled in this order:

```
1. Base layer    — Ubuntu 25.10, git, build-essential, ripgrep, Python 3, uv, podman
2. Stack layer   — Language runtime and tools (e.g., Go, Rust, Node.js)
3. Project layer — Project-specific system dependencies
4. Agent layer   — AI agent installation (Claude, Codex)
```

You customize layers 2-4 via `tsk.toml`:

| Config field | Layer | Purpose |
|---|---|---|
| `stack_config.<stack>.setup` | Stack (2) | Language tooling, compilers, package managers |
| `setup` | Project (3) | Project-specific apt packages, system libraries, custom tools |
| `agent_config.<agent>.setup` | Agent (4) | Agent-specific setup (rarely needed) |

Each `setup` field contains raw Dockerfile commands (`RUN`, `ENV`, `COPY`, etc.) that get injected into the generated Dockerfile at the corresponding layer position. Setup commands run as the `agent` user by default. Use `USER root` to switch to root for operations that require it (e.g., `apt-get`), and always switch back to `USER agent` afterwards.

### Built-in Stacks (What's Already Included)

TSK has built-in stack layers. You only need `stack_config` if the built-in is insufficient.

**rust**: Rust stable via rustup, `CARGO_TARGET_DIR=/home/agent/.cargo/target`

**go**: Go 1.25.0, `GOPATH=/home/agent/gopath`, `CGO_ENABLED=0`

**node**: Node.js LTS, npm, pnpm, yarn, typescript, ts-node, nodemon, eslint, prettier, jest, npm-check-updates. `NODE_ENV=development`

**python**: uv venv at `/home/agent/.venv`, pytest, pip, black, ruff, ty, mypy, poetry

**java**: OpenJDK 17, Maven, Gradle. Maven `settings.xml` and Gradle `gradle.properties` are pre-configured to route through the TSK proxy

**lua**: LuaJIT, Lua 5.1 dev libs, Neovim, stylua, LuaRocks with luacheck/busted/luassert/luafilesystem/nlua

**default**: Empty (base layer only)

### Base Layer (Always Present)

Every container includes: Ubuntu 25.10, git, build-essential, curl, jq, just, ripgrep, sudo, Python 3, uv, podman (for DIND), and an `agent` user (UID 1000).

### Configuration Format

**Project-level** (`.tsk/tsk.toml`):

```toml
# Project-specific dependencies (injected at the project layer)
setup = '''
USER root
RUN apt-get update && apt-get install -y libssl-dev pkg-config cmake
USER agent
'''

# Override or extend the stack layer
[stack_config.rust]
setup = '''
RUN cargo install cargo-nextest sccache
ENV RUSTC_WRAPPER=sccache
'''
```

**User-level** (`~/.config/tsk/tsk.toml`):

```toml
# Default settings for all projects
[defaults]
memory_gb = 16.0
cpu = 8

# Per-project overrides
[project.my-project]
stack = "rust"
setup = '''
USER root
RUN apt-get update && apt-get install -y libssl-dev pkg-config
USER agent
'''

[project.my-project.stack_config.rust]
setup = '''
RUN cargo install cargo-nextest
'''
```

### Common Examples

**Rust project needing system libraries:**
```toml
setup = '''
USER root
RUN apt-get update && apt-get install -y \
    libssl-dev pkg-config cmake protobuf-compiler
USER agent
'''
```

**Python project with native dependencies:**
```toml
setup = '''
USER root
RUN apt-get update && apt-get install -y \
    libpq-dev libffi-dev
USER agent
'''
```

**Node.js project needing native build tools:**
```toml
setup = '''
USER root
RUN apt-get update && apt-get install -y \
    libcairo2-dev libjpeg-dev libpango1.0-dev libgif-dev
USER agent
'''
```

**Go project with CGO and protocol buffers:**
```toml
[stack_config.go]
setup = '''
USER root
ENV CGO_ENABLED=1
RUN apt-get update && apt-get install -y protobuf-compiler
USER agent
RUN go install google.golang.org/protobuf/cmd/protoc-gen-go@latest
'''
```

**Java project with a specific JDK version:**
```toml
[stack_config.java]
setup = '''
USER root
RUN apt-get update && apt-get install -y openjdk-21-jdk
USER agent
ENV JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64
'''
```

**Adding a stack not built into TSK (e.g., Scala):**
```toml
stack = "scala"

[stack_config.scala]
setup = '''
USER root
RUN apt-get update && apt-get install -y openjdk-21-jdk
RUN curl -fL https://github.com/coursier/coursier/releases/latest/download/cs-x86_64-pc-linux.gz | gzip -d > /usr/local/bin/cs \
    && chmod +x /usr/local/bin/cs \
    && cs setup --yes
USER agent
ENV JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64
'''
```

### Writing Good Setup Commands

- Setup commands run as the `agent` user (UID 1000) by default. Use `USER root` when you need elevated privileges (e.g., `apt-get install`), and always switch back to `USER agent` afterwards
- Always combine `apt-get update` with `apt-get install` in the same `RUN` to avoid stale caches
- Use `-y` flag for non-interactive installs
- Combine related commands with `&&` to reduce layers
- Environment variables set with `ENV` persist into the running container
- The project layer runs BEFORE the agent layer, so project setup cannot depend on agent tools

### Deciding What Goes Where

Ask yourself:
- **Is it a language runtime or dev tool?** → `stack_config.<stack>.setup`
- **Is it a system library or project dependency?** → `setup` (project layer)
- **Does it modify how the AI agent works?** → `agent_config.<agent>.setup` (rare)
- **Would it be useful for other projects with the same stack?** → `stack_config` in user-level `[defaults]`
- **Is it specific to this project?** → `setup` in `.tsk/tsk.toml`

Write the configuration file based on the project's needs. If there were deprecated dockerfiles found in step 2, migrate their contents to the appropriate setup fields.

If the user does not have specific customization needs yet, write a minimal config with just the `stack` field set and an empty `setup` with a comment explaining where to add dependencies.

## Step 6: Test the Configuration

Run this command to verify the Docker image builds successfully and tasks work:

```bash
tsk run -t ack
```

This creates a minimal task that just says "ack" and exits. If it completes successfully, the Docker configuration is working. If it fails, examine the error output:

- **Build failures**: Usually a bad `RUN` command in a setup field. Check the Dockerfile output for the failing line.
- **Missing dependencies at runtime**: The setup command succeeded but the tool isn't available. Check `PATH` and `ENV` settings.
- **Permission errors**: May need `USER root` before install commands.

If the build fails, help the user fix the configuration and re-run `tsk run -t ack`.

If the user is stuck and cannot get their project working after multiple attempts, ask if they'd like to file a bug report against TSK. If they agree:

1. Capture the output of `tsk docker build --dry-run`
2. Capture the error message from the failing build or run
3. Ask the user to briefly describe what aspect of their project is causing problems (e.g., specific system libraries, unusual toolchain, network requirements)
4. File the issue using `gh`:

```bash
gh issue create --repo dtormoen/tsk \
  --title "Docker build failure: <short description>" \
  --body "$(cat <<'EOF'
## Problem

<user's description of what aspect of their project causes problems>

## Error

```
<error output>
```

## Generated Dockerfile (`tsk docker build --dry-run`)

```dockerfile
<dry-run output>
```
EOF
)"
```

## Additional Configuration Options

After the basic setup works, mention these optional settings the user might want:

```toml
# Forward host ports to containers (e.g., local databases)
host_ports = [5432, 6379]

# Mount volumes for build caches
volumes = [
    { host = "~/.cache/go-mod", container = "/go/pkg/mod" },
    { name = "build-cache", container = "/home/agent/.cache" },
]

# Pass environment variables to containers
env = [
    { name = "DB_PORT", value = "5432" },
]

# Container resources
memory_gb = 16.0
cpu = 8

# Enable Docker-in-Docker
dind = true
```

Note: To connect to host services via forwarded ports, use the `TSK_PROXY_HOST` environment variable (set automatically by TSK) as the hostname, not `localhost`.

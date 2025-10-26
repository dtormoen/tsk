# Docker Builds in TSK

TSK uses a sophisticated Docker-based execution environment to run AI agents in isolated, secure containers. This document explains how the Docker build system works, how to customize it, and how to debug common issues.

## Table of Contents

- [Overview](#overview)
- [The Four-Layer Architecture](#the-four-layer-architecture)
- [Docker Image Naming](#docker-image-naming)
- [Customizing Docker Images](#customizing-docker-images)
- [Building Docker Images](#building-docker-images)
- [Tech Stack Auto-Detection](#tech-stack-auto-detection)
- [Security and Isolation](#security-and-isolation)
- [Debugging Docker Issues](#debugging-docker-issues)
- [Common Patterns and Examples](#common-patterns-and-examples)

## Overview

TSK's Docker infrastructure provides:
- **Layered architecture** for efficient image building and caching
- **Automatic tech stack detection** based on repository files
- **Multiple customization points** at project and user levels
- **Security-first design** with non-root execution and network isolation
- **Intelligent fallback** mechanisms for missing configurations

## The Four-Layer Architecture

TSK composes Docker images from four distinct layers, each serving a specific purpose. All base dockerfiles are embedded as assets within the TSK binary and are automatically extracted when needed, ensuring TSK works out-of-the-box without requiring separate configuration files.

### 1. Base Layer
The foundation of all TSK containers (`base/default.dockerfile`):
- Ubuntu 24.04 base operating system
- Essential development tools (git, curl, build-essential, ripgrep, etc.)
- Non-root `agent` user (created by renaming the default `ubuntu` user) for security
- Git configuration inherited from host user via build arguments
- Working directory set to `/workspace`
- Contains placeholders (`{{{STACK}}}`, `{{{PROJECT}}}`, `{{{AGENT}}}`) for layer composition

### 2. Stack Layer
Language-specific toolchains and runtimes:
- **default**: Minimal additions, used as fallback
- **rust**: Rust toolchain via rustup with Cargo and Just
- **python**: Python 3 with uv package manager, pytest, black, ruff, mypy, and poetry
- **node**: Node.js LTS with npm, pnpm, yarn, TypeScript, ESLint, and Jest
- **go**: Go 1.25.0 with gopls, delve debugger, goimports, and staticcheck
- **java**: OpenJDK 17 with Maven, Gradle, Kotlin, Groovy, and SDKMAN
- **lua**: Neovim with LuaJIT, Luarocks, luacheck, busted, and stylua

### 3. Agent Layer
AI agent installations and configurations:
- **claude**: Claude Code CLI (installed via npm with Node.js 20.x)
- **codex**: Codex CLI (installed via npm with Node.js 20.x)
- **no-op**: Testing/debugging agent (displays instructions without executing)

Agent versions are automatically detected and tracked via the `TSK_AGENT_VERSION` build argument, triggering image rebuilds when agents are upgraded on the host system.

### 4. Project Layer
Project-specific dependencies and optimizations:
- **default**: Empty layer, used as fallback
- **Custom layers**: Created per-project for dependency caching

## Docker Image Naming

TSK uses a hierarchical naming convention for Docker images:

```
tsk/{stack}/{agent}/{project}
```

For example:
- `tsk/rust/claude/my-project`
- `tsk/python/claude/default`
- `tsk/node/codex/web-app`

If a specific project layer doesn't exist, TSK automatically falls back to the `default` project layer.

## Customizing Docker Images

### Project-Level Customization

Create custom Dockerfiles in your repository under `.tsk/dockerfiles/`:

```
.tsk/
└── dockerfiles/
    └── project/
        └── {project-name}.dockerfile
```

Example for a Rust project (`.tsk/dockerfiles/project/my-rust-app.dockerfile`):

```dockerfile
# Pre-build dependencies for faster subsequent builds
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src
```

Example for a Node.js project (`.tsk/dockerfiles/project/my-node-app.dockerfile`):

```dockerfile
# Install dependencies
COPY package.json package-lock.json ./
RUN npm ci
```

### User-Level Customization

Add personal preferences in `~/.config/tsk/dockerfiles/`:

```
~/.config/tsk/
└── dockerfiles/
    ├── base/
    │   └── default.dockerfile
    ├── stack/
    │   ├── python.dockerfile
    │   └── rust.dockerfile
    ├── agent/
    │   └── claude.dockerfile
    └── project/
        └── my-project.dockerfile
```

User-level customizations are useful for:
- Personal tool preferences
- Custom shell configurations
- Additional development utilities
- Alternative package sources or mirrors

### Asset Priority

TSK checks for Docker assets in this order:
1. **Project level**: `.tsk/dockerfiles/` in the repository
2. **User level**: `~/.config/tsk/dockerfiles/`
3. **Embedded**: Built into the TSK binary

The first matching asset is used.

## Building Docker Images

### Manual Building

Use the `docker build` command to manually build images:

```bash
# Build with auto-detected settings
tsk docker build

# Specify stack explicitly
tsk docker build --stack rust

# Build for a specific agent
tsk docker build --agent codex

# Build for a specific project
tsk docker build --project my-app

# Build without cache
tsk docker build --no-cache

# Preview the composed Dockerfile
tsk docker build --dry-run
```

### Automatic Building

TSK automatically builds missing images when:
- Running a task (`tsk run`)
- Starting an interactive shell (`tsk shell`)
- Adding tasks to the queue (`tsk add`)

## Tech Stack Auto-Detection

TSK automatically detects your project's technology stack based on repository files:

| Stack | Detection Files |
|-------|-----------------|
| Rust | `Cargo.toml` |
| Python | `pyproject.toml`, `requirements.txt`, `setup.py` |
| Node.js | `package.json` |
| Go | `go.mod` |
| Java | `pom.xml`, `build.gradle`, `build.gradle.kts` |
| Lua | files ending in `rockspec`, `.luacheckrc`, `init.lua` |
| Default | Used when no specific files found |

Auto-detection is used when the `--stack` flag is not provided.

## Security and Isolation

TSK implements multiple security layers:

### Non-Root Execution
- Containers run as the `agent` user (created by renaming the default `ubuntu` user)
- No sudo access within containers
- Limited filesystem permissions

### Network Isolation
- Containers use a dedicated Docker network (`tsk-network`)
- Internet access only through Squid proxy container (`tsk-proxy`)
- Proxy allows API access and package registry access while blocking general browsing
- Proxy allows access to:
  - **AI APIs**: api.anthropic.com, api.openai.com, sentry.io, statsig.com
  - **Python**: PyPI (pypi.org, pypi.python.org, files.pythonhosted.org)
  - **Rust**: crates.io, index.crates.io, static.crates.io
  - **Go**: proxy.golang.org, sum.golang.org, pkg.go.dev, golang.org, google.golang.org
  - **Java**: Maven Central (repo.maven.apache.org, repo1.maven.org), Gradle repositories
  - **Node.js**: registry.npmjs.org, nodejs.org, npmjs.com

### Resource Limits and Container Configuration
- Memory limited to 12GB
- CPU quota of 8 CPUs (800000 microseconds)
- Automatic cleanup of stopped containers
- Task containers named: `tsk-{task-id}` (e.g., `tsk-abc123def4`)
- Interactive containers named: `tsk-interactive-{task-id}`

### Volume Mounts
Each container has the following volumes mounted:
- Repository copy: `{repo_path}:/workspace` (read-write)
- Agent configuration: Agent-specific (e.g., `~/.claude:/home/agent/.claude` for Claude)
- Instructions: `{instructions_dir}:/instructions:ro` (read-only)
- Output: `{task_dir}/output:/output` (read-write)

### Capability Dropping
Containers run with minimal Linux capabilities, dropping exactly 9 capabilities:
- `NET_ADMIN` - Can't manage network interfaces
- `NET_RAW` - Can't create raw sockets
- `SETPCAP` - Can't change capability sets
- `SYS_ADMIN` - Can't mount filesystems or perform namespace operations
- `SYS_PTRACE` - Can't trace processes
- `DAC_OVERRIDE` - Can't bypass file read/write/execute permissions
- `AUDIT_WRITE` - Can't write audit logs
- `SETUID` - Can't change user IDs
- `SETGID` - Can't change group IDs

## Debugging Docker Issues

### Common Issues and Solutions

#### 1. Git Configuration Error
**Error**: "Git user.name is not set"

**Solution**: Git configuration is automatically inherited from your host system. Configure git on your host machine:
```bash
git config --global user.name "Your Name"
git config --global user.email "your@email.com"
```

#### 2. Missing Docker Layer
**Error**: "Stack layer 'xyz' not found"

**Solution**: Check available layers:
```bash
# List all available Docker templates
ls ~/.config/tsk/dockerfiles/stack/
ls .tsk/dockerfiles/stack/

# Or check embedded stacks (built into TSK)
# Available: rust, python, node, go, java, lua, default
```

#### 3. Build Failures
**Error**: Build errors during image creation

**Solutions**:
- Use `--dry-run` to inspect the composed Dockerfile
- Check Docker daemon logs
- Ensure Docker has sufficient disk space
- Try building with `--no-cache`

#### 4. Container Startup Issues
**Error**: Container fails to start or exits immediately

**Solutions**:
- Use `tsk shell` to get an interactive shell for debugging
- Check container logs: `docker logs <container-id>`
- Verify volume mounts and permissions

### Debugging Commands

```bash
# Interactive debugging shell
tsk shell

# View composed Dockerfile
tsk docker build --dry-run

# Check Docker images
docker images | grep tsk

# Inspect running containers
docker ps --filter "label=tsk"

# View proxy logs
docker logs tsk-proxy

# Stop the proxy if needed
tsk proxy stop
```

## Common Patterns and Examples

### Caching Python Dependencies

`.tsk/dockerfiles/project/my-python-app.dockerfile`:
```dockerfile
# Install Python dependencies using uv (recommended)
COPY requirements.txt ./
RUN uv pip install --system -r requirements.txt

# Alternative: Using pip
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt

# For projects using Poetry
COPY pyproject.toml poetry.lock ./
RUN poetry install --no-dev
```

### Pre-compiling Java Dependencies

`.tsk/dockerfiles/project/my-java-app.dockerfile`:
```dockerfile
# For Maven projects
COPY pom.xml ./
RUN mvn dependency:go-offline

# For Gradle projects
COPY build.gradle ./
RUN gradle dependencies
```

### Installing Additional Tools

User-level customization in `~/.config/tsk/dockerfiles/stack/python.dockerfile`:
```dockerfile
# Override embedded Python stack to add additional tools
# Note: This replaces the entire embedded stack, so you may want to copy
# the embedded stack first and then add your customizations

# Using uv (recommended)
RUN uv pip install --system ipython

# Or using pip
RUN pip install ipython

# System packages
RUN apt-get update && apt-get install -y postgresql-client \
    && rm -rf /var/lib/apt/lists/*
```

### Custom Environment Variables

Project-specific environment in `.tsk/dockerfiles/project/my-app.dockerfile`:
```dockerfile
ENV DATABASE_URL=postgresql://localhost/myapp_dev
ENV REDIS_URL=redis://localhost:6379
ENV NODE_ENV=development
```

## Best Practices

1. **Use Project Layers for Dependencies**: Cache project dependencies in custom project layers to speed up builds.

2. **Keep Layers Focused**: Each layer should have a single responsibility. Don't mix tech-stack setup with project dependencies.

3. **Minimize Layer Size**: Remove package manager caches and temporary files:
   ```dockerfile
   RUN apt-get update && apt-get install -y package \
       && rm -rf /var/lib/apt/lists/*
   ```

4. **Version Lock Dependencies**: Use lock files (`Cargo.lock`, `package-lock.json`, etc.) for reproducible builds.

5. **Test Locally**: Use `tsk shell` to test your custom Dockerfiles interactively before running tasks.

6. **Document Custom Layers**: Add comments in your Dockerfiles explaining why customizations are needed.

## Troubleshooting Checklist

When encountering issues:

1. ✓ Verify Docker daemon is running
2. ✓ Check available disk space
3. ✓ Ensure git is configured properly
4. ✓ Verify file permissions in `.tsk/` directory
5. ✓ Use `--dry-run` to inspect Dockerfile composition
6. ✓ Try building with `--no-cache`
7. ✓ Check TSK logs with `RUST_LOG=debug tsk <command>`
8. ✓ Test with `tsk shell` for interactive debugging

## Further Resources

- [TSK README](../README.md) - General TSK documentation
- [CLAUDE.md](../CLAUDE.md) - Project conventions and development guide
- Docker documentation - For advanced Docker concepts
- TSK GitHub Issues - For reporting bugs or requesting features

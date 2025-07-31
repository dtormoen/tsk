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

TSK composes Docker images from four distinct layers, each serving a specific purpose:

### 1. Base Layer
The foundation of all TSK containers:
- Ubuntu 24.04 base operating system
- Essential development tools (git, curl, build-essential, ripgrep, etc.)
- Non-root `agent` user (UID 1000) for security
- Git configuration inherited from host user
- Working directory set to `/workspace`

### 2. Tech-Stack Layer
Language-specific toolchains and runtimes:
- **default**: Minimal additions, used as fallback
- **rust**: Rust toolchain via rustup
- **python**: Python 3.11 with pip and common tools
- **node**: Node.js 20.x with npm
- **go**: Go 1.21 with module support
- **java**: OpenJDK 17 with Maven and Gradle
- **lua**: Lua 5.4 with LuaRocks

### 3. Agent Layer
AI agent installations and configurations:
- **claude-code**: Claude Code CLI with Node.js runtime
- Additional agents can be added as needed

### 4. Project Layer
Project-specific dependencies and optimizations:
- **default**: Empty layer, used as fallback
- **Custom layers**: Created per-project for dependency caching

## Docker Image Naming

TSK uses a hierarchical naming convention for Docker images:

```
tsk/{tech-stack}/{agent}/{project}
```

For example:
- `tsk/rust/claude-code/my-project`
- `tsk/python/claude-code/default`
- `tsk/node/claude-code/web-app`

If a specific project layer doesn't exist, TSK automatically falls back to the `default` project layer.

## Customizing Docker Images

### Project-Level Customization

Create custom Dockerfiles in your repository under `.tsk/dockerfiles/`:

```
.tsk/
└── dockerfiles/
    └── project/
        └── {project-name}/
            └── Dockerfile
```

Example for a Rust project (`.tsk/dockerfiles/project/my-rust-app/Dockerfile`):

```dockerfile
# Pre-build dependencies for faster subsequent builds
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src
```

Example for a Node.js project (`.tsk/dockerfiles/project/my-node-app/Dockerfile`):

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
    │   └── Dockerfile
    ├── tech-stack/
    │   └── python/
    │       └── Dockerfile
    └── agent/
        └── claude-code/
            └── Dockerfile
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

Use the `docker-build` command to manually build images:

```bash
# Build with auto-detected settings
tsk docker-build

# Specify tech stack explicitly
tsk docker-build --tech-stack rust

# Build for a specific project
tsk docker-build --project my-app

# Build without cache
tsk docker-build --no-cache

# Preview the composed Dockerfile
tsk docker-build --dry-run
```

### Automatic Building

TSK automatically builds missing images when:
- Running a task (`tsk run`)
- Starting a debug session (`tsk debug`)
- Executing a quick task (`tsk quick`)

## Tech Stack Auto-Detection

TSK automatically detects your project's technology stack based on repository files:

| Tech Stack | Detection Files |
|------------|-----------------|
| Rust | `Cargo.toml` |
| Python | `pyproject.toml`, `requirements.txt`, `setup.py` |
| Node.js | `package.json` |
| Go | `go.mod` |
| Java | `pom.xml`, `build.gradle`, `build.gradle.kts` |
| Lua | `*.rockspec`, `.luacheckrc`, `init.lua` |
| Default | Used when no specific files found |

Auto-detection is used when the `--tech-stack` flag is not provided.

## Security and Isolation

TSK implements multiple security layers:

### Non-Root Execution
- Containers run as the `agent` user (UID 1000)
- No sudo access within containers
- Limited filesystem permissions

### Network Isolation
- Containers use a dedicated Docker network (`tsk-network`)
- Internet access only through Squid proxy (`tsk-proxy`)
- Proxy allows API access while blocking general browsing
- Proxy allows access to package registries for all supported languages:
  - **Python**: PyPI (pypi.org, files.pythonhosted.org)
  - **Rust**: crates.io and related domains
  - **Go**: proxy.golang.org, sum.golang.org, and other Go infrastructure
  - **Java**: Maven Central, Gradle plugins, and common repositories
  - **npm**: registry.npmjs.org and Node.js resources

### Resource Limits
- Memory limited to 4GB
- CPU quota of 4 (400% of a single core)
- Automatic cleanup of stopped containers

### Capability Dropping
Containers run with minimal Linux capabilities, removing:
- `CAP_AUDIT_WRITE`
- `CAP_MKNOD`
- `CAP_NET_RAW`
- And others unnecessary for development

## Debugging Docker Issues

### Common Issues and Solutions

#### 1. Git Configuration Error
**Error**: "Git user.name is not set"

**Solution**: Configure git globally:
```bash
git config --global user.name "Your Name"
git config --global user.email "your@email.com"
```

#### 2. Missing Docker Layer
**Error**: "Tech stack layer 'xyz' not found"

**Solution**: Check available layers:
```bash
# List all available Docker templates
ls ~/.config/tsk/dockerfiles/tech-stack/
ls .tsk/dockerfiles/tech-stack/
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
- Use `tsk debug` to get an interactive shell
- Check container logs: `docker logs <container-id>`
- Verify volume mounts and permissions

### Debugging Commands

```bash
# Interactive debugging container
tsk debug

# View composed Dockerfile
tsk docker-build --dry-run

# Check Docker images
docker images | grep tsk

# Inspect running containers
docker ps --filter "label=tsk"

# View proxy logs
docker logs tsk-proxy
```

## Common Patterns and Examples

### Caching Python Dependencies

`.tsk/dockerfiles/project/my-python-app/Dockerfile`:
```dockerfile
# Install Python dependencies
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt

# For projects using Poetry
COPY pyproject.toml poetry.lock ./
RUN poetry install --no-dev
```

### Pre-compiling Java Dependencies

`.tsk/dockerfiles/project/my-java-app/Dockerfile`:
```dockerfile
# For Maven projects
COPY pom.xml ./
RUN mvn dependency:go-offline

# For Gradle projects
COPY build.gradle ./
RUN gradle dependencies
```

### Installing Additional Tools

User-level customization in `~/.config/tsk/dockerfiles/tech-stack/python/Dockerfile`:
```dockerfile
# Add after the embedded Python layer
RUN pip install ipython black flake8 mypy
RUN apt-get update && apt-get install -y postgresql-client
```

### Custom Environment Variables

Project-specific environment in `.tsk/dockerfiles/project/my-app/Dockerfile`:
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

5. **Test Locally**: Use `tsk debug` to test your custom Dockerfiles interactively before running tasks.

6. **Document Custom Layers**: Add comments in your Dockerfiles explaining why customizations are needed.

## Troubleshooting Checklist

When encountering issues:

1. ✓ Verify Docker daemon is running
2. ✓ Check available disk space
3. ✓ Ensure git is configured properly
4. ✓ Verify file permissions in `.tsk/` directory
5. ✓ Use `--dry-run` to inspect Dockerfile composition
6. ✓ Try building with `--no-cache`
7. ✓ Check TSK logs with `RUST_LOG=debug`
8. ✓ Test with `tsk debug` for interactive debugging

## Further Resources

- [TSK README](../README.md) - General TSK documentation
- [CLAUDE.md](../CLAUDE.md) - Project conventions and development guide
- Docker documentation - For advanced Docker concepts
- TSK GitHub Issues - For reporting bugs or requesting features

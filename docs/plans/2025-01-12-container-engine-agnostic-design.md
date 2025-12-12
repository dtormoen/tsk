# Container Engine Agnostic Design

## Overview

Make TSK work with both Docker and Podman, allowing users to use whichever container engine they have installed without configuration changes.

## Goals

- Zero-config experience for most users (auto-detection)
- Explicit control when needed (CLI flag and config)
- Transparent behavior with clear messaging
- Clean handling of engine switches

## Design Decisions

| Aspect | Decision |
|--------|----------|
| Detection | Check both engines, prefer Podman |
| Persistence | Auto-save detected engine to `~/.config/tsk/config.toml` |
| Override | Global `--engine` flag |
| Socket paths | Auto-detect with config override option |
| Fallback | Use other engine with warning (don't update config) |
| Terminology | "Docker/Podman" explicitly in messages |
| Orphan cleanup | Prompt user when engine switches |

## Configuration

### File: `~/.config/tsk/config.toml`

```toml
[engine]
# Detected/configured engine: "docker" or "podman"
name = "podman"

# Track last-used engine for orphan cleanup detection
last_used = "podman"

# Optional: override socket path (auto-detected if not set)
# socket_path = "/run/user/1000/podman/podman.sock"
```

### Resolution Order (highest to lowest priority)

1. `--engine` flag (if provided)
2. `config.toml` engine setting (if exists)
3. Auto-detection (check both, prefer Podman)

## Engine Detection

### Detection Logic

1. Check if Podman socket exists and is responsive (rootless first, then rootful)
2. Check if Docker socket exists and is responsive
3. Return first working engine, preferring Podman
4. Error if neither found: "Neither Docker nor Podman found. Please install one."

### Socket Path Resolution

| Engine | Path |
|--------|------|
| Podman rootless | `$XDG_RUNTIME_DIR/podman/podman.sock` |
| Podman rootful | `/var/run/podman/podman.sock` |
| Docker | `/var/run/docker.sock` |

### Connection

Bollard's `Docker::connect_with_socket()` works with both engines - pass the resolved socket path based on detected/configured engine.

## CLI Integration

### Global Flag

```
tsk --engine <docker|podman> [COMMAND]
```

### Examples

```bash
# Use auto-detected/configured engine
tsk run "fix the bug"

# Override to use Docker for this command
tsk --engine docker run "fix the bug"

# Override for server
tsk --engine podman server start
```

### First-Run Experience

```
$ tsk run "fix the bug"
Detected Podman, saving to config...
[task executes normally]
```

### Fallback Warning

```
$ tsk run "fix the bug"
Warning: Podman (configured) is unavailable, falling back to Docker
[task executes normally]
```

## Orphan Container Cleanup

### Trigger

When TSK detects the engine has changed from `last_used` to current engine.

### What Gets Cleaned Up

- `tsk-proxy` container from the other engine
- `tsk-network` network from the other engine

### User Interaction

```
$ tsk run "fix the bug"
Found orphaned Docker containers from previous engine:
  - tsk-proxy (container)
  - tsk-network (network)
Remove them? [Y/n]: y
Removed tsk-proxy container
Removed tsk-network network
[task executes normally]
```

### Implementation

1. Temporarily connect to the other engine's socket
2. Stop and remove `tsk-proxy` container
3. Remove `tsk-network` network
4. Update `last_used` in config to match current engine

## Code Changes

### Renames

| Current | New |
|---------|-----|
| `src/context/docker_client.rs` | `src/context/container_client.rs` |
| `DockerClient` trait | `ContainerClient` |
| `DefaultDockerClient` | `DefaultContainerClient` |
| `src/docker/` | `src/container/` |
| `DockerManager` | `ContainerManager` |
| `DockerImageManager` | `ImageManager` |
| `DockerBuildLockManager` | `BuildLockManager` |
| `DockerComposer` | `DockerfileComposer` (keep "Dockerfile" - it's the file format) |

### New Files

| File | Purpose |
|------|---------|
| `src/container/engine.rs` | `ContainerEngine` enum, `EngineConfig` struct, detection logic |
| `src/container/cleanup.rs` | Orphan detection and cleanup logic |

### Modified Files

| File | Changes |
|------|---------|
| `src/storage/tsk_config.rs` | Add `config.toml` parsing, `EngineConfig`, detection methods |
| `src/main.rs` / `src/cli.rs` | Add `--engine` global flag |
| `src/context/mod.rs` | Accept engine config, init client with correct socket |

## Migration Path

### Backward Compatibility

- Existing users with no `config.toml` get auto-detection on next run
- No breaking changes to CLI commands (flag is optional)
- Docker remains fully supported

### Testing

- Existing mock clients continue to work (just renamed)
- Add unit tests for engine detection logic
- Add unit tests for socket path resolution
- Integration tests can use either engine based on CI environment

### Documentation

- Update CLAUDE.md with new terminology
- Add engine configuration section to user-facing docs

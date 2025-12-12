# Container Engine Agnostic Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make TSK work with both Docker and Podman through auto-detection, configuration persistence, and CLI override.

**Architecture:** Add a `ContainerEngine` abstraction that handles engine detection and socket resolution. The `DefaultContainerClient` is modified to accept a socket path. Configuration is persisted in `~/.config/tsk/config.toml`. A global `--engine` flag allows runtime override.

**Tech Stack:** Rust, toml crate (new dependency), Bollard (existing)

---

## Task 1: Add toml dependency

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add toml dependency**

Edit `Cargo.toml` to add the toml crate for config file parsing:

```toml
toml = "0.8"
```

Add after the `handlebars` dependency line.

**Step 2: Verify build**

Run: `cargo build`
Expected: Build succeeds with new dependency

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: add toml dependency for config file parsing"
```

---

## Task 2: Create container engine module

**Files:**
- Create: `src/container/engine.rs`
- Create: `src/container/mod.rs`

**Step 1: Create the container module directory**

Run: `mkdir -p src/container`

**Step 2: Write the engine module**

Create `src/container/engine.rs`:

```rust
//! Container engine detection and configuration
//!
//! This module provides abstractions for detecting and configuring
//! container engines (Docker and Podman).

use std::env;
use std::path::PathBuf;

/// Supported container engines
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerEngine {
    Docker,
    Podman,
}

impl std::fmt::Display for ContainerEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerEngine::Docker => write!(f, "docker"),
            ContainerEngine::Podman => write!(f, "podman"),
        }
    }
}

impl std::str::FromStr for ContainerEngine {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "docker" => Ok(ContainerEngine::Docker),
            "podman" => Ok(ContainerEngine::Podman),
            _ => Err(format!("Unknown container engine: {s}. Use 'docker' or 'podman'.")),
        }
    }
}

/// Configuration for a container engine
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub engine: ContainerEngine,
    pub socket_path: PathBuf,
}

impl EngineConfig {
    /// Create a new engine configuration
    pub fn new(engine: ContainerEngine, socket_path: PathBuf) -> Self {
        Self { engine, socket_path }
    }
}

/// Get the default socket path for an engine
pub fn default_socket_path(engine: ContainerEngine) -> Option<PathBuf> {
    match engine {
        ContainerEngine::Podman => {
            // Try rootless first
            if let Ok(xdg_runtime) = env::var("XDG_RUNTIME_DIR") {
                let rootless_path = PathBuf::from(&xdg_runtime).join("podman/podman.sock");
                if rootless_path.exists() {
                    return Some(rootless_path);
                }
            }
            // Fall back to rootful
            let rootful_path = PathBuf::from("/var/run/podman/podman.sock");
            if rootful_path.exists() {
                return Some(rootful_path);
            }
            None
        }
        ContainerEngine::Docker => {
            let docker_path = PathBuf::from("/var/run/docker.sock");
            if docker_path.exists() {
                return Some(docker_path);
            }
            None
        }
    }
}

/// Check if an engine is available by verifying its socket exists
pub fn is_engine_available(engine: ContainerEngine) -> bool {
    default_socket_path(engine).is_some()
}

/// Detect available container engines
///
/// Returns the preferred engine (Podman first) if available,
/// or falls back to the other engine.
pub fn detect_engine() -> Option<ContainerEngine> {
    // Prefer Podman
    if is_engine_available(ContainerEngine::Podman) {
        return Some(ContainerEngine::Podman);
    }
    // Fall back to Docker
    if is_engine_available(ContainerEngine::Docker) {
        return Some(ContainerEngine::Docker);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_engine_display() {
        assert_eq!(ContainerEngine::Docker.to_string(), "docker");
        assert_eq!(ContainerEngine::Podman.to_string(), "podman");
    }

    #[test]
    fn test_container_engine_from_str() {
        assert_eq!(
            "docker".parse::<ContainerEngine>().unwrap(),
            ContainerEngine::Docker
        );
        assert_eq!(
            "podman".parse::<ContainerEngine>().unwrap(),
            ContainerEngine::Podman
        );
        assert_eq!(
            "Docker".parse::<ContainerEngine>().unwrap(),
            ContainerEngine::Docker
        );
        assert_eq!(
            "PODMAN".parse::<ContainerEngine>().unwrap(),
            ContainerEngine::Podman
        );
        assert!("invalid".parse::<ContainerEngine>().is_err());
    }

    #[test]
    fn test_engine_config_new() {
        let config = EngineConfig::new(
            ContainerEngine::Docker,
            PathBuf::from("/var/run/docker.sock"),
        );
        assert_eq!(config.engine, ContainerEngine::Docker);
        assert_eq!(config.socket_path, PathBuf::from("/var/run/docker.sock"));
    }
}
```

**Step 3: Create the container module**

Create `src/container/mod.rs`:

```rust
//! Container engine abstraction module
//!
//! This module provides engine-agnostic container operations,
//! supporting both Docker and Podman.

pub mod engine;

pub use engine::{ContainerEngine, EngineConfig, default_socket_path, detect_engine, is_engine_available};
```

**Step 4: Verify build**

Run: `cargo build`
Expected: Build succeeds

**Step 5: Run tests**

Run: `cargo test container::engine`
Expected: All tests pass

**Step 6: Commit**

```bash
git add src/container/
git commit -m "feat: add container engine detection module"
```

---

## Task 3: Add engine configuration to TskConfig

**Files:**
- Modify: `src/context/tsk_config.rs`

**Step 1: Add config file structures**

Add these imports at the top of `src/context/tsk_config.rs`:

```rust
use crate::container::{ContainerEngine, EngineConfig, default_socket_path, detect_engine};
```

**Step 2: Add config file structures**

Add after the `TskConfigError` enum:

```rust
/// Configuration file structure for config.toml
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub engine: EngineSection,
}

/// Engine configuration section in config.toml
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct EngineSection {
    /// The configured container engine (docker or podman)
    pub name: Option<String>,
    /// The last-used engine (for orphan cleanup detection)
    pub last_used: Option<String>,
    /// Optional custom socket path
    pub socket_path: Option<String>,
}
```

**Step 3: Add config file methods to TskConfig**

Add these methods to the `impl TskConfig` block:

```rust
    /// Get the path to the config.toml file
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Load the configuration file
    pub fn load_config_file(&self) -> Result<ConfigFile, TskConfigError> {
        let config_path = self.config_file();
        if !config_path.exists() {
            return Ok(ConfigFile::default());
        }
        let content = std::fs::read_to_string(&config_path)
            .map_err(TskConfigError::Io)?;
        toml::from_str(&content)
            .map_err(|e| TskConfigError::ConfigParse(e.to_string()))
    }

    /// Save the configuration file
    pub fn save_config_file(&self, config: &ConfigFile) -> Result<(), TskConfigError> {
        let config_path = self.config_file();
        let content = toml::to_string_pretty(config)
            .map_err(|e| TskConfigError::ConfigParse(e.to_string()))?;
        std::fs::write(&config_path, content)
            .map_err(TskConfigError::Io)
    }

    /// Resolve the container engine configuration
    ///
    /// Priority order:
    /// 1. CLI override (if provided)
    /// 2. Config file setting
    /// 3. Auto-detection (prefer Podman)
    ///
    /// Returns the engine config and whether a fallback was used
    pub fn resolve_engine_config(
        &self,
        cli_override: Option<ContainerEngine>,
    ) -> Result<(EngineConfig, bool), TskConfigError> {
        let mut config_file = self.load_config_file()?;
        let mut used_fallback = false;

        // Determine the engine to use
        let engine = if let Some(engine) = cli_override {
            // CLI override takes precedence
            engine
        } else if let Some(ref name) = config_file.engine.name {
            // Use configured engine
            name.parse::<ContainerEngine>()
                .map_err(|e| TskConfigError::ConfigParse(e))?
        } else {
            // Auto-detect and persist
            let detected = detect_engine()
                .ok_or_else(|| TskConfigError::NoContainerEngine)?;

            println!("Detected {}, saving to config...", detected);
            config_file.engine.name = Some(detected.to_string());
            config_file.engine.last_used = Some(detected.to_string());
            self.save_config_file(&config_file)?;

            detected
        };

        // Resolve socket path
        let socket_path = if let Some(ref path) = config_file.engine.socket_path {
            PathBuf::from(path)
        } else {
            // Check if configured engine is available
            if let Some(path) = default_socket_path(engine) {
                path
            } else {
                // Engine not available, try fallback
                let other_engine = match engine {
                    ContainerEngine::Docker => ContainerEngine::Podman,
                    ContainerEngine::Podman => ContainerEngine::Docker,
                };

                if let Some(path) = default_socket_path(other_engine) {
                    eprintln!(
                        "Warning: {} (configured) is unavailable, falling back to {}",
                        engine, other_engine
                    );
                    used_fallback = true;
                    // Update last_used but NOT name (keep user's config)
                    config_file.engine.last_used = Some(other_engine.to_string());
                    self.save_config_file(&config_file)?;
                    path
                } else {
                    return Err(TskConfigError::NoContainerEngine);
                }
            }
        };

        // Update last_used if not a fallback
        if !used_fallback && config_file.engine.last_used.as_deref() != Some(&engine.to_string()) {
            config_file.engine.last_used = Some(engine.to_string());
            self.save_config_file(&config_file)?;
        }

        Ok((EngineConfig::new(engine, socket_path), used_fallback))
    }
```

**Step 4: Add new error variant**

Add to the `TskConfigError` enum:

```rust
    #[error("Failed to parse configuration: {0}")]
    ConfigParse(String),
    #[error("Neither Docker nor Podman found. Please install one.")]
    NoContainerEngine,
```

**Step 5: Verify build**

Run: `cargo build`
Expected: Build succeeds

**Step 6: Add tests**

Add to the `#[cfg(test)] mod tests` section:

```rust
    #[test]
    fn test_config_file_path() {
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();
        assert!(config.config_file().to_string_lossy().contains("config.toml"));
    }

    #[test]
    fn test_load_missing_config_file() {
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();
        let result = config.load_config_file();
        assert!(result.is_ok());
        let config_file = result.unwrap();
        assert!(config_file.engine.name.is_none());
    }

    #[test]
    fn test_save_and_load_config_file() {
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();

        let mut config_file = ConfigFile::default();
        config_file.engine.name = Some("podman".to_string());
        config_file.engine.last_used = Some("podman".to_string());

        config.save_config_file(&config_file).unwrap();

        let loaded = config.load_config_file().unwrap();
        assert_eq!(loaded.engine.name, Some("podman".to_string()));
        assert_eq!(loaded.engine.last_used, Some("podman".to_string()));
    }
```

**Step 7: Run tests**

Run: `cargo test tsk_config`
Expected: All tests pass

**Step 8: Commit**

```bash
git add src/context/tsk_config.rs
git commit -m "feat: add container engine configuration to TskConfig"
```

---

## Task 4: Update main.rs with --engine flag

**Files:**
- Modify: `src/main.rs`

**Step 1: Add container module to main.rs**

Add after the `mod docker;` line:

```rust
mod container;
```

**Step 2: Add global engine flag to Cli struct**

Modify the `Cli` struct:

```rust
#[derive(Parser)]
#[command(name = "tsk")]
#[command(author, version, about = "TSK - Task delegation to AI agents", long_about = None)]
#[command(help_template = r#"{name} {version}
{author-with-newline}
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}
"#)]
struct Cli {
    /// Container engine to use (docker or podman)
    #[arg(long, global = true, value_parser = parse_container_engine)]
    engine: Option<container::ContainerEngine>,

    #[command(subcommand)]
    command: Commands,
}
```

**Step 3: Add parser function**

Add before the `Cli` struct:

```rust
fn parse_container_engine(s: &str) -> Result<container::ContainerEngine, String> {
    s.parse()
}
```

**Step 4: Pass engine to AppContext**

Modify the main function to pass engine override:

```rust
#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Create the AppContext using the builder pattern
    let app_context = AppContext::builder()
        .with_engine_override(cli.engine)
        .build();

    // ... rest of the function unchanged
}
```

**Step 5: Verify build**

Run: `cargo build`
Expected: Build fails (with_engine_override doesn't exist yet - that's expected)

**Step 6: Commit partial progress**

```bash
git add src/main.rs
git commit -m "feat: add --engine global flag to CLI"
```

---

## Task 5: Update AppContext to handle engine configuration

**Files:**
- Modify: `src/context/mod.rs`

**Step 1: Add engine_override to AppContextBuilder**

Add to the `AppContextBuilder` struct:

```rust
    engine_override: Option<crate::container::ContainerEngine>,
```

**Step 2: Initialize engine_override in new()**

Add to the `AppContextBuilder::new()` method:

```rust
    engine_override: None,
```

**Step 3: Add with_engine_override method**

Add to `impl AppContextBuilder`:

```rust
    /// Configure the container engine override for this context
    ///
    /// This overrides the configured/detected engine for the current session
    pub fn with_engine_override(mut self, engine: Option<crate::container::ContainerEngine>) -> Self {
        self.engine_override = engine;
        self
    }
```

**Step 4: Update the production build() to use engine config**

In the `#[cfg(not(test))]` block of the `build()` method, replace the docker_client initialization:

```rust
            let docker_client = self.docker_client.unwrap_or_else(|| {
                // Resolve engine configuration
                let engine_config = tsk_config
                    .resolve_engine_config(self.engine_override)
                    .unwrap_or_else(|e| {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    });

                Arc::new(docker_client::DefaultDockerClient::with_socket(
                    &engine_config.0.socket_path,
                ))
            });
```

**Step 5: Verify build**

Run: `cargo build`
Expected: Build fails (DefaultDockerClient::with_socket doesn't exist yet - that's expected)

**Step 6: Commit partial progress**

```bash
git add src/context/mod.rs
git commit -m "feat: add engine override support to AppContext"
```

---

## Task 6: Update DefaultDockerClient to accept socket path

**Files:**
- Modify: `src/context/docker_client.rs`

**Step 1: Add with_socket constructor**

Add to `impl DefaultDockerClient`:

```rust
    /// Create a new DockerClient connected to a specific socket path
    ///
    /// This allows connecting to Docker or Podman via their respective sockets.
    #[allow(dead_code)] // Used in production code
    pub fn with_socket(socket_path: &std::path::Path) -> Self {
        let socket_str = socket_path.to_string_lossy();
        match Docker::connect_with_socket(&socket_str, 120, bollard::API_DEFAULT_VERSION) {
            Ok(docker) => Self { docker },
            Err(e) => panic!(
                "Failed to connect to container engine at {}: {e}\n\n\
                Please ensure Docker or Podman is installed and running:\n\
                  - Docker: Check 'docker ps' or start Docker Desktop\n\
                  - Podman: Check 'podman ps' or run 'systemctl --user start podman.socket'\n\n\
                If the daemon is running, check socket permissions.",
                socket_str
            ),
        }
    }
```

**Step 2: Verify build**

Run: `cargo build`
Expected: Build succeeds

**Step 3: Run tests**

Run: `cargo test`
Expected: All tests pass

**Step 4: Commit**

```bash
git add src/context/docker_client.rs
git commit -m "feat: add socket path constructor to DefaultDockerClient"
```

---

## Task 7: Add orphan cleanup module

**Files:**
- Create: `src/container/cleanup.rs`
- Modify: `src/container/mod.rs`

**Step 1: Create cleanup module**

Create `src/container/cleanup.rs`:

```rust
//! Orphan container cleanup
//!
//! This module handles detection and cleanup of orphaned containers
//! from a previous container engine when switching between Docker and Podman.

use crate::container::{ContainerEngine, default_socket_path};
use crate::context::docker_client::DefaultDockerClient;
use crate::context::tsk_config::TskConfig;
use bollard::query_parameters::RemoveContainerOptions;
use std::io::{self, Write};
use std::sync::Arc;

const PROXY_CONTAINER_NAME: &str = "tsk-proxy";
const TSK_NETWORK_NAME: &str = "tsk-network";

/// Check if orphan cleanup is needed and prompt the user
///
/// Returns true if cleanup was performed or skipped, false if error
pub async fn check_and_cleanup_orphans(
    tsk_config: &TskConfig,
    current_engine: ContainerEngine,
) -> Result<(), String> {
    let config_file = tsk_config
        .load_config_file()
        .map_err(|e| e.to_string())?;

    // Check if we switched engines
    let last_used = match &config_file.engine.last_used {
        Some(last) => last.parse::<ContainerEngine>().ok(),
        None => None,
    };

    let other_engine = match current_engine {
        ContainerEngine::Docker => ContainerEngine::Podman,
        ContainerEngine::Podman => ContainerEngine::Docker,
    };

    // If last_used matches the other engine, we may have orphans
    if last_used == Some(other_engine) {
        // Check if the other engine is available
        if let Some(socket_path) = default_socket_path(other_engine) {
            // Try to connect and check for orphans
            let client = DefaultDockerClient::with_socket(&socket_path);
            let client = Arc::new(client);

            // Check if proxy container exists
            let has_orphans = check_orphan_container(&client).await;

            if has_orphans {
                prompt_cleanup(&client, other_engine).await?;
            }
        }
    }

    Ok(())
}

async fn check_orphan_container(
    client: &Arc<DefaultDockerClient>,
) -> bool {
    use crate::context::docker_client::DockerClient;

    // Try to inspect the proxy container
    match client.inspect_container(PROXY_CONTAINER_NAME).await {
        Ok(_) => true,
        Err(e) if e.contains("No such container") => false,
        Err(_) => false,
    }
}

async fn prompt_cleanup(
    client: &Arc<DefaultDockerClient>,
    other_engine: ContainerEngine,
) -> Result<(), String> {
    use crate::context::docker_client::DockerClient;

    println!("\nFound orphaned {} containers from previous engine:", other_engine);
    println!("  - {} (container)", PROXY_CONTAINER_NAME);
    println!("  - {} (network)", TSK_NETWORK_NAME);
    print!("Remove them? [Y/n]: ");
    io::stdout().flush().map_err(|e| e.to_string())?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| e.to_string())?;

    let input = input.trim().to_lowercase();
    if input.is_empty() || input == "y" || input == "yes" {
        // Remove container
        match client
            .remove_container(
                PROXY_CONTAINER_NAME,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(_) => println!("Removed {} container", PROXY_CONTAINER_NAME),
            Err(e) if e.contains("No such container") => (),
            Err(e) => eprintln!("Warning: Failed to remove container: {}", e),
        }

        // Note: Network removal would require additional DockerClient method
        // For now, just inform the user
        println!("Note: Run '{} network rm {}' to remove the orphaned network",
            other_engine, TSK_NETWORK_NAME);
    } else {
        println!("Skipping cleanup");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_names() {
        assert_eq!(PROXY_CONTAINER_NAME, "tsk-proxy");
        assert_eq!(TSK_NETWORK_NAME, "tsk-network");
    }
}
```

**Step 2: Update container/mod.rs**

Add to `src/container/mod.rs`:

```rust
pub mod cleanup;

pub use cleanup::check_and_cleanup_orphans;
```

**Step 3: Verify build**

Run: `cargo build`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add src/container/
git commit -m "feat: add orphan container cleanup module"
```

---

## Task 8: Integrate orphan cleanup into startup

**Files:**
- Modify: `src/context/mod.rs`

**Step 1: Add cleanup check to build()**

In the `#[cfg(not(test))]` block of the `build()` method, after resolving engine config, add:

```rust
                // Check for orphaned containers from engine switch
                let engine_config = engine_config.0;
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    if let Err(e) = crate::container::check_and_cleanup_orphans(
                        &tsk_config,
                        engine_config.engine,
                    ).await {
                        eprintln!("Warning: Failed to check for orphaned containers: {}", e);
                    }
                });
```

Note: This is tricky because `build()` is not async. We'll need to handle this carefully.

**Step 2: Alternative approach - make cleanup lazy**

Actually, let's make cleanup happen on first container operation instead. Modify `src/docker/proxy_manager.rs`:

Add at the start of `ensure_proxy()`:

```rust
        // Check for orphaned containers on first proxy startup
        // This handles the case where the user switched container engines
        // TODO: Implement orphan cleanup check here
```

For now, let's skip the integration and leave it as a manual step. The cleanup module is available for future use.

**Step 3: Commit**

```bash
git add src/docker/proxy_manager.rs
git commit -m "docs: add TODO for orphan cleanup integration"
```

---

## Task 9: Update CLAUDE.md documentation

**Files:**
- Modify: `CLAUDE.md`

**Step 1: Add container engine section**

Add a new section after "Docker Infrastructure":

```markdown
### Container Engine Support

TSK supports both Docker and Podman as container engines:

- **Auto-detection**: On first run, TSK detects available engines (prefers Podman)
- **Configuration**: Engine choice is persisted in `~/.config/tsk/config.toml`
- **Override**: Use `tsk --engine docker` or `tsk --engine podman` to override

**Configuration file format:**
```toml
[engine]
name = "podman"           # Configured engine
last_used = "podman"      # Last actually used engine
# socket_path = "/custom/path.sock"  # Optional custom socket
```

**Socket paths (auto-detected):**
- Docker: `/var/run/docker.sock`
- Podman rootless: `$XDG_RUNTIME_DIR/podman/podman.sock`
- Podman rootful: `/var/run/podman/podman.sock`
```

**Step 2: Update terminology**

In the "Docker Infrastructure" section, add a note:

```markdown
Note: While the module is named "docker", TSK's container operations work with both Docker and Podman via the Bollard library's compatible API.
```

**Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: add container engine support documentation"
```

---

## Task 10: Run full test suite and fix any issues

**Step 1: Run all tests**

Run: `cargo test`
Expected: All tests pass

**Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

**Step 3: Run format check**

Run: `cargo fmt -- --check`
Expected: No formatting issues

**Step 4: Test CLI help**

Run: `cargo run -- --help`
Expected: Shows `--engine` flag in help output

**Step 5: Test engine flag parsing**

Run: `cargo run -- --engine podman list`
Run: `cargo run -- --engine docker list`
Run: `cargo run -- --engine invalid list`
Expected: First two work (may error on no engine), third shows parse error

**Step 6: Commit any fixes**

```bash
git add -A
git commit -m "fix: address test and lint issues"
```

---

## Task 11: Final integration test

**Step 1: Test with Docker (if available)**

```bash
# Ensure Docker is running
docker ps

# Run TSK with Docker
cargo run -- --engine docker list
```

**Step 2: Test with Podman (if available)**

```bash
# Ensure Podman socket is running
systemctl --user start podman.socket
podman ps

# Run TSK with Podman
cargo run -- --engine podman list
```

**Step 3: Test auto-detection**

```bash
# Remove config file if it exists
rm -f ~/.config/tsk/config.toml

# Run TSK without --engine flag
cargo run -- list

# Check that config was created
cat ~/.config/tsk/config.toml
```

**Step 4: Final commit**

```bash
git add -A
git commit -m "feat: complete container engine agnostic support"
```

---

## Summary

This implementation plan adds Docker/Podman agnostic support through:

1. **New container module** (`src/container/`) with engine detection
2. **Configuration file** (`~/.config/tsk/config.toml`) for persistence
3. **Global CLI flag** (`--engine`) for runtime override
4. **Socket path resolution** for both engines
5. **Fallback support** with warnings when configured engine unavailable
6. **Orphan cleanup module** (ready for future integration)

The implementation preserves all existing functionality while adding the new capability.

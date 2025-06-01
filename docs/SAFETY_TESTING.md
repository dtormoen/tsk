# Safety in Testing

The `git.rs` and `docker.rs` modules use a factory pattern to ensure that unit tests cannot accidentally modify git repositories or start Docker containers.

## How it works

### Factory Functions

Both modules expose factory functions that return different implementations based on whether the code is running in test mode:

- `get_worktree_manager()` - Returns a WorktreeManager instance
- `get_docker_manager()` - Returns a DockerManager instance

In production (`cfg(not(test))`), these return the real implementations that interact with git and Docker.

In test mode (`cfg(test)`), these return dummy implementations that panic with a helpful error message if any method is called.

### Proper Testing

To test code that uses these managers, you should create mock implementations:

```rust
// For WorktreeManager
use crate::git::{WorktreeManager, CommandExecutor};

struct MockExecutor;
impl CommandExecutor for MockExecutor {
    fn execute(&self, program: &str, args: &[&str]) -> Result<Output, String> {
        // Return mock results
    }
}

let manager = WorktreeManager::with_executor(Box::new(MockExecutor));

// For DockerManager
use crate::docker::{DockerManager, DockerClient};

struct MockDockerClient;
#[async_trait]
impl DockerClient for MockDockerClient {
    // Implement mock methods
}

let manager = DockerManager::with_client(MockDockerClient);
```

### Why This Approach?

This pattern ensures:
1. Unit tests can't accidentally create git worktrees or branches
2. Unit tests can't accidentally start Docker containers
3. Developers are forced to explicitly mock these resources
4. Clear error messages guide developers to the correct approach

## Example Test

See `src/factory_test.rs` for examples of:
- Tests that verify the panic behavior
- Tests that properly mock the resources
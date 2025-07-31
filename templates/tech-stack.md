# Create Tech-Stack Docker Layer

You need to create a technology stack Docker layer (Dockerfile) that will be used by TSK to provide language-specific toolchains and development tools.

## Context

TSK uses a layered Docker image system where each task runs in a container built from these layers:
1. **Base layer**: Ubuntu 24.04 with essential development tools (git, curl, build-essential, etc.)
2. **Tech-stack layer**: Language-specific toolchains (what you're creating)
3. **Agent layer**: AI agent setup
4. **Project layer**: Project-specific dependencies

The tech-stack layer installs the core language runtime, package managers, and common development tools for a specific technology stack.

## Requirements for the Tech-Stack Layer

Your Dockerfile should:

1. **Build on the base layer**: Assume Ubuntu 24.04 with basic tools installed
2. **Run as the agent user**: The base layer sets up an `agent` user (UID 1000)
3. **Install the language runtime**: Install the appropriate version of the language
4. **Install package managers**: Include the standard package managers for the ecosystem
5. **Install common development tools**: Linters, formatters, test runners, etc.
6. **Set up environment variables**: PATH, language-specific variables
7. **Return to /workspace**: End with `WORKDIR /workspace`

## Instructions

1. First, determine which technology stack you're implementing for:
   - What language runtime needs to be installed
   - What package managers are standard
   - What development tools are commonly used
   - What environment variables need to be set

2. Create a Dockerfile at `.tsk/dockerfiles/tech-stack/{tech-stack-name}/Dockerfile` where `{tech-stack-name}` is one of: `rust`, `python`, `node`, `go`, `java`, `lua`, etc.

3. The Dockerfile should follow this general pattern:
   ```dockerfile
   # {Language} tech stack layer

   # Already running as agent user from base layer
   WORKDIR /home/agent

   # Install language runtime
   # Install package managers
   # Install common development tools

   # Set environment variables
   ENV PATH="..."

   # Switch back to workspace directory
   WORKDIR /workspace
   ```

4. Important notes:
   - User is already `agent` (not root), use `sudo` for system packages
   - Python3 and pip are pre-installed in base layer
   - Consider using version managers (rustup, nvm, pyenv) when appropriate
   - Install tools to user home directory when possible
   - Keep the layer focused on language toolchain, not project dependencies

5. Example patterns for reference:

   **Rust tech-stack:**
   - Install via rustup
   - Install common tools: just, cargo-watch, etc.
   - Set CARGO_TARGET_DIR to avoid workspace pollution

   **Python tech-stack:**
   - Upgrade pip, install pipx
   - Install tools via pipx: poetry, black, ruff, pytest
   - Consider installing uv for fast package management

   **Node tech-stack:**
   - Install Node.js LTS via NodeSource or nvm
   - Install pnpm, yarn as alternatives to npm
   - Global tools: typescript, eslint, prettier

   **Go tech-stack:**
   - Download and extract Go tarball
   - Install tools: gopls, dlv, goimports, staticcheck
   - Set GOPATH and update PATH

   **Java tech-stack:**
   - Install OpenJDK (prefer LTS versions)
   - Install Maven and Gradle
   - Consider SDKMAN for version management

Remember: This layer provides the development toolchain that all projects using this technology will share. Project-specific dependencies go in the project layer.

---
description: Create a tech-stack tsk Docker layer
---

# Create Tech-Stack Docker Layer

You need to create a technology stack Docker layer (Dockerfile) that will be used by TSK to provide language-specific toolchains and development tools.

## Context

TSK uses a layered Docker image system where each task runs in a container built from these layers:
1. **Base layer**: Ubuntu 24.04 with essential development tools (git, curl, build-essential, etc.)
2. **Stack layer**: Language-specific toolchains (what you're creating)
3. **Project layer**: Project-specific dependencies
4. **Agent layer**: AI agent setup (added last)

The stack layer installs the core language runtime, package managers, and common development tools for a specific technology stack.

## Requirements for the Stack Layer

Your Dockerfile should:

1. **Build on the base layer**: Assume Ubuntu 24.04 with basic tools installed
2. **Run as the agent user**: The base layer sets up an `agent` user (created by renaming the default ubuntu user)
3. **Install the language runtime**: Install the appropriate version of the language
4. **Install package managers**: Include the standard package managers for the ecosystem
5. **Install common development tools**: Linters, formatters, test runners, etc.
6. **Set up environment variables**: PATH, language-specific variables

## Instructions

1. First, determine which technology stack you're implementing for:
   - What language runtime needs to be installed
   - What package managers are standard
   - What development tools are commonly used
   - What environment variables need to be set

2. Create a Dockerfile using the correct location and naming:
   - **For embedded stacks** (built into TSK): `dockerfiles/stack/{stack-name}.dockerfile`
   - **For user-level customization**: `~/.config/tsk/dockerfiles/stack/{stack-name}.dockerfile`
   - **For project-level override**: `.tsk/dockerfiles/stack/{stack-name}.dockerfile`

   Where `{stack-name}` is one of: `rust`, `python`, `node`, `go`, `java`, `lua`, `default`, etc.

   Note: Use `stack` (not `tech-stack`) and `.dockerfile` extension (not `Dockerfile`).

3. The Dockerfile should follow this general pattern:
   ```dockerfile
   # {Language} tech stack layer

   # Install language runtime (as agent user)
   # Install package managers
   # Install common development tools

   # Set environment variables
   ENV PATH="/home/agent/.local/bin:${PATH}"
   ```

   Note: Don't change WORKDIR - the base layer already sets it to `/workspace` and maintains it throughout.

4. Important notes:
   - The stack layer runs as the `agent` user (not root)
   - For system packages, you'll need `sudo` (the agent user has sudo access during build)
   - Python3 and pip are pre-installed in the base layer
   - Consider using version managers (rustup, nvm, pyenv) when appropriate
   - Install tools to the user's home directory (`/home/agent/`) when possible
   - Keep the layer focused on language toolchain, not project dependencies
   - The working directory remains `/workspace` throughout - don't change it

5. Example patterns for reference:

   **Rust tech-stack:**
   - Install via rustup
   - Install common tools: just, cargo-watch, etc.
   - Set CARGO_TARGET_DIR to avoid workspace pollution

   **Python tech-stack:**
   - Install uv for fast package management (recommended primary tool)
   - Create a virtual environment in /home/agent/.venv using uv
   - Install common tools via uv pip: poetry, black, ruff, pytest, mypy
   - Set up VIRTUAL_ENV and PATH to use the venv

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

Remember: This layer provides the development toolchain that all projects using this technology will share. Project-specific dependencies go in the project layer (which comes after the stack layer but before the agent layer).

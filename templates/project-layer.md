---
description: Create a project-specific tsk Docker layer
---

# Create Project-Specific Docker Layer

You need to create a project-specific Docker layer (Dockerfile) that will be used by TSK to speed up builds and manage dependencies for this repository.

## Context

TSK uses a layered Docker image system where each task runs in a container built from these layers:
1. **Base layer**: Ubuntu 24.04 with essential development tools
2. **Stack layer**: Language-specific toolchains (e.g., Rust/Cargo, Python/uv, Node/npm)
3. **Project layer**: Project-specific dependencies and optimizations (what you're creating)
4. **Agent layer**: AI agent setup (added last)

The repository will be mounted at `/workspace/{project_name}` in the container, and the working directory will be set to `/workspace/{project_name}`. The container runs as the `agent` user (not root).

## Requirements for the Project Layer

Your Dockerfile should:

1. **Assume the tech stack is already installed**: Don't reinstall language runtimes or package managers
2. **Install project-specific dependencies**: Based on dependency files in the repository (e.g., `Cargo.toml`, `requirements.txt`, `package.json`)
3. **Cache dependencies efficiently**: Copy only dependency files first, install dependencies, then copy source code
4. **Pre-compile or pre-build when beneficial**: For compiled languages, consider pre-building dependencies
5. **Keep the layer focused**: Only include what speeds up builds or is required for the project

## Instructions

1. First, examine the repository to understand:
   - What technology stack is being used
   - What dependencies need to be installed
   - What build tools or pre-compilation would be beneficial

2. Create a Dockerfile at `.tsk/dockerfiles/project/{project-name}.dockerfile` where `{project-name}` is the directory this repository is in (lowercase, with dots, underscores, and hyphens allowed as valid Docker name characters).

3. The Dockerfile should follow this general pattern:
   ```dockerfile
   # Project-specific layer for {project-name}

   # Copy dependency files only (for caching)
   # Install dependencies
   # Optional: Pre-compile or pre-build steps
   ```

4. Important notes:
   - The build-time working directory is `/workspace` (at runtime it becomes `/workspace/{project_name}`)
   - The user is already set to `agent` (not root)
   - Language toolchains are already installed
   - Network access goes through a proxy (already configured)

5. Example patterns for common tech stacks:

   **Rust projects:**
   ```dockerfile
   # Copy dependency files
   COPY Cargo.toml Cargo.lock ./
   # Create dummy main.rs to build dependencies
   RUN mkdir src && echo "fn main() {}" > src/main.rs
   # Build dependencies (artifacts go to $CARGO_TARGET_DIR set in tech-stack layer)
   RUN cargo build
   # Remove dummy source
   RUN rm -rf src
   ```

   **Python projects:**
   ```dockerfile
   # Copy dependency files
   COPY requirements.txt ./
   # Install dependencies using uv (preferred - faster and more reliable)
   RUN uv pip install --system -r requirements.txt
   # Or using pip (if uv is unavailable)
   RUN pip install -r requirements.txt
   ```

   **Node.js projects:**
   ```dockerfile
   # Copy dependency files
   COPY package.json package-lock.json ./
   # Install dependencies
   RUN npm ci
   ```

Remember: Focus on what will speed up task execution and ensure all project dependencies are available. The actual project code will be mounted at runtime, so don't copy the entire source tree.

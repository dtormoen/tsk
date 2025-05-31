FROM ubuntu:22.04

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install essential tools
RUN apt-get update && apt-get install -y \
    git \
    curl \
    ca-certificates \
    python3 \
    python3-pip \
    nodejs \
    npm \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

# Install just using cargo
RUN cargo install just

# Create a non-root user for running the agent
RUN useradd -m -s /bin/bash agent

# Set up working directory
WORKDIR /workspace

# Switch to non-root user
USER agent

# Set environment variables
ENV PATH="/home/agent/.local/bin:${PATH}"
ENV PYTHONUNBUFFERED=1

# Default command (will be overridden by agent setup)
CMD ["/bin/bash"]
FROM ubuntu:22.04

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install system dependencies
RUN apt-get update && apt-get install -y \
    curl \
    git \
    openssh-client \
    python3 \
    python3-pip \
    build-essential \
    sudo \
    vim \
    jq \
    ca-certificates \
    gnupg \
    iputils-ping \
    dnsutils \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js 20.x
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs

RUN npm install -g @anthropic-ai/claude-code

# Create a non-root user for running the agent with UID/GID 1000
# This helps with file permissions when mounting volumes
RUN groupadd -g 1000 agent && \
    useradd -m -s /bin/bash -u 1000 -g agent agent

# Switch to agent user to install Rust and just
USER agent
WORKDIR /home/agent

# Install Rust for the agent user
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable && \
    . $HOME/.cargo/env && \
    cargo install just

# Switch back to root for remaining setup
USER root

# Set up working directory
WORKDIR /workspace

# Switch to agent user
USER agent

# Build arguments for git configuration
ARG GIT_USER_NAME
ARG GIT_USER_EMAIL

# Configure git with the settings from build arguments
RUN git config --global user.name "$GIT_USER_NAME" && \
    git config --global user.email "$GIT_USER_EMAIL"

# Set environment variables
ENV PATH="/home/agent/.cargo/bin:/home/agent/.local/bin:${PATH}"
ENV PYTHONUNBUFFERED=1

# Default command (will be overridden by agent setup)
CMD ["/bin/bash"]

FROM ubuntu:24.04

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install system dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    ca-certificates \
    curl \
    dnsutils \
    git \
    gnupg \
    iputils-ping \
    jq \
    just \
    openssh-client \
    python3 \
    python3-pip \
    ripgrep \
    sudo \
    unzip \
    vim \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js 20.x
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs

RUN npm install -g @anthropic-ai/claude-code

# Ubuntu 24.04 already has a ubuntu user with UID/GID 1000
# We'll rename it to agent and ensure proper permissions
RUN usermod -l agent ubuntu && \
    groupmod -n agent ubuntu && \
    usermod -d /home/agent -m agent

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

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
    iptables \
    ipset \
    iproute2 \
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

# Copy and set up scripts
COPY init-firewall.sh /usr/local/bin/
COPY entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/init-firewall.sh /usr/local/bin/entrypoint.sh && \
    # Ensure scripts are owned by root
    chown root:root /usr/local/bin/init-firewall.sh /usr/local/bin/entrypoint.sh

# Note: We don't switch to agent user here anymore.
# The container will start as root via the entrypoint script,
# which handles firewall setup and user switching.

# Set environment variables
ENV PATH="/home/agent/.cargo/bin:/home/agent/.local/bin:${PATH}"
ENV PYTHONUNBUFFERED=1

# Default command (will be overridden by agent setup)
CMD ["/bin/bash"]

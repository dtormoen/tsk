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
    zip \
    && rm -rf /var/lib/apt/lists/*

# Ubuntu 24.04 already has a ubuntu user with UID/GID 1000
# We'll rename it to agent and ensure proper permissions
RUN usermod -l agent ubuntu && \
    groupmod -n agent ubuntu && \
    usermod -d /home/agent -m agent

# Set up working directory
WORKDIR /workspace
RUN chown -R agent:agent /workspace

# Set environment variables
ENV PYTHONUNBUFFERED=1
ENV PATH="/home/agent/.local/bin:${PATH}"

USER agent
RUN curl -LsSf https://astral.sh/uv/install.sh | sh

# Stack layer
{{{STACK}}}
# End of Stack layer

# Project layer
{{{PROJECT}}}
# End of Project layer

# Build arguments for git configuration
ARG GIT_USER_NAME
ARG GIT_USER_EMAIL

# Configure git with the settings from build arguments
USER agent
RUN git config --global user.name "$GIT_USER_NAME" && \
    git config --global user.email "$GIT_USER_EMAIL"

# Agent version ARG to invalidate cache when agent updates
ARG TSK_AGENT_VERSION

# Agent layer
{{{AGENT}}}
# End of Agent layer

USER agent
ENV HOME="/home/agent"
ENV USER="agent"

# Default command (will be overridden by agent setup)
CMD ["/bin/bash"]

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
    ripgrep \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user for running the agent with UID/GID 1000
# This helps with file permissions when mounting volumes
RUN groupadd -g 1000 agent && \
    useradd -m -s /bin/bash -u 1000 -g agent agent

# Set up working directory
WORKDIR /workspace
RUN chown -R agent:agent /workspace

# Build arguments for git configuration
ARG GIT_USER_NAME
ARG GIT_USER_EMAIL

# Configure git with the settings from build arguments
USER agent
RUN git config --global user.name "$GIT_USER_NAME" && \
    git config --global user.email "$GIT_USER_EMAIL"

# Set environment variables
ENV PYTHONUNBUFFERED=1

# Default command (will be overridden by agent setup)
CMD ["/bin/bash"]

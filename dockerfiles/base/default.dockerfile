FROM ubuntu:24.04

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Configure apt proxy if building through a proxy (e.g., Podman builds inside TSK containers).
# This is a no-op when building on host machines without proxy env vars.
RUN if [ -n "${http_proxy:-}" ]; then \
        echo "Acquire::http::Proxy \"${http_proxy}\";" > /etc/apt/apt.conf.d/99proxy; \
        echo "Acquire::https::Proxy \"${http_proxy}\";" >> /etc/apt/apt.conf.d/99proxy; \
    fi

# Install system dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    dnsutils \
    git \
    gnupg \
    iputils-ping \
    jq \
    just \
    libcap2-bin `# DIND: file capabilities for newuidmap/newgidmap` \
    openssh-client \
    podman `# DIND: nested container builds` \
    python3 \
    python3-pip \
    ripgrep \
    strace `# DIND: debugging syscall-level issues in nested containers` \
    sudo \
    uidmap `# DIND: user namespace mapping for rootless Podman` \
    unzip \
    vim \
    zip \
    && rm -rf /var/lib/apt/lists/*

# Ubuntu 24.04 already has a ubuntu user with UID/GID 1000
# We'll rename it to agent and ensure proper permissions
RUN usermod -l agent ubuntu && \
    groupmod -n agent ubuntu && \
    usermod -d /home/agent -m agent

# Configure rootless Podman for nested container support (DIND).
# 1. Subordinate UID/GID ranges for user namespace mapping
# 2. File capabilities on newuidmap/newgidmap instead of SUID bits.
#    File caps run as the calling user (not root), so the user can open
#    /proc/<pid>/uid_map (owner permission) without CAP_DAC_OVERRIDE.
#    This is the standard approach used by Fedora/RHEL since 2018.
RUN echo "agent:100000:65536" >> /etc/subuid && \
    echo "agent:100000:65536" >> /etc/subgid && \
    chmod u-s /usr/bin/newuidmap /usr/bin/newgidmap && \
    setcap cap_setuid+ep /usr/bin/newuidmap && \
    setcap cap_setgid+ep /usr/bin/newgidmap

# Configure Podman runtime and storage for nested containers:
# containers.conf:
#   - default_sysctls = []: Prevents crun from writing to read-only /proc/sys
#   - netns = "host": Nested containers share the agent container's network namespace,
#     allowing them to reach tsk-proxy for internet access through the Squid proxy
#   - pidns = "host": Avoids mounting a new /proc (blocked inside containers)
#   - cgroup_manager = "cgroupfs": Uses cgroupfs directly (no systemd inside containers)
#   - build_isolation = "chroot": Avoids user namespace issues during image builds
# storage.conf:
#   - overlay driver using native kernel overlay on tmpfs (mounted at runtime)
#   - Explicit graphroot pins storage to the tmpfs mount point so that overriding
#     XDG_DATA_HOME (e.g., in integration tests) doesn't move storage onto the
#     outer container's overlayfs, where native overlay fails with EINVAL on exec.
#   - No mount_program — native overlay works because the storage path is on
#     tmpfs, not the outer container's overlayfs. Previously used fuse-overlayfs
#     but execve() fails on FUSE mounts in user namespaces on LinuxKit 6.10.14+.
#     Revert to fuse-overlayfs once docker/for-mac#7413 is fixed.
RUN mkdir -p /home/agent/.config/containers && \
    printf '[containers]\ndefault_sysctls = []\nnetns = "host"\npidns = "host"\n\n[engine]\ncgroup_manager = "cgroupfs"\nbuild_isolation = "chroot"\n' \
    > /home/agent/.config/containers/containers.conf && \
    printf '[storage]\ndriver = "overlay"\ngraphroot = "/home/agent/.local/share/containers/storage"\n' \
    > /home/agent/.config/containers/storage.conf && \
    chown -R agent:agent /home/agent/.config

# Alias docker to podman so agents and scripts that use `docker` commands work seamlessly
RUN ln -s /usr/bin/podman /usr/local/bin/docker

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

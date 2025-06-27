# Python tech stack layer

# Already running as agent user from base layer
WORKDIR /home/agent

# Python3 and pip are already installed in base layer
# Upgrade pip and install common Python development tools
RUN python3 -m pip install --user --upgrade pip setuptools wheel && \
    python3 -m pip install --user pipx && \
    python3 -m pipx ensurepath

# Add pipx binaries to PATH
ENV PATH="/home/agent/.local/bin:${PATH}"

# Install common Python tools via pipx for isolation
RUN pipx install poetry && \
    pipx install black && \
    pipx install ruff && \
    pipx install mypy && \
    pipx install pytest && \
    pipx install ipython

# Install uv for fast Python package management
RUN curl -LsSf https://astral.sh/uv/install.sh | sh

# Set Python environment variables
ENV PYTHONPATH="/workspace:${PYTHONPATH}"
ENV PIP_USER=1

# Switch back to workspace directory
WORKDIR /workspace
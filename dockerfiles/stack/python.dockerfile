# Python tech stack layer

# Create a virtual environment outside of workspace using uv
# This avoids conflicts with workspace files and keeps the environment clean
RUN uv venv /home/agent/.venv

# Activate the virtual environment by updating PATH and setting VIRTUAL_ENV
ENV VIRTUAL_ENV=/home/agent/.venv
ENV PATH="/home/agent/.venv/bin:${PATH}"

# Install common Python development tools
RUN uv pip install \
    pytest \
    pip \
    black \
    ruff \
    ty \
    mypy \
    poetry

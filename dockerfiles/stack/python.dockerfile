# Python tech stack layer

# Create a virtual environment outside of workspace using uv
# This avoids conflicts with workspace files and keeps the environment clean
RUN uv venv /home/agent/.venv

# Activate the virtual environment by updating PATH and setting VIRTUAL_ENV
ENV VIRTUAL_ENV=/home/agent/.venv
ENV PATH="/home/agent/.venv/bin:${PATH}"

# Install common Python development dependencies using uv
# These are commonly needed tools for Python development
RUN uv pip install \
    pytest \
    pytest-cov \
    pytest-asyncio \
    pytest-mock \
    pip \
    black \
    ruff \
    ty \
    mypy \
    ipython \
    ipdb \
    requests \
    httpx \
    pydantic \
    typing-extensions \
    python-dotenv \
    rich \
    click \
    tqdm

# Install Poetry separately as it's a package manager that some projects might use
RUN uv pip install poetry

# Set Python environment variables
ENV PYTHONUNBUFFERED=1

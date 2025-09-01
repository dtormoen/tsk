# Rust tech stack layer

# Already running as agent user from base layer
WORKDIR /home/agent

# Install Rust for the agent user
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable && \
    . $HOME/.cargo/env && \
    cargo install just

# Add Rust to PATH
ENV PATH="/home/agent/.cargo/bin:${PATH}"

# Set CARGO_TARGET_DIR to store build artifacts outside of workspace
ENV CARGO_TARGET_DIR="/home/agent/.cargo/target"

# Switch back to workspace directory
WORKDIR /workspace
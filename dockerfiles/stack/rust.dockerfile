# Rust tech stack layer

# Install Rust for agent user
RUN curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable

# Add Rust to PATH
ENV PATH="/home/agent/.cargo/bin:${PATH}"

# Set CARGO_TARGET_DIR to store build artifacts outside of workspace
ENV CARGO_TARGET_DIR="/home/agent/.cargo/target"

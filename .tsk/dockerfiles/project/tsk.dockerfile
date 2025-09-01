# Project-specific layer for tsk

# TSK is a Rust project, so we'll optimize dependency compilation
# The Rust toolchain is already installed in the tech-stack layer

# Copy dependency files for caching
COPY Cargo.toml Cargo.lock ./

# Create dummy source files to build dependencies
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    echo "pub fn lib() {}" > src/lib.rs

# Build dependencies in release mode
# This will compile all dependencies specified in Cargo.toml
# The compiled artifacts will be cached in $CARGO_TARGET_DIR (set in tech-stack layer)
RUN cargo build

# Remove dummy source files
# The actual source code will be mounted at runtime
RUN rm -rf src


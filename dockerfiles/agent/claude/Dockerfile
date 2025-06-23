# Claude agent layer

# Switch to root to install Node.js and Claude Code
USER root

# Install Node.js 20.x (required by Claude Code)
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs

# Install Claude Code CLI
RUN npm install -g @anthropic-ai/claude-code

# Switch back to agent user
USER agent

# Add local bin to PATH for claude
ENV PATH="/home/agent/.local/bin:${PATH}"

# Claude-specific environment
ENV HOME="/home/agent"
ENV USER="agent"
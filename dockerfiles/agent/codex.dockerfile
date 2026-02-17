# Codex agent layer

# Switch to root temporarily for system package installation
USER root

# Install Node.js 20.x (required by Codex)
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs

# Switch back to agent user
USER agent

# Create npm global directory for agent user and configure npm
RUN mkdir -p /home/agent/.npm-global && \
    npm config set prefix "/home/agent/.npm-global" && \
    echo 'export PATH=/home/agent/.npm-global/bin:$PATH' >> /home/agent/.bashrc

# Install Codex CLI to agent's global directory
RUN npm install -g @openai/codex

# Add npm global binaries and local bin to PATH
ENV PATH="/home/agent/.npm-global/bin:/home/agent/.local/bin:${PATH}"

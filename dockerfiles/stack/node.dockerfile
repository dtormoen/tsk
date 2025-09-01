# Node.js tech stack layer

# Switch to root temporarily for system package installation
USER root

# Install Node.js via NodeSource repository (LTS version)
RUN curl -fsSL https://deb.nodesource.com/setup_lts.x | bash - && \
    apt-get install -y nodejs && \
    rm -rf /var/lib/apt/lists/*

# Switch back to agent user
USER agent

# Create npm global directory for agent user and configure npm
RUN mkdir -p /home/agent/.npm-global && \
    npm config set prefix "/home/agent/.npm-global" && \
    echo 'export PATH=/home/agent/.npm-global/bin:$PATH' >> /home/agent/.bashrc

# Install pnpm globally for faster package management
RUN npm install -g pnpm@latest

# Install common Node.js development tools globally
RUN npm install -g \
    yarn \
    typescript \
    ts-node \
    nodemon \
    eslint \
    prettier \
    jest \
    npm-check-updates

# Add npm global binaries to PATH
ENV PATH="/home/agent/.npm-global/bin:${PATH}"

# Set Node environment variables
ENV NODE_ENV=development

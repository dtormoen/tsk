# Node.js tech stack layer

# Already running as agent user from base layer
WORKDIR /home/agent

# Install Node.js via NodeSource repository (LTS version)
RUN curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash - && \
    sudo apt-get install -y nodejs && \
    sudo rm -rf /var/lib/apt/lists/*

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

# Create npm global directory for agent user
RUN mkdir -p ~/.npm-global && \
    npm config set prefix '~/.npm-global' && \
    echo 'export PATH=$HOME/.npm-global/bin:$PATH' >> ~/.bashrc

# Add npm global binaries to PATH
ENV PATH="/home/agent/.npm-global/bin:${PATH}"

# Set Node environment variables
ENV NODE_ENV=development

# Switch back to workspace directory
WORKDIR /workspace
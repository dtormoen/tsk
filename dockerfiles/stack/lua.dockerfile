# Lua tech stack layer for Neovim plugin development

# Switch to root temporarily for system package installation
USER root

# Install LuaJIT (preferred for Neovim) and development tools
RUN apt-get update && \
    apt-get install -y \
    luajit \
    libluajit-5.1-dev \
    lua5.1 \
    liblua5.1-dev \
    luarocks \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Configure LuaRocks for agent user
RUN mkdir -p /home/agent/.luarocks

# Install Rust and cargo for stylua (for agent user)
RUN curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
RUN . /home/agent/.cargo/env && cargo install stylua

# Install Neovim plugin development tools using luarocks for agent user
RUN luarocks --local install luacheck && \
    luarocks --local install ldoc && \
    luarocks --local install busted && \
    luarocks --local install luassert && \
    luarocks --local install luafilesystem && \
    luarocks --local install penlight && \
    luarocks --local install inspect && \
    luarocks --local install nlua

# Install Neovim (latest stable version)
RUN apt-get update && \
    apt-get install -y software-properties-common && \
    add-apt-repository ppa:neovim-ppa/stable && \
    apt-get update && \
    apt-get install -y neovim && \
    rm -rf /var/lib/apt/lists/*

# Switch back to agent user
USER agent

# Set up LuaRocks paths for agent user
ENV PATH="/home/agent/.cargo/bin:/home/agent/.luarocks/bin:${PATH}"
ENV LUA_PATH="/home/agent/.luarocks/share/lua/5.1/?.lua;/home/agent/.luarocks/share/lua/5.1/?/init.lua;;"
ENV LUA_CPATH="/home/agent/.luarocks/lib/lua/5.1/?.so;;"


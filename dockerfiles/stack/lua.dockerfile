# Lua tech stack layer for Neovim plugin development

# Switch to root temporarily for system package installation
USER root

# Install Neovim, LuaJIT, and development tools
RUN apt-get update && \
    apt-get install -y --no-install-recommends software-properties-common && \
    add-apt-repository ppa:neovim-ppa/stable && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
    neovim \
    luajit \
    libluajit-5.1-dev \
    lua5.1 \
    liblua5.1-dev \
    luarocks \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Install stylua from prebuilt binary
RUN ARCH=$(dpkg --print-architecture) && \
    case "$ARCH" in \
        amd64) STYLUA_ARCH="linux-x86_64" ;; \
        arm64) STYLUA_ARCH="linux-aarch64" ;; \
    esac && \
    curl -fsSL "https://github.com/JohnnyMorganz/StyLua/releases/latest/download/stylua-${STYLUA_ARCH}.zip" -o /tmp/stylua.zip && \
    unzip /tmp/stylua.zip -d /usr/local/bin && \
    chmod +x /usr/local/bin/stylua && \
    rm /tmp/stylua.zip

# Switch back to agent user
USER agent

# Configure LuaRocks for agent user
RUN mkdir -p /home/agent/.luarocks

# Install Neovim plugin development tools
RUN luarocks --local install luacheck && \
    luarocks --local install busted && \
    luarocks --local install luassert && \
    luarocks --local install luafilesystem && \
    luarocks --local install nlua

# Set up LuaRocks paths for agent user
ENV LUA_PATH="./?.lua;/usr/local/share/lua/5.1/?.lua;/usr/local/share/lua/5.1/?/init.lua;/usr/local/lib/lua/5.1/?.lua;/usr/local/lib/lua/5.1/?/init.lua;/usr/share/lua/5.1/?.lua;/usr/share/lua/5.1/?/init.lua;/home/agent/.luarocks/share/lua/5.1/?.lua;/home/agent/.luarocks/share/lua/5.1/?/init.lua;;"
ENV LUA_CPATH="./?.so;/usr/local/lib/lua/5.1/?.so;/usr/lib/aarch64-linux-gnu/lua/5.1/?.so;/usr/lib/lua/5.1/?.so;/usr/local/lib/lua/5.1/loadall.so;/home/agent/.luarocks/lib/lua/5.1/?.so;;"
ENV PATH="/home/agent/.luarocks/bin:${PATH}"

# Lua tech stack layer

# Already running as agent user from base layer
WORKDIR /home/agent

# Install Lua 5.4 and LuaRocks
RUN sudo apt-get update && \
    sudo apt-get install -y \
    lua5.4 \
    liblua5.4-dev \
    luarocks \
    && sudo rm -rf /var/lib/apt/lists/*

# Create symbolic links for default lua commands
RUN sudo update-alternatives --install /usr/bin/lua lua /usr/bin/lua5.4 100 && \
    sudo update-alternatives --install /usr/bin/luac luac /usr/bin/luac5.4 100

# Configure LuaRocks to install to user directory
RUN luarocks init --lua-version=5.4

# Install common Lua development tools
RUN luarocks install --local luacheck && \
    luarocks install --local luaformatter && \
    luarocks install --local ldoc && \
    luarocks install --local busted && \
    luarocks install --local penlight

# Add LuaRocks binaries to PATH
ENV PATH="/home/agent/.luarocks/bin:${PATH}"
ENV LUA_PATH="/home/agent/.luarocks/share/lua/5.4/?.lua;/home/agent/.luarocks/share/lua/5.4/?/init.lua;${LUA_PATH}"
ENV LUA_CPATH="/home/agent/.luarocks/lib/lua/5.4/?.so;${LUA_CPATH}"

# Switch back to workspace directory
WORKDIR /workspace
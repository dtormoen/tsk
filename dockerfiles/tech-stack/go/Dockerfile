# Go tech stack layer

# Already running as agent user from base layer
WORKDIR /home/agent

# Install Go
ENV GO_VERSION=1.23.4
RUN curl -fsSL https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz | tar -xz -C /home/agent && \
    echo 'export PATH=$HOME/go/bin:$PATH' >> ~/.bashrc && \
    echo 'export GOPATH=$HOME/gopath' >> ~/.bashrc && \
    echo 'export PATH=$GOPATH/bin:$PATH' >> ~/.bashrc

# Add Go to PATH
ENV PATH="/home/agent/go/bin:/home/agent/gopath/bin:${PATH}"
ENV GOPATH="/home/agent/gopath"

# Install common Go tools
RUN go install golang.org/x/tools/gopls@latest && \
    go install github.com/go-delve/delve/cmd/dlv@latest && \
    go install golang.org/x/tools/cmd/goimports@latest && \
    go install honnef.co/go/tools/cmd/staticcheck@latest

# Switch back to workspace directory
WORKDIR /workspace
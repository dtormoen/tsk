# Go tech stack layer

# Switch to root temporarily for system installation
USER root

# Install Go
ENV GO_VERSION=1.25.0
RUN curl -fsSL https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz | tar -xz -C /usr/local && \
    ln -s /usr/local/go/bin/go /usr/local/bin/go && \
    ln -s /usr/local/go/bin/gofmt /usr/local/bin/gofmt

# Switch back to agent user
USER agent

# Set up Go workspace and environment for agent user
RUN mkdir -p /home/agent/gopath && \
    echo 'export PATH=/usr/local/go/bin:$PATH' >> /home/agent/.bashrc && \
    echo 'export GOPATH=/home/agent/gopath' >> /home/agent/.bashrc && \
    echo 'export PATH=$GOPATH/bin:$PATH' >> /home/agent/.bashrc

# Add Go to PATH
ENV PATH="/usr/local/go/bin:/home/agent/gopath/bin:${PATH}"
ENV GOPATH="/home/agent/gopath"
ENV CGO_ENABLED=0

# Install common Go tools
RUN go install golang.org/x/tools/gopls@latest && \
    go install github.com/go-delve/delve/cmd/dlv@latest && \
    go install golang.org/x/tools/cmd/goimports@latest && \
    go install honnef.co/go/tools/cmd/staticcheck@latest

# Go tech stack layer

# Switch to root temporarily for system installation
USER root

# Install Go
ENV GO_VERSION=1.25.0
RUN GOARCH=$(dpkg --print-architecture) && \
    curl -fsSL https://go.dev/dl/go${GO_VERSION}.linux-${GOARCH}.tar.gz | tar -xz -C /usr/local && \
    ln -s /usr/local/go/bin/go /usr/local/bin/go && \
    ln -s /usr/local/go/bin/gofmt /usr/local/bin/gofmt

# Switch back to agent user
USER agent

# Set up Go environment
RUN mkdir -p /home/agent/gopath
ENV PATH="/usr/local/go/bin:/home/agent/gopath/bin:${PATH}"
ENV GOPATH="/home/agent/gopath"
ENV CGO_ENABLED=0

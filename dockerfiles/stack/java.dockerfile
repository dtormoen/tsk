# Java tech stack layer

# Switch to root temporarily for system package installation
USER root

# Install OpenJDK 17 (LTS) and Maven
RUN apt-get update && \
    apt-get install -y --no-install-recommends openjdk-17-jdk maven gradle && \
    rm -rf /var/lib/apt/lists/*

# Set JAVA_HOME via symlink to handle any architecture (amd64/arm64)
RUN ln -sfn "$(dirname "$(readlink -f /usr/bin/javac)")/.." /usr/lib/jvm/java-17-openjdk
ENV JAVA_HOME=/usr/lib/jvm/java-17-openjdk
ENV PATH="${JAVA_HOME}/bin:${PATH}"

# Switch back to agent user
USER agent

# Set Maven options for better performance
ENV MAVEN_OPTS="-Xmx1024m"

# Create directories for Maven and Gradle caches
# Proxy configuration is injected at runtime via JAVA_TOOL_OPTIONS env var
# (Maven and Gradle ignore HTTP_PROXY env vars, so JVM system properties are used instead)
RUN mkdir -p ~/.m2 ~/.gradle

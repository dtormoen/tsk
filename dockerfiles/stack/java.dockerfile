# Java tech stack layer

# Switch to root temporarily for system package installation
USER root

# Install OpenJDK 17 (LTS) and Maven
RUN apt-get update && \
    apt-get install -y openjdk-17-jdk maven gradle && \
    rm -rf /var/lib/apt/lists/*

# Switch back to agent user
USER agent

# Set JAVA_HOME
ENV JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
ENV PATH="${JAVA_HOME}/bin:${PATH}"

# Install SDKMAN for managing Java tools
RUN export SDKMAN_DIR="/home/agent/.sdkman" && \
    curl -s "https://get.sdkman.io" | bash && \
    bash -c "source /home/agent/.sdkman/bin/sdkman-init.sh && \
    sdk install kotlin && \
    sdk install groovy"

# Add SDKMAN to PATH
ENV PATH="/home/agent/.sdkman/candidates/kotlin/current/bin:/home/agent/.sdkman/candidates/groovy/current/bin:${PATH}"

# Set Maven options for better performance
ENV MAVEN_OPTS="-Xmx1024m -XX:MaxPermSize=256m"

# Create directories for Maven and Gradle caches
RUN mkdir -p ~/.m2 ~/.gradle

# Java tech stack layer

# Already running as agent user from base layer
WORKDIR /home/agent

# Install OpenJDK 17 (LTS) and Maven
RUN sudo apt-get update && \
    sudo apt-get install -y openjdk-17-jdk maven gradle && \
    sudo rm -rf /var/lib/apt/lists/*

# Set JAVA_HOME
ENV JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
ENV PATH="${JAVA_HOME}/bin:${PATH}"

# Install SDKMAN for managing Java tools
RUN curl -s "https://get.sdkman.io" | bash

# Source SDKMAN and install additional tools
RUN bash -c "source $HOME/.sdkman/bin/sdkman-init.sh && \
    sdk install kotlin && \
    sdk install groovy"

# Add SDKMAN to PATH
ENV PATH="/home/agent/.sdkman/candidates/kotlin/current/bin:/home/agent/.sdkman/candidates/groovy/current/bin:${PATH}"

# Set Maven options for better performance
ENV MAVEN_OPTS="-Xmx1024m -XX:MaxPermSize=256m"

# Create directories for Maven and Gradle caches
RUN mkdir -p ~/.m2 ~/.gradle

# Switch back to workspace directory
WORKDIR /workspace
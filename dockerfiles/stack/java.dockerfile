# Java tech stack layer

# Switch to root temporarily for system package installation
USER root

# Install OpenJDK 17 (LTS) and Maven
RUN apt-get update && \
    apt-get install -y openjdk-17-jdk maven gradle && \
    rm -rf /var/lib/apt/lists/*

# Set JAVA_HOME via symlink to handle any architecture (amd64/arm64)
RUN ln -sfn "$(dirname "$(readlink -f /usr/bin/javac)")/.." /usr/lib/jvm/java-17-openjdk
ENV JAVA_HOME=/usr/lib/jvm/java-17-openjdk
ENV PATH="${JAVA_HOME}/bin:${PATH}"

# Switch back to agent user
USER agent

# Install SDKMAN for managing Java tools
RUN export SDKMAN_DIR="/home/agent/.sdkman" && \
    curl -s "https://get.sdkman.io" | bash && \
    bash -c "source /home/agent/.sdkman/bin/sdkman-init.sh && \
    sdk install kotlin && \
    sdk install groovy"

# Add SDKMAN to PATH
ENV PATH="/home/agent/.sdkman/candidates/kotlin/current/bin:/home/agent/.sdkman/candidates/groovy/current/bin:${PATH}"

# Set Maven options for better performance
ENV MAVEN_OPTS="-Xmx1024m"

# Create directories for Maven and Gradle caches and configure proxy
# TSK containers always route traffic through tsk-proxy:3128 (Squid proxy)
# Maven and Gradle don't respect HTTP_PROXY env vars, so we configure them explicitly
RUN mkdir -p ~/.m2 ~/.gradle && \
    printf '<settings>\n  <proxies>\n    <proxy>\n      <id>tsk-http</id>\n      <active>true</active>\n      <protocol>http</protocol>\n      <host>tsk-proxy</host>\n      <port>3128</port>\n      <nonProxyHosts>localhost|127.0.0.1</nonProxyHosts>\n    </proxy>\n    <proxy>\n      <id>tsk-https</id>\n      <active>true</active>\n      <protocol>https</protocol>\n      <host>tsk-proxy</host>\n      <port>3128</port>\n      <nonProxyHosts>localhost|127.0.0.1</nonProxyHosts>\n    </proxy>\n  </proxies>\n</settings>\n' > ~/.m2/settings.xml && \
    printf 'systemProp.http.proxyHost=tsk-proxy\nsystemProp.http.proxyPort=3128\nsystemProp.http.nonProxyHosts=localhost|127.0.0.1\nsystemProp.https.proxyHost=tsk-proxy\nsystemProp.https.proxyPort=3128\nsystemProp.https.nonProxyHosts=localhost|127.0.0.1\n' > ~/.gradle/gradle.properties

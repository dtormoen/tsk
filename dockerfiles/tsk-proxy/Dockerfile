FROM alpine:latest

RUN apk add --no-cache squid curl socat

# Copy our squid configuration and entrypoint script
COPY squid.conf /etc/squid/squid.conf
COPY entrypoint.sh /entrypoint.sh

# Create necessary directories with proper permissions
RUN mkdir -p /var/cache/squid /var/log/squid /var/run/squid && \
    chown -R squid:squid /var/cache/squid /var/log/squid /var/run/squid && \
    chmod +x /entrypoint.sh

# Expose the proxy port
EXPOSE 3128

# Add health check - Squid responds with 400 to direct requests which is fine
# We just need to verify the port is responding
HEALTHCHECK --interval=2s --timeout=3s --start-period=10s --retries=3 \
    CMD nc -z localhost 3128 || exit 1

# Run squid in foreground as squid user
USER squid
ENTRYPOINT ["/entrypoint.sh"]
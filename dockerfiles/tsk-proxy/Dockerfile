FROM alpine:latest

RUN apk add --no-cache squid socat iptables su-exec

# Copy our squid configuration and entrypoint script
COPY squid.conf /etc/squid/squid.conf
COPY entrypoint.sh /entrypoint.sh

# Create necessary directories with proper permissions
RUN mkdir -p /var/cache/squid /var/log/squid /var/run/squid && \
    chown -R squid:squid /var/cache/squid /var/log/squid /var/run/squid && \
    chmod +x /entrypoint.sh

# Expose the proxy port
EXPOSE 3128

# Run entrypoint as root for iptables setup, then drop to squid user via su-exec
ENTRYPOINT ["/entrypoint.sh"]
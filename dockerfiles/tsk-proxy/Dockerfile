FROM alpine:latest

RUN apk add --no-cache squid

# Copy our squid configuration
COPY squid.conf /etc/squid/squid.conf

# Create necessary directories with proper permissions
RUN mkdir -p /var/cache/squid /var/log/squid /var/run/squid && \
    chown -R squid:squid /var/cache/squid /var/log/squid /var/run/squid

# Expose the proxy port
EXPOSE 3128

# Run squid in foreground as squid user
USER squid
CMD ["squid", "-N", "-f", "/etc/squid/squid.conf"]
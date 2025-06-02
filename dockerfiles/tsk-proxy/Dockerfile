FROM alpine:latest

RUN apk add --no-cache squid

# Copy our squid configuration
COPY squid.conf /etc/squid/squid.conf

# Create cache directories
RUN mkdir -p /var/cache/squid && \
    chown -R squid:squid /var/cache/squid

# Expose the proxy port
EXPOSE 3128

# Run squid in foreground
CMD ["squid", "-N", "-f", "/etc/squid/squid.conf"]
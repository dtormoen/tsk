#!/bin/sh
set -e

# Fix ownership of tmpfs mounts (they're mounted as root, but squid needs to write)
chown squid:squid /var/cache/squid /var/log/squid /var/run/squid

# Clean up any stale PID file from previous runs
if [ -f /var/run/squid/squid.pid ]; then
    echo "Removing stale PID file from previous run..."
    rm -f /var/run/squid/squid.pid
fi

# Configure iptables firewall rules (must be done as root before dropping privileges)
# This blocks all incoming connections except Squid (3128) and configured host service ports
echo "Configuring firewall rules..."

# IPv4 rules
iptables -A INPUT -i lo -j ACCEPT
iptables -A INPUT -m state --state ESTABLISHED,RELATED -j ACCEPT
iptables -A INPUT -p tcp --dport 3128 -j ACCEPT
if [ -n "$TSK_HOST_SERVICES" ]; then
    for port in $(echo "$TSK_HOST_SERVICES" | tr ',' ' '); do
        iptables -A INPUT -p tcp --dport "$port" -j ACCEPT
    done
fi
iptables -A INPUT -j DROP

# IPv6 rules (defense-in-depth)
if command -v ip6tables >/dev/null 2>&1; then
    ip6tables -A INPUT -i lo -j ACCEPT
    ip6tables -A INPUT -m state --state ESTABLISHED,RELATED -j ACCEPT
    ip6tables -A INPUT -p tcp --dport 3128 -j ACCEPT
    if [ -n "$TSK_HOST_SERVICES" ]; then
        for port in $(echo "$TSK_HOST_SERVICES" | tr ',' ' '); do
            ip6tables -A INPUT -p tcp --dport "$port" -j ACCEPT
        done
    fi
    ip6tables -A INPUT -j DROP
fi

# Start TCP forwarders for host services if configured (as squid user for security)
# Note: Ports must be >= 1024 since we run as non-root
if [ -n "$TSK_HOST_SERVICES" ]; then
    echo "Starting TCP forwarders for host services: $TSK_HOST_SERVICES"
    for port in $(echo "$TSK_HOST_SERVICES" | tr ',' ' '); do
        echo "  Forwarding port $port -> host.docker.internal:$port"
        su-exec squid socat TCP-LISTEN:$port,fork,reuseaddr TCP:host.docker.internal:$port &
    done
fi

# Start squid in foreground mode as squid user (drop privileges via su-exec)
exec su-exec squid squid -N -f /etc/squid/squid.conf

#!/bin/sh
set -e

# Clean up any stale PID file from previous runs
if [ -f /var/run/squid/squid.pid ]; then
    echo "Removing stale PID file from previous run..."
    rm -f /var/run/squid/squid.pid
fi

# Start TCP forwarders for host services if configured
if [ -n "$TSK_HOST_SERVICES" ]; then
    echo "Starting TCP forwarders for host services: $TSK_HOST_SERVICES"
    for port in $(echo "$TSK_HOST_SERVICES" | tr ',' ' '); do
        echo "  Forwarding port $port -> host.docker.internal:$port"
        socat TCP-LISTEN:$port,fork,reuseaddr TCP:host.docker.internal:$port &
    done
fi

# Start squid in foreground mode
exec squid -N -f /etc/squid/squid.conf

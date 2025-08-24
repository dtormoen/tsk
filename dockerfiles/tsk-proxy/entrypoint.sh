#!/bin/sh
set -e

# Clean up any stale PID file from previous runs
if [ -f /var/run/squid/squid.pid ]; then
    echo "Removing stale PID file from previous run..."
    rm -f /var/run/squid/squid.pid
fi

# Start squid in foreground mode
exec squid -N -f /etc/squid/squid.conf
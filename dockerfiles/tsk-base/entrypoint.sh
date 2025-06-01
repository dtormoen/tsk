#!/bin/bash
set -euo pipefail

# This script runs as root, sets up the firewall, ensures proper permissions,
# then switches to the agent user to run the actual command

# Debug information
echo "Running as user: $(whoami) (UID: $(id -u))"
echo "Current directory: $(pwd)"

# Check if the firewall script exists and is executable
if [ ! -x /usr/local/bin/init-firewall.sh ]; then
    echo "ERROR: /usr/local/bin/init-firewall.sh is not executable or doesn't exist"
    ls -la /usr/local/bin/init-firewall.sh || echo "File not found"
    exit 1
fi

# Run the firewall initialization script
echo "Initializing firewall rules..."
/usr/local/bin/init-firewall.sh || {
    echo "ERROR: Firewall initialization failed with exit code $?"
    exit 1
}

# Ensure the agent user can write to the workspace
# This is important when the host directory is mounted
chown -R agent:agent /workspace 2>/dev/null || true

# If no command provided, just exit
if [ $# -eq 0 ]; then
    echo "No command provided, exiting."
    exit 0
fi

# Switch to agent user and run the command
# Use -p to preserve environment variables
echo "Switching to agent user and running command..."
# Properly escape the command for su -c
# Build the command string with proper quoting
CMD="cd /workspace && exec"
for arg in "$@"; do
    # Escape single quotes in the argument
    escaped_arg=$(echo "$arg" | sed "s/'/'\\\\''/g")
    CMD="$CMD '$escaped_arg'"
done

# Use exec to replace this process with su, ensuring proper signal handling
exec su -p agent -c "$CMD"
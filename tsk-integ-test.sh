#!/bin/bash
set -euo pipefail

# Integration test script for running TSK inside a TSK container.
# Used by the integ agent for nested integration tests (podman-in-docker,
# podman-in-podman).

just integration-test

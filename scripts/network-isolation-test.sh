#!/usr/bin/env bash
# TSK Security Test Script
# Tests network isolation in agent containers
#
# This script verifies that the TSK container network isolation is working correctly.
# It tests various connection methods to ensure:
# 1. Allowed domains are accessible through the proxy
# 2. Blocked domains are not accessible through the proxy
# 3. Direct connections (bypassing proxy) are blocked
# 4. Various network protocols (TCP, UDP, ICMP, SSH) are properly restricted
#
# Exit codes:
# 0 - All security tests passed (isolation is working)
# 1 - One or more security tests failed (potential security issue)

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Counters
TESTS_PASSED=0
TESTS_FAILED=0
TESTS_TOTAL=0

# Test timeout in seconds
TIMEOUT=5

# Test result tracking
declare -a FAILED_TESTS=()

# Temp file for capturing test output
_TEST_OUTPUT=$(mktemp)
trap 'rm -f "$_TEST_OUTPUT"' EXIT

# Print functions
print_header() {
    echo -e "\n${BLUE}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
}

print_test() {
    echo -e "\n${YELLOW}Testing:${NC} $1"
}

print_pass() {
    echo -e "  ${GREEN}[PASS]${NC} $1"
    TESTS_PASSED=$((TESTS_PASSED + 1))
    TESTS_TOTAL=$((TESTS_TOTAL + 1))
}

print_fail() {
    echo -e "  ${RED}[FAIL]${NC} $1"
    TESTS_FAILED=$((TESTS_FAILED + 1))
    TESTS_TOTAL=$((TESTS_TOTAL + 1))
    FAILED_TESTS+=("$1")
}

print_info() {
    echo -e "  ${BLUE}[INFO]${NC} $1"
}

# Run a test, suppressing output if it passes
# Usage: run_test <test_function> [args...]
run_test() {
    local before_failed=$TESTS_FAILED
    "$@" > "$_TEST_OUTPUT" 2>&1
    if [ "$TESTS_FAILED" -gt "$before_failed" ]; then
        cat "$_TEST_OUTPUT"
    fi
}

# Test if a command exists
command_exists() {
    command -v "$1" &> /dev/null
}

# Test HTTP/HTTPS connection through proxy
# Args: $1 = URL, $2 = expected_result ("pass" or "fail"), $3 = description
# Note: "pass" means connection should succeed (any HTTP response including 4xx)
#       "fail" means connection should be blocked (proxy deny, timeout, or connection refused)
test_http_proxy() {
    local url="$1"
    local expected="$2"
    local desc="$3"

    print_test "$desc"

    local http_code
    local result

    # Get HTTP status code - any code means connection succeeded
    # Proxy block typically shows as 403 from Squid or connection failure (000)
    http_code=$(timeout "$TIMEOUT" curl -s -o /dev/null -w "%{http_code}" "$url" 2>/dev/null) || http_code="000"

    # Check if we got a response (non-000 status code)
    # 403 from proxy means denied, 000 means connection failed/timeout
    if [ "$http_code" = "000" ] || [ "$http_code" = "403" ]; then
        result="fail"
    else
        result="pass"
    fi

    if [ "$result" = "$expected" ]; then
        print_pass "$desc - connection ${expected}ed as expected (HTTP $http_code)"
    else
        print_fail "$desc - expected $expected but got $result (HTTP $http_code)"
    fi
}

# Test direct connection (bypassing proxy)
# Args: $1 = host, $2 = port, $3 = description
test_direct_connection() {
    local host="$1"
    local port="$2"
    local desc="$3"

    print_test "$desc"

    # Try to connect directly using /dev/tcp (bash built-in)
    # The file descriptor is automatically closed when the subshell exits
    if timeout "$TIMEOUT" bash -c "exec 3<>/dev/tcp/$host/$port && exec 3>&-" 2>/dev/null; then
        print_fail "$desc - direct TCP connection succeeded (should be blocked)"
    else
        print_pass "$desc - direct TCP connection blocked as expected"
    fi
}

# Test direct HTTP connection bypassing proxy
# Args: $1 = URL, $2 = description
test_direct_http() {
    local url="$1"
    local desc="$2"

    print_test "$desc"

    # Unset proxy env vars for this test
    local result
    if timeout "$TIMEOUT" env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy \
        curl -s --noproxy '*' -o /dev/null -w "%{http_code}" "$url" 2>/dev/null | grep -qE "^(2|3)[0-9][0-9]$"; then
        result="success"
    else
        result="blocked"
    fi

    if [ "$result" = "blocked" ]; then
        print_pass "$desc - direct HTTP blocked as expected"
    else
        print_fail "$desc - direct HTTP succeeded (should be blocked)"
    fi
}

# Test DNS resolution (should fail - agents should not resolve DNS directly)
# Args: $1 = domain, $2 = description
test_dns_resolution() {
    local domain="$1"
    local desc="$2"

    print_test "$desc"

    if command_exists dig; then
        if timeout "$TIMEOUT" dig +short "$domain" 2>/dev/null | grep -qE "^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$"; then
            print_fail "$desc - DNS resolution succeeded (agents should not resolve DNS directly)"
        else
            print_pass "$desc - DNS resolution blocked as expected"
        fi
    elif command_exists nslookup; then
        if timeout "$TIMEOUT" nslookup "$domain" 2>/dev/null | grep -q "Address:"; then
            print_fail "$desc - DNS resolution succeeded (agents should not resolve DNS directly)"
        else
            print_pass "$desc - DNS resolution blocked as expected"
        fi
    else
        print_info "$desc - Could not test DNS (no dig/nslookup)"
    fi
}

# Test external DNS server access (UDP)
# Args: $1 = dns_server, $2 = description
test_external_dns() {
    local dns_server="$1"
    local desc="$2"

    print_test "$desc"

    if command_exists dig; then
        # Try to query an external DNS server directly
        if timeout "$TIMEOUT" dig @"$dns_server" +short google.com 2>/dev/null | grep -qE "^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$"; then
            print_fail "$desc - external DNS query succeeded (UDP should be blocked)"
        else
            print_pass "$desc - external DNS query blocked as expected"
        fi
    else
        print_info "$desc - Could not test (dig not available)"
    fi
}

# Test SSH connection
# Args: $1 = host, $2 = description
test_ssh_connection() {
    local host="$1"
    local desc="$2"

    print_test "$desc"

    if command_exists ssh; then
        # Try to connect with a very short timeout
        # SSH can hang, so we use a very short ConnectTimeout
        local ssh_output
        ssh_output=$(timeout 3 ssh -v -o ConnectTimeout=2 -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "$host" exit 2>&1 || true)

        # Check if connection was established (would see "Connection established" in verbose output)
        if echo "$ssh_output" | grep -qi "Connection established\|Authenticated"; then
            print_fail "$desc - SSH connection succeeded (should be blocked)"
        else
            # SSH failed to connect, which is expected
            print_pass "$desc - SSH connection blocked as expected"
        fi
    else
        print_info "$desc - Could not test (ssh not available)"
    fi
}

# Test ICMP ping
# Args: $1 = host, $2 = description
test_ping() {
    local host="$1"
    local desc="$2"

    print_test "$desc"

    if command_exists ping; then
        local ping_output
        ping_output=$(timeout 3 ping -c 1 -W 2 "$host" 2>&1 || true)

        if echo "$ping_output" | grep -q "1 received"; then
            print_fail "$desc - ping succeeded (ICMP should be blocked)"
        elif echo "$ping_output" | grep -qi "operation not permitted\|not permitted\|permission denied"; then
            print_pass "$desc - ping blocked as expected (NET_RAW capability dropped)"
        else
            print_pass "$desc - ping blocked (no response or timeout)"
        fi
    else
        print_info "$desc - Could not test (ping not available)"
    fi
}

# Test connection to non-standard port through proxy
# Args: $1 = url, $2 = description
test_nonstandard_port() {
    local url="$1"
    local desc="$2"

    print_test "$desc"

    # Proxy should deny CONNECT to non-SSL ports
    if timeout "$TIMEOUT" curl -s -o /dev/null -w "%{http_code}" "$url" 2>/dev/null | grep -qE "^(2|3)[0-9][0-9]$"; then
        print_fail "$desc - non-standard port connection succeeded (should be blocked)"
    else
        print_pass "$desc - non-standard port connection blocked as expected"
    fi
}

# Test localhost bypass
test_localhost_access() {
    print_test "Localhost access (should work per NO_PROXY setting)"

    # This should work as localhost is in NO_PROXY
    # We're just verifying the NO_PROXY setting works
    if curl -s -o /dev/null --connect-timeout 2 http://localhost:12345 2>/dev/null; then
        print_info "Localhost connection attempted (port may not be listening)"
    else
        print_info "Localhost connection refused (expected if nothing is listening)"
    fi
    # This is informational, not a pass/fail test
}

# Test raw socket creation (should fail due to NET_RAW being dropped)
test_raw_socket() {
    print_test "Raw socket creation"

    if ! command_exists ping; then
        print_info "Raw socket test - ping not available"
        return
    fi

    local ping_output
    ping_output=$(timeout 3 ping -c 1 8.8.8.8 2>&1 || true)

    if echo "$ping_output" | grep -qi "operation not permitted\|not permitted\|permission denied"; then
        print_pass "Raw socket creation blocked (NET_RAW capability dropped)"
    elif echo "$ping_output" | grep -q "1 received"; then
        print_fail "Raw socket creation succeeded (NET_RAW should be dropped)"
    else
        print_pass "Raw socket operation blocked (timeout or unreachable)"
    fi
}

# Test container-to-external direct IP connection
# Args: $1 = ip, $2 = port, $3 = description
test_direct_ip_connection() {
    local ip="$1"
    local port="$2"
    local desc="$3"

    print_test "$desc"

    # Try direct connection to IP (bypassing DNS and potentially proxy)
    if timeout "$TIMEOUT" env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy \
        bash -c "exec 3<>/dev/tcp/$ip/$port" 2>/dev/null; then
        exec 3>&- 2>/dev/null || true
        print_fail "$desc - direct IP connection succeeded (should be blocked)"
    else
        print_pass "$desc - direct IP connection blocked as expected"
    fi
}

# Test IPv6 connectivity
# Args: $1 = ipv6_host, $2 = port, $3 = description
test_ipv6_connection() {
    local host="$1"
    local port="$2"
    local desc="$3"

    print_test "$desc"

    # Check if IPv6 is available in the container
    if ! ip -6 addr show 2>/dev/null | grep -q "inet6"; then
        print_pass "$desc - IPv6 not available in container (secure)"
        return
    fi

    # Try to connect via IPv6
    if timeout "$TIMEOUT" curl -6 -s -o /dev/null --connect-timeout 3 "http://[$host]:$port" 2>/dev/null; then
        print_fail "$desc - IPv6 connection succeeded (should be blocked)"
    else
        print_pass "$desc - IPv6 connection blocked as expected"
    fi
}

# Test cloud metadata endpoint (AWS/GCP/Azure)
# Args: $1 = url, $2 = description
test_cloud_metadata() {
    local url="$1"
    local desc="$2"

    print_test "$desc"

    # Try to access cloud metadata endpoint (bypassing proxy)
    local http_code
    http_code=$(timeout "$TIMEOUT" env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy \
        curl -s -o /dev/null -w "%{http_code}" --connect-timeout 3 "$url" 2>/dev/null) || http_code="000"

    if [ "$http_code" = "000" ] || [ "$http_code" = "403" ]; then
        print_pass "$desc - metadata endpoint blocked as expected (HTTP $http_code)"
    else
        print_fail "$desc - metadata endpoint accessible (HTTP $http_code) - CRITICAL SECURITY ISSUE"
    fi
}

# Test local network access
# Args: $1 = ip_range_example, $2 = port, $3 = description
test_local_network() {
    local ip="$1"
    local port="$2"
    local desc="$3"

    print_test "$desc"

    # Try direct connection to local network IP
    if timeout "$TIMEOUT" env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy \
        bash -c "exec 3<>/dev/tcp/$ip/$port" 2>/dev/null; then
        exec 3>&- 2>/dev/null || true
        print_fail "$desc - local network connection succeeded (should be blocked)"
    else
        print_pass "$desc - local network connection blocked as expected"
    fi
}

# Test direct access to proxy container (container-to-container)
# Args: $1 = container_host, $2 = port, $3 = description, $4 = expected ("pass" or "fail")
test_container_access() {
    local host="$1"
    local port="$2"
    local desc="$3"
    local expected="$4"

    print_test "$desc"

    # Try direct connection to container
    local result
    if timeout "$TIMEOUT" bash -c "exec 3<>/dev/tcp/$host/$port" 2>/dev/null; then
        exec 3>&- 2>/dev/null || true
        result="pass"
    else
        result="fail"
    fi

    if [ "$result" = "$expected" ]; then
        if [ "$expected" = "pass" ]; then
            print_pass "$desc - connection succeeded as expected (needed for proxy)"
        else
            print_pass "$desc - connection blocked as expected"
        fi
    else
        if [ "$expected" = "pass" ]; then
            print_fail "$desc - connection failed (proxy should be reachable)"
        else
            print_fail "$desc - connection succeeded (should be blocked)"
        fi
    fi
}

# Test that unconfigured host service ports on proxy are not accessible
# Args: $1 = port, $2 = description
test_unconfigured_host_port() {
    local port="$1"
    local desc="$2"

    print_test "$desc"

    # Try to connect to an unconfigured port on the proxy
    # This should fail because no socat forwarder is running for this port
    if timeout "$TIMEOUT" bash -c "exec 3<>/dev/tcp/tsk-proxy/$port" 2>/dev/null; then
        exec 3>&- 2>/dev/null || true
        print_fail "$desc - connection succeeded to unconfigured port (should be blocked)"
    else
        print_pass "$desc - connection refused to unconfigured port as expected"
    fi
}

# Test WebSocket connection through proxy
# Args: $1 = url, $2 = description
test_websocket() {
    local url="$1"
    local desc="$2"

    print_test "$desc"

    # WebSocket upgrade request - if blocked, we should get connection refused or 403
    local http_code
    http_code=$(timeout "$TIMEOUT" curl -s -o /dev/null -w "%{http_code}" \
        -H "Upgrade: websocket" \
        -H "Connection: Upgrade" \
        -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
        -H "Sec-WebSocket-Version: 13" \
        "$url" 2>/dev/null) || http_code="000"

    # WebSocket on blocked domain should fail
    if [ "$http_code" = "000" ] || [ "$http_code" = "403" ]; then
        print_pass "$desc - WebSocket blocked as expected (HTTP $http_code)"
    else
        print_fail "$desc - WebSocket connection not blocked (HTTP $http_code)"
    fi
}

# Main test execution
main() {
    echo -e "${BLUE}"
    echo "╔═══════════════════════════════════════════════════════════════╗"
    echo "║          TSK Container Security Test Suite                     ║"
    echo "╚═══════════════════════════════════════════════════════════════╝"
    echo -e "${NC}"

    # Check if we're running in the right environment
    if [ -z "${HTTP_PROXY:-}" ]; then
        echo -e "${RED}WARNING: HTTP_PROXY is not set. This test should run inside a TSK container.${NC}"
        echo "Proxy environment variables:"
        env | grep -i proxy || echo "  (none found)"
        echo ""
    fi

    # =========================================================================
    # SECTION 1: HTTP/HTTPS Through Proxy - Allowed Domains
    # =========================================================================
    print_header "HTTP/HTTPS Through Proxy - Allowed Domains (should succeed)" > /dev/null

    run_test test_http_proxy "https://api.anthropic.com" "pass" "Anthropic API"
    run_test test_http_proxy "https://api.openai.com" "pass" "OpenAI API"
    run_test test_http_proxy "https://pypi.org" "pass" "PyPI"
    run_test test_http_proxy "https://index.crates.io" "pass" "crates.io index"
    run_test test_http_proxy "https://registry.npmjs.org" "pass" "npm registry"
    run_test test_http_proxy "https://proxy.golang.org" "pass" "Go proxy"
    run_test test_http_proxy "https://repo.maven.apache.org/maven2/" "pass" "Maven Central"
    run_test test_http_proxy "https://github.com" "pass" "GitHub"

    # =========================================================================
    # SECTION 2: HTTP/HTTPS Through Proxy - Blocked Domains
    # =========================================================================
    print_header "HTTP/HTTPS Through Proxy - Blocked Domains (should fail)" > /dev/null

    run_test test_http_proxy "https://www.google.com" "fail" "Google (blocked)"
    run_test test_http_proxy "https://www.example.com" "fail" "example.com (blocked)"
    run_test test_http_proxy "https://httpbin.org/get" "fail" "httpbin.org (blocked)"
    run_test test_http_proxy "https://icanhazip.com" "fail" "icanhazip.com (blocked)"
    run_test test_http_proxy "https://ifconfig.me" "fail" "ifconfig.me (blocked)"

    # =========================================================================
    # SECTION 3: Direct HTTP/HTTPS (Bypassing Proxy)
    # =========================================================================
    print_header "Direct HTTP/HTTPS Connections (bypassing proxy - should fail)" > /dev/null

    run_test test_direct_http "http://www.google.com" "Direct HTTP to Google"
    run_test test_direct_http "https://www.google.com" "Direct HTTPS to Google"
    run_test test_direct_http "http://api.anthropic.com" "Direct HTTP to Anthropic"
    run_test test_direct_http "https://api.anthropic.com" "Direct HTTPS to Anthropic"

    # =========================================================================
    # SECTION 4: Direct TCP Connections
    # =========================================================================
    print_header "Direct TCP Connections (should fail)" > /dev/null

    run_test test_direct_connection "google.com" "80" "Direct TCP to google.com:80"
    run_test test_direct_connection "google.com" "443" "Direct TCP to google.com:443"
    run_test test_direct_connection "8.8.8.8" "53" "Direct TCP to Google DNS (8.8.8.8:53)"

    # =========================================================================
    # SECTION 5: Direct IP Connections (bypassing DNS)
    # =========================================================================
    print_header "Direct IP Connections (should fail)" > /dev/null

    # Google's IP (one of many)
    run_test test_direct_ip_connection "142.250.80.46" "80" "Direct to Google IP:80"
    run_test test_direct_ip_connection "142.250.80.46" "443" "Direct to Google IP:443"

    # =========================================================================
    # SECTION 6: UDP/DNS Tests
    # =========================================================================
    print_header "UDP/DNS Tests" > /dev/null

    # Test if we can reach external DNS servers (UDP traffic)
    run_test test_external_dns "8.8.8.8" "External DNS query to Google (8.8.8.8)"
    run_test test_external_dns "1.1.1.1" "External DNS query to Cloudflare (1.1.1.1)"

    # Direct DNS resolution test (should fail - proxy handles DNS for HTTP requests)
    run_test test_dns_resolution "google.com" "Direct DNS resolution for google.com"

    # =========================================================================
    # SECTION 7: SSH Connections
    # =========================================================================
    print_header "SSH Connection Tests (should fail)" > /dev/null

    run_test test_ssh_connection "github.com" "SSH to github.com"
    run_test test_ssh_connection "gitlab.com" "SSH to gitlab.com"

    # =========================================================================
    # SECTION 8: ICMP/Ping Tests
    # =========================================================================
    print_header "ICMP/Ping Tests (should fail - NET_RAW dropped)" > /dev/null

    run_test test_ping "8.8.8.8" "Ping to 8.8.8.8"
    run_test test_ping "google.com" "Ping to google.com"

    # =========================================================================
    # SECTION 9: Raw Socket Tests
    # =========================================================================
    print_header "Raw Socket Tests (should fail - NET_RAW dropped)" > /dev/null

    run_test test_raw_socket

    # =========================================================================
    # SECTION 10: Non-Standard Port Tests
    # =========================================================================
    print_header "Non-Standard Port Tests (should fail)" > /dev/null

    run_test test_nonstandard_port "http://portquiz.net:8080" "HTTP on port 8080"
    run_test test_nonstandard_port "https://example.com:8443" "HTTPS on port 8443"

    # =========================================================================
    # SECTION 11: Localhost/NO_PROXY Tests (informational)
    # =========================================================================
    print_header "Localhost Tests (informational)" > /dev/null

    run_test test_localhost_access

    # =========================================================================
    # SECTION 12: IPv6 Tests (should fail)
    # =========================================================================
    print_header "IPv6 Connectivity Tests (should fail)" > /dev/null

    # Google's IPv6 address
    run_test test_ipv6_connection "2607:f8b0:4004:800::200e" "80" "IPv6 to Google"
    # Cloudflare's IPv6
    run_test test_ipv6_connection "2606:4700:4700::1111" "443" "IPv6 to Cloudflare"

    # =========================================================================
    # SECTION 13: Cloud Metadata Tests (should fail - critical)
    # =========================================================================
    print_header "Cloud Metadata Endpoint Tests (should fail - CRITICAL)" > /dev/null

    # AWS metadata endpoint
    run_test test_cloud_metadata "http://169.254.169.254/latest/meta-data/" "AWS metadata endpoint"
    # GCP metadata endpoint
    run_test test_cloud_metadata "http://metadata.google.internal/computeMetadata/v1/" "GCP metadata endpoint"
    # Azure metadata endpoint
    run_test test_cloud_metadata "http://169.254.169.254/metadata/instance" "Azure metadata endpoint"
    # Link-local range (general)
    run_test test_direct_ip_connection "169.254.169.254" "80" "Link-local metadata IP"

    # =========================================================================
    # SECTION 14: Local Network Tests (should fail)
    # =========================================================================
    print_header "Local Network Access Tests (should fail)" > /dev/null

    # Common local network ranges - test a sample IP from each
    run_test test_local_network "192.168.1.1" "80" "Local network 192.168.x.x"
    run_test test_local_network "10.0.0.1" "80" "Local network 10.x.x.x"
    run_test test_local_network "172.16.0.1" "80" "Local network 172.16.x.x"

    # =========================================================================
    # SECTION 15: Container-to-Container Tests
    # =========================================================================
    print_header "Container-to-Container Communication Tests" > /dev/null

    # Proxy should be reachable on port 3128 (needed for HTTP proxy)
    run_test test_container_access "tsk-proxy" "3128" "Proxy container on port 3128" "pass"
    # But other ports on proxy should not be accessible
    run_test test_container_access "tsk-proxy" "22" "Proxy container on port 22 (SSH)" "fail"
    run_test test_container_access "tsk-proxy" "80" "Proxy container on port 80 (HTTP)" "fail"

    # Test unconfigured host service ports (should fail)
    # Port 9999 should not have a socat forwarder unless explicitly configured
    run_test test_unconfigured_host_port "9999" "Unconfigured host service port 9999"

    # =========================================================================
    # SECTION 16: WebSocket Tests (should fail on blocked domains)
    # =========================================================================
    print_header "WebSocket Tests (should fail on blocked domains)" > /dev/null

    run_test test_websocket "https://echo.websocket.org" "WebSocket to echo.websocket.org"
    run_test test_websocket "https://socketsbay.com/wss/v2/1/demo/" "WebSocket to socketsbay.com"

    # =========================================================================
    # FINAL RESULTS
    # =========================================================================
    print_header "TEST RESULTS SUMMARY"

    echo ""
    echo -e "Total tests run: ${TESTS_TOTAL}"
    echo -e "${GREEN}Tests passed: ${TESTS_PASSED}${NC}"
    echo -e "${RED}Tests failed: ${TESTS_FAILED}${NC}"
    echo ""

    # Calculate and display score
    if [ "$TESTS_TOTAL" -gt 0 ]; then
        local score=$((TESTS_PASSED * 100 / TESTS_TOTAL))
        echo -e "Security Score: ${score}%"
        echo ""
    fi

    # List failed tests
    if [ ${#FAILED_TESTS[@]} -gt 0 ]; then
        echo -e "${RED}Failed tests:${NC}"
        for test in "${FAILED_TESTS[@]}"; do
            echo -e "  ${RED}x${NC} $test"
        done
        echo ""
    fi

    # Final verdict
    if [ "$TESTS_FAILED" -eq 0 ]; then
        echo -e "${GREEN}+---------------------------------------------------------------+${NC}"
        echo -e "${GREEN}|  ALL SECURITY TESTS PASSED - Network isolation is working    |${NC}"
        echo -e "${GREEN}+---------------------------------------------------------------+${NC}"
        exit 0
    else
        echo -e "${RED}+---------------------------------------------------------------+${NC}"
        echo -e "${RED}|  SECURITY TESTS FAILED - Potential security issues found      |${NC}"
        echo -e "${RED}+---------------------------------------------------------------+${NC}"
        exit 1
    fi
}

# Run main function
main "$@"

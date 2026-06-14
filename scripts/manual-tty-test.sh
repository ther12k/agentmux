#!/usr/bin/env bash
set -euo pipefail

echo "=== AgentMux Manual TTY Test ==="
echo ""

# Step 1: Doctor
echo "Step 1: Running doctor..."
agentmux doctor
echo ""

# Step 2: Start a test session
echo "Step 2: Starting test session..."
agentmux run shell --name tty-test --cwd .
echo ""

# Step 3: List
echo "Step 3: Listing sessions..."
agentmux list
echo ""

# Step 4: Attach (interactive)
echo "Step 4: Attaching to tty-test..."
echo ""
echo "  >>> INSIDE THE ATTACHED SHELL <<<"
echo "  1. Run: echo hello"
echo "  2. Press Ctrl-b then d to detach"
echo "  3. Re-run: agentmux attach tty-test"
echo "  4. Run: exit"
echo "  5. Confirm attach returns immediately"
echo ""
echo "Press Enter to attach..."
read -r
agentmux attach tty-test

echo ""
echo "=== Re-attach test ==="
echo "Press Enter to re-attach..."
read -r
agentmux attach tty-test

echo ""
echo "=== Final state ==="
agentmux list
agentmux logs tty-test --tail 20
echo ""
echo "=== Manual TTY test complete ==="

#!/usr/bin/env bash
set -euo pipefail

# AgentMux install script
# Builds a release binary and copies it to ~/.local/bin/agentmux

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "Building agentmux (release)..."
cd "$PROJECT_ROOT"
cargo build --release

BINARY="$PROJECT_ROOT/target/release/agentmux"
if [[ ! -f "$BINARY" ]]; then
    echo "Error: built binary not found at $BINARY" >&2
    exit 1
fi

INSTALL_DIR="$HOME/.local/bin"
DATA_DIR="$HOME/.local/share/agentmux/logs"

echo "Installing to $INSTALL_DIR/agentmux..."
mkdir -p "$INSTALL_DIR"
mkdir -p "$DATA_DIR"
cp "$BINARY" "$INSTALL_DIR/agentmux"
chmod +x "$INSTALL_DIR/agentmux"

echo "Install complete."

case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        ;;
    *)
        echo ""
        echo "NOTE: $INSTALL_DIR is not in your PATH."
        echo "Add this line to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
        echo ""
        echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
        echo ""
        ;;
esac

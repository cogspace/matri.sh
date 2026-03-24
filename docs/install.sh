#!/bin/sh
set -e

BASE_URL="https://github.com/cogspace/matri.sh/releases/latest/download"

if [ -n "$INSTALL_DIR" ]; then
  : # use provided value
elif [ -w /usr/local/bin ]; then
  INSTALL_DIR="/usr/local/bin"
else
  INSTALL_DIR="$HOME/.local/bin"
  mkdir -p "$INSTALL_DIR"
fi

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)
    case "$ARCH" in
      x86_64)  BINARY="matri.sh-linux-x86_64" ;;
      aarch64) BINARY="matri.sh-linux-aarch64" ;;
      *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  Darwin)
    case "$ARCH" in
      arm64)  BINARY="matri.sh-macos-aarch64" ;;
      x86_64) echo "No prebuilt binary for macOS x86_64. Please build from source." >&2; exit 1 ;;
      *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Unsupported OS: $OS" >&2
    exit 1
    ;;
esac

URL="$BASE_URL/$BINARY"
DEST="$INSTALL_DIR/matri.sh"

echo "Downloading $BINARY..."
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$URL" -o "$DEST"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$DEST" "$URL"
else
  echo "curl or wget is required" >&2
  exit 1
fi

chmod +x "$DEST"
echo "Installed to $DEST"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo "Note: $INSTALL_DIR is not in your PATH."
    case "${SHELL##*/}" in
      zsh)  RC="$HOME/.zshrc" ;;
      fish) RC="$HOME/.config/fish/config.fish" ;;
      *)    RC="$HOME/.bashrc" ;;
    esac
    echo "Add it by running:"
    echo ""
    if [ "${SHELL##*/}" = "fish" ]; then
      echo "    echo 'fish_add_path $INSTALL_DIR' >> $RC"
    else
      echo "    echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> $RC"
    fi
    ;;
esac

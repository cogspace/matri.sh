#!/bin/sh
# matri.sh installer
# Usage: curl -fsSL https://matri.sh/install.sh | sh

set -e

RAW_URL="https://raw.githubusercontent.com/cogspace/matri.sh/main/matri.sh"
BIN_DIR="${HOME}/.local/bin"
DEST="${BIN_DIR}/matri.sh"

# ── Helpers ────────────────────────────────────────────────────────────────────

green()  { printf '\033[32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }
red()    { printf '\033[31m%s\033[0m\n' "$*"; }
die()    { red "error: $*" >&2; exit 1; }

# ── Python check ───────────────────────────────────────────────────────────────

PYTHON=""
for py in python3 python; do
  if command -v "$py" >/dev/null 2>&1; then
    if "$py" -c "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)" 2>/dev/null; then
      PYTHON="$py"
      break
    fi
  fi
done

[ -n "$PYTHON" ] || die "Python 3.10+ is required. Install it from https://python.org and try again."

# ── Download ───────────────────────────────────────────────────────────────────

mkdir -p "$BIN_DIR"

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$RAW_URL" -o "$DEST"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$DEST" "$RAW_URL"
else
  die "curl or wget is required to download matri.sh."
fi

chmod +x "$DEST"

# ── Python dependencies ────────────────────────────────────────────────────────

green "Installing Python dependencies..."
"$PYTHON" -m pip install --quiet pyte

# wcwidth is optional — don't fail if it can't install
"$PYTHON" -m pip install --quiet wcwidth 2>/dev/null || \
  yellow "  (wcwidth not installed — wide characters will default to width 1)"

# ── PATH hint ─────────────────────────────────────────────────────────────────

RC_HINT=""
case "${SHELL}" in
  */zsh)  RC_HINT="~/.zshrc" ;;
  */bash) RC_HINT="~/.bashrc" ;;
  */fish) RC_HINT="~/.config/fish/config.fish" ;;
esac

echo ""
green "matri.sh installed to ${DEST}"
echo ""

# Check if BIN_DIR is already on PATH
case ":${PATH}:" in
  *":${BIN_DIR}:"*)
    green "Run it with:  matri.sh"
    ;;
  *)
    yellow "Add ~/.local/bin to your PATH to run it from anywhere:"
    echo ""
    if [ "$SHELL" = "*/fish" ]; then
      echo "    fish_add_path ~/.local/bin"
    else
      echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
      [ -n "$RC_HINT" ] && echo "    (add the above line to ${RC_HINT})"
    fi
    echo ""
    yellow "Or run it directly:"
    echo "    ${DEST}"
    ;;
esac
echo ""

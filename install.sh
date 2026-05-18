#!/usr/bin/env sh
# homn installer — downloads a verified prebuilt binary from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/rohansx/homn/master/install.sh | sh
#
# Flags (pass after `-s --` when piping, e.g. `... | sh -s -- --version v0.1.0`):
#   --version vX.Y.Z   install a specific release (default: latest)
#   --bin-dir DIR      install location (default: ~/.local/bin)
set -eu

REPO="rohansx/homn"
VERSION=""
BIN_DIR="${HOMN_BIN_DIR:-$HOME/.local/bin}"

while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="$2"; shift 2 ;;
        --bin-dir) BIN_DIR="$2"; shift 2 ;;
        *) echo "homn install: unknown flag '$1'" >&2; exit 1 ;;
    esac
done

need() { command -v "$1" >/dev/null 2>&1 || { echo "homn install: missing '$1'" >&2; exit 1; }; }
need curl
need tar

# --- detect platform -> Rust target triple ---------------------------------
detect_triple() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Linux)  os_part="unknown-linux-gnu" ;;
        Darwin) os_part="apple-darwin" ;;
        *) echo "homn install: unsupported OS '$os' — build from source: cargo install --git https://github.com/$REPO homn-bin" >&2; exit 1 ;;
    esac
    case "$arch" in
        x86_64|amd64)  arch_part="x86_64" ;;
        aarch64|arm64) arch_part="aarch64" ;;
        *) echo "homn install: unsupported arch '$arch' — build from source: cargo install --git https://github.com/$REPO homn-bin" >&2; exit 1 ;;
    esac
    echo "${arch_part}-${os_part}"
}
TRIPLE=$(detect_triple)

# --- resolve the release tag ------------------------------------------------
if [ -z "$VERSION" ]; then
    VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | grep '"tag_name"' | head -1 | cut -d'"' -f4)
    [ -n "$VERSION" ] || { echo "homn install: could not resolve the latest release" >&2; exit 1; }
fi

ASSET="homn-${VERSION}-${TRIPLE}.tar.gz"
BASE="https://github.com/$REPO/releases/download/$VERSION"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "homn install: downloading $ASSET ($VERSION)"
curl -fsSL "$BASE/$ASSET"        -o "$TMP/$ASSET"
curl -fsSL "$BASE/$ASSET.sha256" -o "$TMP/$ASSET.sha256"

# --- verify checksum --------------------------------------------------------
echo "homn install: verifying checksum"
( cd "$TMP" && \
  if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "$ASSET.sha256"; \
  else shasum -a 256 -c "$ASSET.sha256"; fi ) \
  || { echo "homn install: CHECKSUM MISMATCH — aborting" >&2; exit 1; }

# --- install ----------------------------------------------------------------
# The release tarball wraps everything in a top-level homn-<tag>-<triple>/ dir;
# --strip-components=1 drops it so the binary lands directly at $TMP/homn.
tar -xzf "$TMP/$ASSET" -C "$TMP" --strip-components=1
mkdir -p "$BIN_DIR"
mv "$TMP/homn" "$BIN_DIR/homn"
chmod +x "$BIN_DIR/homn"
echo "homn install: installed to $BIN_DIR/homn"

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo ""
       echo "  NOTE: $BIN_DIR is not on your PATH. Add this to your shell rc file:"
       echo "      export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

echo ""
echo "homn installed. Next: run  homn setup"

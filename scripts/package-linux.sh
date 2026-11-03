#!/bin/sh
# Build a Linux release archive: target/release/MiniDiff-linux-<arch>.tar.gz (+ .sha256)
#
#   scripts/package-linux.sh            # build and package
#   scripts/package-linux.sh --install  # also install into ~/.local (bin, icon, .desktop)
#
# The archive holds MiniDiff/{minidiff, minidiff.desktop, minidiff.png, LICENSE, README.md}.
set -e
cd "$(dirname "$0")/.."

ARCH=$(uname -m)
NAME="MiniDiff-linux-$ARCH"
OUT=target/release
STAGE="$OUT/linux/MiniDiff"

cargo build --release
rm -rf "$OUT/linux"
mkdir -p "$STAGE"
cp "$OUT/minidiff" "$STAGE/minidiff"
strip "$STAGE/minidiff" 2>/dev/null || true
cp linux/minidiff.desktop "$STAGE/"
cp assets/icon-256.png "$STAGE/minidiff.png"
cp LICENSE README.md "$STAGE/"

tar -C "$OUT/linux" -czf "$OUT/$NAME.tar.gz" MiniDiff
(cd "$OUT" && sha256sum "$NAME.tar.gz" | cut -d' ' -f1 > "$NAME.tar.gz.sha256")
echo "$OUT/$NAME.tar.gz"

if [ "${1:-}" = "--install" ]; then
  install -Dm755 "$STAGE/minidiff" "$HOME/.local/bin/minidiff"
  install -Dm644 "$STAGE/minidiff.png" "$HOME/.local/share/icons/hicolor/256x256/apps/minidiff.png"
  install -Dm644 "$STAGE/minidiff.desktop" "$HOME/.local/share/applications/minidiff.desktop"
  echo "Installed to ~/.local (make sure ~/.local/bin is on your PATH)"
fi

#!/bin/sh
# Register MiniDiff as a diff & merge tool in Tower (macOS).
set -e
cd "$(dirname "$0")/.."
DEST="$HOME/Library/Application Support/com.fournova.Tower3/CompareTools"
mkdir -p "$DEST"
cp integrations/tower/minidiff.sh "$DEST/minidiff.sh"
chmod +x "$DEST/minidiff.sh"

if [ -f "$DEST/CompareTools.plist" ] && ! grep -q "<string>minidiff</string>" "$DEST/CompareTools.plist"; then
  echo "⚠  $DEST/CompareTools.plist already exists with other tools."
  echo "   Add the <dict> from integrations/tower/CompareTools.plist to its <array> by hand."
else
  cp integrations/tower/CompareTools.plist "$DEST/CompareTools.plist"
fi
echo "Installed. Restart Tower, then pick MiniDiff in Settings → Git Config → Diff Tool / Merge Tool."
[ -x /Applications/MiniDiff.app/Contents/MacOS/minidiff ] || \
  echo "Note: /Applications/MiniDiff.app is missing — run scripts/bundle-macos.sh --install first."

#!/bin/sh
# Build MiniDiff.app (release) into target/release/bundle/MiniDiff.app
#
#   scripts/bundle-macos.sh            # native architecture
#   scripts/bundle-macos.sh universal  # arm64 + x86_64 (needs both rustup targets)
#   scripts/bundle-macos.sh --install  # also copy to /Applications and link the CLI
set -e
cd "$(dirname "$0")/.."

UNIVERSAL=0
INSTALL=0
for arg in "$@"; do
  case "$arg" in
    universal) UNIVERSAL=1 ;;
    --install) INSTALL=1 ;;
  esac
done

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/release/bundle/MiniDiff.app

if [ "$UNIVERSAL" = 1 ]; then
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/release
  BIN=target/release/minidiff-universal
  lipo -create -output "$BIN" \
    target/aarch64-apple-darwin/release/minidiff \
    target/x86_64-apple-darwin/release/minidiff
else
  cargo build --release
  BIN=target/release/minidiff
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/minidiff"
sed "s/__VERSION__/$VERSION/g" macos/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# Icon: render the iconset from assets/icon-1024.png.
ICONSET=target/release/bundle/MiniDiff.iconset
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d assets/icon-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/MiniDiff.icns"
rm -rf "$ICONSET"

# Ad-hoc signature so Gatekeeper / Apple Events treat the bundle consistently.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
echo "Built $APP ($VERSION)"

if [ "$INSTALL" = 1 ]; then
  rm -rf /Applications/MiniDiff.app
  cp -R "$APP" /Applications/
  # Re-register so Finder picks up the document types.
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/MiniDiff.app || true
  mkdir -p "$HOME/.local/bin"
  ln -sf /Applications/MiniDiff.app/Contents/MacOS/minidiff "$HOME/.local/bin/minidiff"
  echo "Installed /Applications/MiniDiff.app and ~/.local/bin/minidiff"
fi

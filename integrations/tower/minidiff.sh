#!/bin/sh
# Tower launch script for MiniDiff.
#   diff:  minidiff.sh LOCAL REMOTE
#   merge: minidiff.sh LOCAL REMOTE BASE MERGED
# MiniDiff runs in the foreground, so Tower waits until the window is closed.
# As merge tool it exits 0 only if the result was saved.

for CMD in \
  "/Applications/MiniDiff.app/Contents/MacOS/minidiff" \
  "$HOME/Applications/MiniDiff.app/Contents/MacOS/minidiff" \
  "$(command -v minidiff)"; do
  [ -n "$CMD" ] && [ -x "$CMD" ] && break
done

if [ ! -x "$CMD" ]; then
  echo "MiniDiff not found. Build it with scripts/bundle-macos.sh --install" >&2
  exit 1
fi

if [ -n "$4" ]; then
  exec "$CMD" --merge "$1" "$2" "$3" "$4"
else
  exec "$CMD" "$1" "$2"
fi

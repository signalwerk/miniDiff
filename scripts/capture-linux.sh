#!/usr/bin/env bash
# Headless Linux screenshot of MiniDiff (Xvfb + xdotool + ImageMagick).
#
#   scripts/capture-linux.sh OUT.png WIDTH HEIGHT [KEY ...] -- MINIDIFF_ARGS...
#
# Keys are xdotool key names sent after the first frame (e.g. a 3 shift+n).
# Environment: THEME=Dark|Light, SHOW_BASE=true|false, BIN=path/to/minidiff,
# WAIT=seconds before input. Uses an isolated settings profile.
set -euo pipefail
out=$1 width=$2 height=$3
shift 3
keys=()
while [ $# -gt 0 ] && [ "$1" != "--" ]; do keys+=("$1"); shift; done
[ "${1:-}" = "--" ] && shift
bin=${BIN:-target/debug/minidiff}
profile=$(mktemp -d)
trap 'rm -rf "$profile"' EXIT
printf '{"window":"(inner_size_points:Some((x:%s.0,y:%s.0)))","app":"(theme:%s,auto_update:false,view:(font_size:13.0,show_base:%s))"}' \
  "$width" "$height" "${THEME:-Dark}" "${SHOW_BASE:-false}" > "$profile/app.ron"
export MINIDIFF_STORAGE_PATH="$profile/app.ron" OUT="$out" BIN="$bin" WAIT="${WAIT:-6}" KEYS="${keys[*]:-}"
xvfb-run -a -s "-screen 0 ${width}x${height}x24" bash -c '
  "$BIN" "$@" & pid=$!
  sleep "$WAIT"
  # No window manager: focus the window explicitly so it receives key events.
  xdotool search --sync --onlyvisible --class minidiff windowfocus --sync >/dev/null 2>&1 || true
  xdotool mousemove $(( '"$width"' - 400 )) 16 click 1; sleep 0.5
  for k in $KEYS; do xdotool key --delay 80 "$k"; sleep 0.7; done
  sleep 1.5
  import -window root "$OUT"
  kill "$pid"' _ "$@"

#!/bin/bash
# Opens the release binary as a stock Ubuntu desktop would: X11 (Xvfb,
# started by the caller with DISPLAY set), a session bus (run this under
# dbus-run-session) and the GTK desktop portal, activated on it. Fails if
# the window crashes, never draws, or doesn't answer on its socket.
# Usage: launch-test.sh <gutenberg binary> <directory for the log and screenshot>
set -euo pipefail
bin=$1
out=$2
mkdir -p "$out"
log="$out/window.log"

missing=$(ldd "$bin" | grep "not found" || true)
if [ -n "$missing" ]; then
  echo "::error::the binary needs libraries Ubuntu doesn't have: $missing"
  exit 1
fi

home=$(mktemp -d)
runtime=$(mktemp -d)
chmod 700 "$runtime"
export HOME=$home XDG_RUNTIME_DIR=$runtime
unset WAYLAND_DISPLAY

"$bin" >"$log" 2>&1 &
app=$!
# Software rendering is slow to start; the first frame comes well before this.
sleep 15
if ! kill -0 "$app" 2>/dev/null; then
  status=0
  wait "$app" || status=$?
  cat "$log"
  echo "::error::the window exited by itself, with status $status"
  exit 1
fi

xwd -root -silent | convert xwd:- "$out/window.png"
colors=$(convert "$out/window.png" -format %k info:)
echo "the screen has $colors colors"
if [ "$colors" -lt 16 ]; then
  cat "$log"
  echo "::error::the window is blank: $colors colors on screen"
  exit 1
fi

# A second launch hands over to the open window and exits; a script
# reaches it over the socket.
timeout 10 "$bin"
timeout 10 "$bin" --reload-theme
kill -0 "$app"

kill "$app"
wait "$app" || true
cat "$log"
if grep -q "panicked" "$log" || [ -e "$home/.local/share/gutenberg/crashes" ]; then
  cat "$home"/.local/share/gutenberg/crashes/* 2>/dev/null || true
  echo "::error::the window panicked"
  exit 1
fi

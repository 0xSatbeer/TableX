#!/usr/bin/env bash
# Screenshot the running app on GNOME Wayland through the screenshot portal (needs the one-time
# permission). The portal captures the whole screen, so the TableX window must be visible:
# GNOME opens it behind the focused window when launched from a terminal.
# Usage: scripts/shot.sh OUT.png [SECONDS_BEFORE_SHOT] [ENV=VALUE ...]
set -euo pipefail
# one capture at a time: parallel runs would photograph each other's windows
exec 9>/tmp/tablex-shot.lock; flock 9
# a cold debug build takes well over 4s to show its window
out=$1; wait=${2:-15}; shift $(( $# > 1 ? 2 : 1 ))
bin=$(mktemp /tmp/tablex-shot.XXXXXX)
install -m 755 target/debug/tablex "$bin"   # a copy, so a concurrent cargo build can relink freely (mktemp makes it 600)
env "$@" "$bin" >/dev/null &   # keep stderr: a launch failure must be visible
app=$!
trap 'kill $app 2>/dev/null || true; rm -f "$bin"' EXIT
sleep "$wait"
before=$(ls -t ~/Pictures/*.png 2>/dev/null | head -1 || true)
gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.portal.Screenshot.Screenshot "" "{'interactive': <false>}" >/dev/null
for _ in $(seq 40); do
  sleep 0.25
  latest=$(ls -t ~/Pictures/*.png 2>/dev/null | head -1 || true)
  # the portal creates the file before it finishes writing it: wait for a non-empty, stable size
  if [[ -n $latest && $latest != "$before" && -s $latest ]]; then
    size=$(stat -c %s "$latest"); sleep 0.5
    if [[ $(stat -c %s "$latest") == "$size" ]]; then mv "$latest" "$out"; echo "$out"; exit 0; fi
  fi
done
echo "no screenshot arrived" >&2; exit 1

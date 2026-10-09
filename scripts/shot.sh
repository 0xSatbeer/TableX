#!/usr/bin/env bash
# Screenshot the running app on GNOME Wayland through the screenshot portal (needs the one-time
# permission). The portal captures the whole screen, so the QueryX window must be visible:
# GNOME opens it behind the focused window when launched from a terminal.
# Usage: scripts/shot.sh OUT.png [SECONDS_BEFORE_SHOT] [ENV=VALUE ...]
set -euo pipefail
out=$1; wait=${2:-4}; shift $(( $# > 1 ? 2 : 1 ))
bin=$(mktemp /tmp/queryx-shot.XXXXXX)
cp target/debug/queryx "$bin"   # a copy, so a concurrent cargo build can relink freely
env "$@" "$bin" >/dev/null 2>&1 &
app=$!
trap 'kill $app 2>/dev/null || true; rm -f "$bin"' EXIT
sleep "$wait"
before=$(ls -t ~/Pictures/*.png 2>/dev/null | head -1 || true)
gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.portal.Screenshot.Screenshot "" "{'interactive': <false>}" >/dev/null
for _ in $(seq 40); do
  sleep 0.25
  latest=$(ls -t ~/Pictures/*.png 2>/dev/null | head -1 || true)
  if [[ -n $latest && $latest != "$before" ]]; then mv "$latest" "$out"; echo "$out"; exit 0; fi
done
echo "no screenshot arrived" >&2; exit 1

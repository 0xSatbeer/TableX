#!/bin/sh
# Installs the latest QueryX Linux release with its desktop entry and icons.
#   curl -fsSL https://github.com/0xSatbeer/QueryX/releases/latest/download/install.sh | sh
# PREFIX defaults to ~/.local; use PREFIX=/usr/local with sudo for a system-wide install.
set -eu

PREFIX=${PREFIX:-$HOME/.local}
ID=io.github._0xSatbeer.QueryX

[ "$(uname -s)" = Linux ] || { echo "install.sh is Linux-only; see the release page for other builds" >&2; exit 1; }
[ "$(uname -m)" = x86_64 ] || { echo "no prebuilt binary for $(uname -m); build from source" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
url=https://github.com/0xSatbeer/QueryX/releases/latest/download
curl -fsSLo "$tmp/queryx-linux-x64.tar.gz" "$url/queryx-linux-x64.tar.gz"
curl -fsSLo "$tmp/SHA256SUMS" "$url/SHA256SUMS"
(cd "$tmp" && grep ' queryx-linux-x64.tar.gz$' SHA256SUMS | sha256sum -c --quiet -) \
  || { echo "checksum mismatch; download is corrupt or tampered" >&2; exit 1; }
tar xzf "$tmp/queryx-linux-x64.tar.gz" -C "$tmp"

install -Dm755 "$tmp/queryx" "$PREFIX/bin/queryx"
install -Dm644 "$tmp/queryx.png" "$PREFIX/share/icons/hicolor/512x512/apps/$ID.png"
install -Dm644 "$tmp/queryx-64.png" "$PREFIX/share/icons/hicolor/64x64/apps/$ID.png"
# absolute Exec: ~/.local/bin is often missing from the desktop session's PATH
sed "s|^Exec=queryx|Exec=$PREFIX/bin/queryx|" "$tmp/$ID.desktop" > "$tmp/out.desktop"
install -Dm644 "$tmp/out.desktop" "$PREFIX/share/applications/$ID.desktop"

update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" 2>/dev/null || true

echo "Installed QueryX to $PREFIX/bin/queryx"

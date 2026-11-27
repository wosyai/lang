#!/bin/sh
set -eu

fail() { printf 'wosy install: %s\n' "$*" >&2; exit 1; }

[ "$#" -le 1 ] || fail 'usage: sh install.sh [vX.Y.Z]'
release=https://github.com/wosyai/lang/releases
case ${1:-latest} in
    latest) url=$release/latest/download ;;
    v[0-9]*)
        printf '%s\n' "$1" | awk 'NR != 1 || $0 !~ /^v[0-9]+\.[0-9]+\.[0-9]+$/ {bad=1} END {exit (NR != 1 || bad)}' ||
            fail 'invalid release tag'
        url=$release/download/$1 ;;
    *) fail 'expected latest or a tag such as v0.1.0' ;;
esac

case "$(uname -s):$(uname -m)" in
    Linux:x86_64) asset=wosy-linux-x64.tar.gz ;;
    Darwin:arm64) asset=wosy-macos-arm64.tar.gz ;;
    *) fail 'supported targets: Linux x64 and macOS arm64' ;;
esac

download=$(mktemp -d)
stage=
cleanup() {
    rm -rf "$download"
    if [ -n "$stage" ]; then rm -rf "$stage"; fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

curl -fsSL --proto '=https' "$url/$asset" -o "$download/$asset"
curl -fsSL --proto '=https' "$url/$asset.sha256" -o "$download/checksum"
expected=$(awk 'NR == 1 {print tolower($1); exit}' "$download/checksum")
case $expected in ''|*[!0123456789abcdef]*) fail 'invalid SHA-256 file' ;; esac
[ "${#expected}" -eq 64 ] || fail 'invalid SHA-256 file'
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$download/$asset")
else
    actual=$(shasum -a 256 "$download/$asset")
fi
[ "${actual%% *}" = "$expected" ] || fail 'SHA-256 mismatch; nothing installed'

[ "$(tar -tzf "$download/$asset")" = wosy ] || fail 'archive must contain only wosy at its root'
dir=${WOSY_INSTALL_DIR:-"$HOME/.local/bin"}
mkdir -p "$dir"
[ ! -d "$dir/wosy" ] || fail "destination is a directory: $dir/wosy"
stage=$(mktemp -d "$dir/.wosy-install.XXXXXX")
tar -xOzf "$download/$asset" wosy >"$stage/wosy"
[ -s "$stage/wosy" ] || fail 'archive has no executable'
chmod 755 "$stage/wosy"
mv -f "$stage/wosy" "$dir/wosy"
printf 'Installed %s/wosy\n' "$dir"
case ":$PATH:" in
    *":$dir:"*) ;;
    *) printf 'Add %s to your PATH if wosy is not found.\n' "$dir" ;;
esac

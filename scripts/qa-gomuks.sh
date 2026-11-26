#!/usr/bin/env bash
# The pinned gomuks the QA scripts use as a tool: it prepares accounts and
# recovery keys and plays the encrypted peer. The client itself no longer
# runs gomuks. Prints the executable's path.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
source scripts/gomuks.env
backend="$PWD/.dev/backends/$GOMUKS_VERSION/gomuks"
if [[ ! -x "$backend" ]]; then
    [[ $(uname -sm) == 'Linux x86_64' ]] || { echo 'Set UNTETHERED_BACKEND to a gomuks executable.' >&2; exit 1; }
    mkdir -p "$(dirname "$backend")"
    tmp=$(mktemp "${backend}.XXXXXX")
    trap 'rm -f -- "$tmp"' EXIT
    curl -fsSL "https://github.com/gomuks/gomuks/releases/download/v$GOMUKS_VERSION/gomuks-amd64" -o "$tmp"
    echo "$GOMUKS_LINUX_SHA256  $tmp" | sha256sum -c - >&2
    chmod 755 "$tmp"
    mv "$tmp" "$backend"
fi
echo "$backend"

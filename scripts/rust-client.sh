#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
source scripts/gomuks.env
export UNTETHERED_PROFILE="${UNTETHERED_PROFILE:-$PWD/.dev/rust-profile}"
if [[ -z "${UNTETHERED_BACKEND:-}" ]]; then
    backend="$PWD/.dev/backends/$GOMUKS_VERSION/gomuks"
    if [[ ! -x "$backend" ]]; then
        [[ $(uname -sm) == 'Linux x86_64' ]] || { echo 'Set UNTETHERED_BACKEND to the gomuks executable.' >&2; exit 1; }
        mkdir -p "$(dirname "$backend")"
        tmp=$(mktemp "${backend}.XXXXXX")
        trap 'rm -f -- "$tmp"' EXIT
        curl -fsSL "https://github.com/gomuks/gomuks/releases/download/v$GOMUKS_VERSION/gomuks-amd64" -o "$tmp"
        echo "$GOMUKS_LINUX_SHA256  $tmp" | sha256sum -c -
        chmod 755 "$tmp"
        mv "$tmp" "$backend"
    fi
    export UNTETHERED_BACKEND="$backend"
fi
if [[ $(uname -s) == Linux ]]; then
    cargo build --locked -j "${UNTETHERED_BUILD_JOBS:-3}" --manifest-path rust-client/Cargo.toml --bin untethered-cameras
fi
export ICED_BACKEND="${ICED_BACKEND:-wgpu,tiny-skia}"
exec cargo run --locked -j "${UNTETHERED_BUILD_JOBS:-3}" --manifest-path rust-client/Cargo.toml -- "$@"

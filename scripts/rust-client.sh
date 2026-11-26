#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
export UNTETHERED_PROFILE="${UNTETHERED_PROFILE:-$PWD/.dev/rust-profile}"
if [[ $(uname -s) == Linux ]]; then
    cargo build --locked -j "${UNTETHERED_BUILD_JOBS:-3}" --manifest-path rust-client/Cargo.toml --bin untethered-cameras
fi
export ICED_BACKEND="${ICED_BACKEND:-wgpu,tiny-skia}"
exec cargo run --locked -j "${UNTETHERED_BUILD_JOBS:-3}" --manifest-path rust-client/Cargo.toml -- "$@"

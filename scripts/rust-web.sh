#!/usr/bin/env bash
# Build the web client: the Iced UI and the matrix-sdk engine's worker in one
# wasm module (rust-client/docs/matrix-sdk-migration.md, step 7).
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../rust-client/web"
CARGO_BUILD_JOBS=3 trunk build --no-sri "$@"

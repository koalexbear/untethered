#!/usr/bin/env bash
# Build the same Iced UI with the upstream gomuks browser worker.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
root="$PWD"
src="$root/.dev/gomuks-src"
stage="$root/.dev/wasm-worker"
source scripts/gomuks.env
mkdir -p "$stage"
if [[ ! -d "$src" ]]; then
    git clone --depth=1 --branch "v$GOMUKS_VERSION" https://github.com/gomuks/gomuks.git "$src"
fi
[[ $(git -C "$src" rev-parse HEAD) == "$GOMUKS_COMMIT" ]] || { echo 'Unexpected gomuks checkout' >&2; exit 1; }
cp "$src/cmd/wasmuks/media.go" "$stage/media.go"
patch --batch --fuzz=0 "$stage/media.go" < rust-client/web/worker/media-limit.patch
cp "$src/cmd/wasmuks/main.go" "$stage/main.go"
cp "$src/pkg/gomuks/mediaupload.go" "$stage/mediaupload.go"
patch --batch --fuzz=0 "$stage/main.go" < rust-client/web/worker/upload-main.patch
patch --batch --fuzz=0 "$stage/mediaupload.go" < rust-client/web/worker/upload-client.patch
node -e 'const [src,stage,root]=process.argv.slice(1); process.stdout.write(JSON.stringify({Replace:{
    [src+"/cmd/wasmuks/media.go"]:stage+"/media.go",
    [src+"/cmd/wasmuks/main.go"]:stage+"/main.go",
    [src+"/cmd/wasmuks/upload_transport.go"]:root+"/rust-client/web/worker/upload_transport.go",
    [src+"/pkg/gomuks/mediaupload.go"]:stage+"/mediaupload.go"
}}))' "$src" "$stage" "$root" > "$stage/overlay.json"
(
    cd "$src/web"
    GOMAXPROCS=3 GOFLAGS="-p=3 '-overlay=$stage/overlay.json'" sh build-wasm.sh
)
cp "$src/web/src/api/wasm/"{wasmuks.ts,sqlite_bridge.ts,_gomuks.wasm} "$stage/"
cp "$(go env GOROOT)/lib/wasm/wasm_exec.js" "$stage/go_wasm_exec.js"
cp rust-client/web/worker/{package*.json,entry.js,profile-lock.js,vite.config.js} "$stage/"
(
    cd "$stage"
    patch --batch --fuzz=0 -p1 < "$root/rust-client/web/worker/startup.patch"
    npm ci --no-audit --no-fund
    ./node_modules/.bin/vite build
)
cd rust-client/web
CARGO_BUILD_JOBS=3 trunk build --no-sri "$@"
cp -R "$stage/dist" dist/backend

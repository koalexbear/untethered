$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
if (-not $env:UNTETHERED_PROFILE) {
    $env:UNTETHERED_PROFILE = Join-Path $root ".dev\rust-profile"
}
& cargo run --locked -j 3 --manifest-path (Join-Path $root "rust-client\Cargo.toml")
exit $LASTEXITCODE

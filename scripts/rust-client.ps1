$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$pin = Get-Content -Raw (Join-Path $PSScriptRoot "gomuks.env") | ConvertFrom-StringData
if (-not $env:UNTETHERED_PROFILE) {
    $env:UNTETHERED_PROFILE = Join-Path $root ".dev\rust-profile"
}
if (-not $env:UNTETHERED_BACKEND) {
    $dir = Join-Path $root ".dev\backends\$($pin.GOMUKS_VERSION)"
    $env:UNTETHERED_BACKEND = Join-Path $dir "gomuks.exe"
    if (-not (Test-Path $env:UNTETHERED_BACKEND)) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        $tmp = Join-Path $dir ([System.IO.Path]::GetRandomFileName())
        try {
            Invoke-WebRequest "https://github.com/gomuks/gomuks/releases/download/v$($pin.GOMUKS_VERSION)/gomuks-windows-amd64.exe" -OutFile $tmp
            if ((Get-FileHash $tmp -Algorithm SHA256).Hash -ne $pin.GOMUKS_WINDOWS_SHA256) {
                throw "Unexpected gomuks SHA-256"
            }
            Move-Item -Path $tmp -Destination $env:UNTETHERED_BACKEND
        } finally {
            if (Test-Path $tmp) { Remove-Item $tmp }
        }
    }
}
& cargo run --locked -j 3 --manifest-path (Join-Path $root "rust-client\Cargo.toml")
exit $LASTEXITCODE

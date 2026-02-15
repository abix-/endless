$ErrorActionPreference = 'SilentlyContinue'
Stop-Process -Name endless
$ErrorActionPreference = 'Continue'
Push-Location $PSScriptRoot\rust
cargo build --release
if ($LASTEXITCODE -eq 0) { cargo run --release }
Pop-Location

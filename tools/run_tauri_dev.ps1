$ErrorActionPreference = "Stop"

# Windows PowerShell 5.1 treats a bare -- after -File as an ambiguous script
# parameter. Keep both Tauri/Cargo separators inside a typed argument array,
# rather than passing them through another powershell.exe -File boundary.
$frontendRunner = Join-Path $PSScriptRoot "run_frontend.ps1"
& $frontendRunner -Action tauri -ExtraArgs @("dev", "--", "--", "/prefetch:2")
exit $LASTEXITCODE

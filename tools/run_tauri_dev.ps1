$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot 'development_port.ps1')
$previousPort = $env:ATSUMI_DEV_PORT
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeRoot = Join-Path $projectRoot '.runtime'
New-Item -ItemType Directory -Path $runtimeRoot -Force | Out-Null
$port = if ($previousPort) { Get-AtsumiDevelopmentPort -Candidates @(Get-AtsumiConfiguredDevelopmentPort) } else { Get-AtsumiDevelopmentPort }
$env:ATSUMI_DEV_PORT = [string]$port
# A file avoids Windows PowerShell stripping quotes from inline JSON CLI args.
# Reuse identical contents so unchanged launches keep Cargo's incremental cache.
$configPath = Join-Path $runtimeRoot 'tauri-dev-config.json'
$config = @{ build = @{ devUrl = "http://127.0.0.1:$port" } } | ConvertTo-Json -Compress
if (-not (Test-Path -LiteralPath $configPath) -or [IO.File]::ReadAllText($configPath) -cne $config) {
  [IO.File]::WriteAllText($configPath, $config, [Text.UTF8Encoding]::new($false))
}
Write-Output "Development endpoint: http://127.0.0.1:$port (Windows exclusions and other processes are unchanged)."

# Windows PowerShell 5.1 treats a bare -- after -File as an ambiguous script
# parameter. Keep both Tauri/Cargo separators inside a typed argument array,
# rather than passing them through another powershell.exe -File boundary.
$frontendRunner = Join-Path $PSScriptRoot "run_frontend.ps1"
try {
  & $frontendRunner -Action tauri -ExtraArgs @("dev", "--config", $configPath, "--", "--", "/prefetch:2")
  $result = $LASTEXITCODE
} finally { $env:ATSUMI_DEV_PORT = $previousPort }
exit $result

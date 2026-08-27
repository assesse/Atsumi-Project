[CmdletBinding()]
param(
  [switch]$CheckOnly
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$frontendRunner = Join-Path $PSScriptRoot "run_frontend.ps1"
$runtimeDirectory = Join-Path $projectRoot ".runtime"
$logPath = Join-Path $runtimeDirectory "debug-launch.log"
$standardOutput = Join-Path $runtimeDirectory "debug-app.stdout.log"
$standardError = Join-Path $runtimeDirectory "debug-app.stderr.log"
$launcherMutex = $null
$ownsLauncherMutex = $false

New-Item -ItemType Directory -Force -Path $runtimeDirectory | Out-Null
@(
  "Atsumi - current-source development app"
  "Started: $([DateTimeOffset]::Now.ToString('O'))"
  "Branch: $(git -C $projectRoot branch --show-current)"
  "Commit: $(git -C $projectRoot rev-parse --short HEAD)"
  ""
) | Set-Content -LiteralPath $logPath -Encoding UTF8

if (-not (Test-Path -LiteralPath $frontendRunner -PathType Leaf)) {
  "Missing tools\run_frontend.ps1" | Add-Content -LiteralPath $logPath -Encoding UTF8
  exit 1
}

if (-not (Test-Path -LiteralPath (Join-Path $projectRoot "node_modules\@tauri-apps\cli\tauri.js") -PathType Leaf)) {
  "Frontend dependencies are missing. Run pnpm install once." |
    Add-Content -LiteralPath $logPath -Encoding UTF8
  exit 1
}

if ($CheckOnly) {
  "Debug launcher check completed successfully." |
    Add-Content -LiteralPath $logPath -Encoding UTF8
  exit 0
}

$runningApp = Get-Process -Name "atsumi" -ErrorAction SilentlyContinue
if ($runningApp) {
  "An Atsumi process is already running. Close it before starting the current-source app." |
    Add-Content -LiteralPath $logPath -Encoding UTF8
  exit 74
}

$createdNew = $false
$launcherMutex = [System.Threading.Mutex]::new(
  $true,
  "Local\AtsumiNext.DebugLauncher",
  [ref]$createdNew
)
if (-not $createdNew) {
  $launcherMutex.Dispose()
  exit 73
}
$ownsLauncherMutex = $true

Remove-Item -LiteralPath $standardOutput, $standardError -Force -ErrorAction SilentlyContinue

Push-Location $projectRoot
try {
  "Starting Tauri development mode. Rust uses the incremental debug cache and frontend changes use Vite." |
    Add-Content -LiteralPath $logPath -Encoding UTF8

  $powershellPath = Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
  $previousErrorActionPreference = $ErrorActionPreference
  try {
    $ErrorActionPreference = "Continue"
    & $powershellPath `
      -NoLogo `
      -NoProfile `
      -NonInteractive `
      -ExecutionPolicy Bypass `
      -WindowStyle Hidden `
      -File $frontendRunner `
      tauri dev `
      1> $standardOutput `
      2> $standardError
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
  }

  if ($null -eq $exitCode) {
    $exitCode = 1
  }
  "Development process exit code: $exitCode" |
    Add-Content -LiteralPath $logPath -Encoding UTF8
  exit ([int]$exitCode)
} catch {
  ($_ | Out-String) | Add-Content -LiteralPath $logPath -Encoding UTF8
  exit 1
} finally {
  Pop-Location
  if ($ownsLauncherMutex) {
    $launcherMutex.ReleaseMutex()
  }
  if ($null -ne $launcherMutex) {
    $launcherMutex.Dispose()
  }
}

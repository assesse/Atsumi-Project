[CmdletBinding()]
param(
  [switch]$SkipInstall,
  [switch]$SkipRelease,
  [switch]$LiveSmoke
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$frontendRunner = Join-Path $PSScriptRoot "run_frontend.ps1"
$verificationDirectory = Join-Path $projectRoot ".runtime\verification"
$timestamp = [DateTimeOffset]::Now.ToString("yyyyMMdd-HHmmss")
$logPath = Join-Path $verificationDirectory "verify-$timestamp.log"
$env:CI = "true"
$previousCargoIncremental = $env:CARGO_INCREMENTAL
$env:CARGO_INCREMENTAL = "0"

New-Item -ItemType Directory -Force -Path $verificationDirectory | Out-Null

function Write-VerificationLog {
  param(
    [Parameter(Mandatory = $true)]
    [AllowEmptyString()]
    [string]$Message
  )

  $Message | Tee-Object -FilePath $logPath -Append
}

function Invoke-LoggedNative {
  param(
    [Parameter(Mandatory = $true)][string]$Label,
    [Parameter(Mandatory = $true)][string]$FilePath,
    [string[]]$Arguments = @()
  )

  Write-VerificationLog ""
  Write-VerificationLog "==> $Label"
  $previousPreference = $ErrorActionPreference
  $exitCode = $null
  try {
    # Windows PowerShell reports ordinary native stderr as NativeCommandError.
    # The native exit code remains the authoritative success signal.
    $ErrorActionPreference = "Continue"
    & $FilePath @Arguments 2>&1 |
      ForEach-Object { $_.ToString() } |
      Tee-Object -FilePath $logPath -Append
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previousPreference
  }

  if ($null -eq $exitCode) {
    throw "$Label did not report an exit code. See $logPath"
  }
  if ([int]$exitCode -ne 0) {
    throw "$Label failed with exit code $exitCode. See $logPath"
  }
}

function Resolve-SupportedNode {
  $candidates = @()
  $systemNode = Get-Command node.exe -ErrorAction SilentlyContinue
  if ($systemNode) {
    $candidates += $systemNode.Source
  }
  $bundledNode = Join-Path $env:USERPROFILE ".cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin\node.exe"
  if (Test-Path -LiteralPath $bundledNode -PathType Leaf) {
    $candidates += $bundledNode
  }

  foreach ($candidate in ($candidates | Select-Object -Unique)) {
    try {
      $version = [Version]((& $candidate --version).TrimStart("v").Split("-")[0])
      if (
        ($version.Major -eq 22 -and $version.Minor -ge 13) -or
        $version.Major -ge 24
      ) {
        return @{ Path = $candidate; Version = $version }
      }
    } catch {
      continue
    }
  }

  throw "Node.js 22.13+ or 24+ is required."
}

function Resolve-PnpmCommand {
  param([Parameter(Mandatory = $true)][string]$NodePath)

  $expectedVersion = "11.16.0"
  $localEntrypoint = Join-Path $projectRoot ".pnpm-store\tools\pnpm-11.16.0\package\bin\pnpm.cjs"
  if (Test-Path -LiteralPath $localEntrypoint -PathType Leaf) {
    $actualVersion = (& $NodePath $localEntrypoint --version).Trim()
    if ($actualVersion -eq $expectedVersion) {
      return @{ Path = $NodePath; Prefix = @($localEntrypoint); Version = $actualVersion }
    }
  }

  $pnpmCandidates = @(Get-Command pnpm.cmd, pnpm.exe -ErrorAction SilentlyContinue)
  foreach ($candidate in $pnpmCandidates) {
    try {
      $actualVersion = (& $candidate.Source --version).Trim()
      if ($actualVersion -eq $expectedVersion) {
        return @{ Path = $candidate.Source; Prefix = @(); Version = $actualVersion }
      }
    } catch {
      continue
    }
  }

  throw "pnpm 11.16.0 is required. Install the exact packageManager version from package.json."
}

$node = Resolve-SupportedNode
$pnpm = Resolve-PnpmCommand -NodePath $node.Path
$cargoCommand = Get-Command cargo.exe -ErrorAction SilentlyContinue
$gitCommand = Get-Command git.exe -ErrorAction SilentlyContinue
if (-not $cargoCommand) {
  throw "Rust/Cargo stable is required."
}
if (-not $gitCommand) {
  throw "Git is required."
}

$nodeDirectory = Split-Path -Parent $node.Path
if (($env:Path -split ";") -notcontains $nodeDirectory) {
  $env:Path = "$nodeDirectory;$env:Path"
}

Write-VerificationLog "Atsumi verification"
Write-VerificationLog "Started: $([DateTimeOffset]::Now.ToString('O'))"
Write-VerificationLog "Node.js: $($node.Version)"
Write-VerificationLog "pnpm: $($pnpm.Version)"

Push-Location $projectRoot
try {
  if (-not $SkipInstall) {
    Invoke-LoggedNative `
      -Label "Frozen frontend install" `
      -FilePath $pnpm.Path `
      -Arguments ($pnpm.Prefix + @("install", "--frozen-lockfile"))
  }

  $powershellPath = Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
  $frontendPrefix = @(
    "-NoLogo",
    "-NoProfile",
    "-NonInteractive",
    "-ExecutionPolicy", "Bypass",
    "-File", $frontendRunner
  )
  Invoke-LoggedNative -Label "Frontend tests" -FilePath $powershellPath -Arguments ($frontendPrefix + @("test"))
  Invoke-LoggedNative -Label "Frontend typecheck" -FilePath $powershellPath -Arguments ($frontendPrefix + @("typecheck"))
  Invoke-LoggedNative -Label "Frontend production build" -FilePath $powershellPath -Arguments ($frontendPrefix + @("build"))

  Invoke-LoggedNative -Label "Rust formatting" -FilePath $cargoCommand.Source -Arguments @("+stable", "fmt", "--manifest-path", "src-tauri/Cargo.toml", "--all", "--", "--check")
  Invoke-LoggedNative -Label "Rust check" -FilePath $cargoCommand.Source -Arguments @("+stable", "check", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--all-targets")
  Invoke-LoggedNative -Label "Rust tests" -FilePath $cargoCommand.Source -Arguments @("+stable", "test", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--all-targets")
  Invoke-LoggedNative -Label "Rust clippy" -FilePath $cargoCommand.Source -Arguments @("+stable", "clippy", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--all-targets", "--", "-D", "warnings")

  if ($LiveSmoke) {
    if ($env:ATSUMI_ALLOW_LIVE_SMOKE -ne "1") {
      throw "Set ATSUMI_ALLOW_LIVE_SMOKE=1 to permit the opt-in Hitomi network smoke test."
    }
    Invoke-LoggedNative -Label "Opt-in live Hitomi gallery 4113714 full pipeline smoke" -FilePath $cargoCommand.Source -Arguments @("+stable", "test", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "live_gallery_4113714_download_pipeline", "--", "--ignored", "--nocapture", "--test-threads=1")
  }

  if (-not $SkipRelease) {
    Invoke-LoggedNative -Label "Tauri release build (no bundle)" -FilePath $powershellPath -Arguments ($frontendPrefix + @("tauri", "build", "--no-bundle"))
  }

  Invoke-LoggedNative -Label "Git whitespace check" -FilePath $gitCommand.Source -Arguments @("diff", "--check")
  Write-VerificationLog ""
  Write-VerificationLog "Verification completed successfully."
  Write-VerificationLog "Log: $logPath"
} finally {
  Pop-Location
  if ($null -eq $previousCargoIncremental) {
    Remove-Item Env:CARGO_INCREMENTAL -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_INCREMENTAL = $previousCargoIncremental
  }
}

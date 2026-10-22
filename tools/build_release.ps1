param()
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'build_receipt.ps1')
$root = Split-Path -Parent $PSScriptRoot
# Unrecorded configuration overrides would weaken source/flag provenance. The
# signing key is deliberately neither inspected nor included in the receipt.
foreach($name in @('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER','RUSTC','TAURI_CONFIG','TAURI_DEBUG')) {
  if ([Environment]::GetEnvironmentVariable($name)) { throw 'Unsupported release-build configuration override; use reviewed source configuration.' }
}
if (@(Get-ChildItem Env: | Where-Object { $_.Name.StartsWith('VITE_') }).Count) { throw 'Unrecorded frontend environment overrides are not accepted in a release build.' }
# Explicit build entry point only. Preparation/download and signing-key setup
# are separate operator actions; verification never repairs missing resources.
$versions = {
  $values = [ordered]@{}
  foreach ($item in @(@{key='rustc';exe='rustc';args=@('-vV')},@{key='cargo';exe='cargo';args=@('--version')},@{key='node';exe='node';args=@('--version')},@{key='pnpm';exe='pnpm';args=@('--version')})) {
    $lines = & $item.exe @($item.args)
    if ($LASTEXITCODE -ne 0) { throw 'Compiler version collection failed.' }
    $values[$item.key] = ($lines -join "`n").Trim()
  }
  $sdk = $env:WindowsSDKVersion
  $values.windowsSdk = if ($sdk -and $sdk -match '^\d+(?:\.\d+){1,3}\\?$') { $sdk.TrimEnd('\') } else { 'not-exposed-by-build-environment' }
  $commit=& git -C $root rev-parse --verify HEAD
  if($LASTEXITCODE -ne 0){throw 'Cannot resolve source commit.'}
  $values.gitCommit=([string]$commit).Trim()
  [pscustomobject]$values
}
$build = {
  param($attempt,$version)
  $previous = $env:CARGO_TARGET_DIR
  $previousIncremental = $env:CARGO_INCREMENTAL
  try {
    $env:CARGO_TARGET_DIR = Join-Path $attempt 'target'
    $env:CARGO_INCREMENTAL = '0'
    Push-Location $root
    try {
      & pnpm exec tauri build --target x86_64-pc-windows-msvc --bundles nsis -- --locked | Out-Host
      $code = $LASTEXITCODE
    } finally { Pop-Location }
    return [int]$code
  } finally { $env:CARGO_TARGET_DIR = $previous; $env:CARGO_INCREMENTAL = $previousIncremental }
}
$receiptPath = Invoke-AtsumiBuildReceipt $root $build $versions
Write-Output $receiptPath

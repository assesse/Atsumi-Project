param([switch]$VerifyOnly, [switch]$ReturnManifest)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot 'package_manifest.ps1')

function Assert-MediaToolPath {
  param([string]$Path, [switch]$AllowMissing)
  $itemPath = [IO.Path]::GetFullPath($Path)
  if (-not $AllowMissing -and -not (Test-Path -LiteralPath $itemPath)) {
    throw "Media tools are incomplete. Preserved package requires inspection."
  }
  # Regular files inside a junction are not trusted paths either.
  while ($itemPath) {
    if (Test-Path -LiteralPath $itemPath) {
      $item = Get-Item -LiteralPath $itemPath -Force
      if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Media tool path is a reparse point. Preserved package requires inspection."
      }
    }
    $itemPath = [IO.Path]::GetDirectoryName($itemPath)
  }
}

function Receive-MediaArchive([string]$Source, [string]$Destination, [string]$Hash) {
  $request = [Net.HttpWebRequest]::Create($Source)
  $request.Timeout = 180000; $request.ReadWriteTimeout = 180000
  $response = $request.GetResponse()
  try {
    $stream = $response.GetResponseStream()
    try { Copy-AtsumiVerifiedStream $stream $Destination @{length=-1;sha256=$Hash} } finally { $stream.Dispose() }
  } finally { $response.Dispose() }
}

$projectRoot = Split-Path -Parent $PSScriptRoot
$packageName = "ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0"
$expectedHash = "40eec25b2f55dcad7e4d4e640919b920d29818b56fdaf9353ce1fd8adefc9d6b"
$url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-09-11-13-20/$packageName.zip"
# Upstream daily assets expire. Preserve the exact approved archive, including
# its license files, for reproducible clean builds; never fall back to latest.
$mirrorUrl = "https://github.com/assesse/Atsumi-Project/releases/download/v2.0.0/$packageName.zip"
$runtimeRoot = Join-Path $projectRoot ".runtime\media-tools"
$archive = Join-Path $runtimeRoot "$packageName.zip"
$packageRoot = Join-Path $runtimeRoot $packageName
$ffmpeg = Join-Path $packageRoot "bin\ffmpeg.exe"
$ffprobe = Join-Path $packageRoot "bin\ffprobe.exe"
# Build-time dependency only: never install a global program or alter PATH.
Assert-MediaToolPath $runtimeRoot -AllowMissing
if (-not $VerifyOnly) { New-AtsumiSafeDirectory $runtimeRoot }
Assert-MediaToolPath $archive -AllowMissing
if (-not (Test-Path -LiteralPath $archive)) {
  if ($VerifyOnly) { throw 'Missing approved media archive; verification never downloads or repairs.' }
  $pending = $archive + '.download-' + [Guid]::NewGuid().ToString('N')
  Assert-MediaToolPath $pending -AllowMissing
  $lease = Lock-AtsumiPackageDirectories @($archive)
  try {
  try {
    Receive-MediaArchive $url $pending $expectedHash
  } catch {
    Write-Warning "Upstream pinned media archive is unavailable; trying the byte-identical Atsumi release mirror."
    # Preserve a failed partial download, including when retrying the mirror.
    $pending = $archive + '.download-' + [Guid]::NewGuid().ToString('N')
    Receive-MediaArchive $mirrorUrl $pending $expectedHash
  }
  [IO.File]::Move($pending, $archive)
  } finally { $lease.Dispose() }
}
if ((Get-AtsumiFileProof $archive).sha256 -cne $expectedHash) {
  throw "Media tool archive checksum mismatch. The downloaded file was preserved; do not execute it."
}
$manifest = Get-AtsumiArchiveManifest $archive $packageName $expectedHash
foreach ($required in @('bin/ffmpeg.exe','bin/ffprobe.exe','LICENSE.txt')) {
  if ($required -cnotin @($manifest.files | ForEach-Object { $_.path })) { throw 'Approved media archive is incomplete.' }
}
Assert-MediaToolPath $packageRoot -AllowMissing
if (-not (Test-Path -LiteralPath $packageRoot)) {
  if ($VerifyOnly) { throw 'Missing media package; verification never downloads or repairs.' }
  Expand-AtsumiPackageArchive $archive $packageName $packageRoot $manifest
}
# Every shipped entry, including license/docs/headers, is immutable and exact.
Assert-AtsumiPackageTree $packageRoot $manifest
if ($ReturnManifest) { return [pscustomobject]@{ package=$packageRoot; manifest=$manifest } }
Write-Output "Recording merge tools verified: $packageRoot"

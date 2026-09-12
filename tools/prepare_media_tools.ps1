param()
$ErrorActionPreference = "Stop"

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

function Assert-MediaToolBinary {
  param($Entry, [string]$Path)
  Assert-MediaToolPath $Path
  $item = Get-Item -LiteralPath $Path -Force
  if ($item.PSIsContainer -or $item.Length -ne $Entry.Length) {
    throw "Media tool binary size mismatch. Preserved package requires inspection."
  }
  $stream = $Entry.Open()
  $sha = [Security.Cryptography.SHA256]::Create()
  try {
    $expected = [BitConverter]::ToString($sha.ComputeHash($stream)).Replace("-", "")
  } finally { $stream.Dispose(); $sha.Dispose() }
  if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ne $expected) {
    throw "Media tool binary checksum mismatch. Preserved package requires inspection."
  }
}

$projectRoot = Split-Path -Parent $PSScriptRoot
$packageName = "ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0"
$expectedHash = "40eec25b2f55dcad7e4d4e640919b920d29818b56fdaf9353ce1fd8adefc9d6b"
$url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-09-11-13-20/$packageName.zip"
$runtimeRoot = Join-Path $projectRoot ".runtime\media-tools"
$archive = Join-Path $runtimeRoot "$packageName.zip"
$packageRoot = Join-Path $runtimeRoot $packageName
$ffmpeg = Join-Path $packageRoot "bin\ffmpeg.exe"
$ffprobe = Join-Path $packageRoot "bin\ffprobe.exe"
# Build-time dependency only: never install a global program or alter PATH.
Assert-MediaToolPath $runtimeRoot -AllowMissing
New-Item -ItemType Directory -Path $runtimeRoot -Force | Out-Null
Assert-MediaToolPath $archive -AllowMissing
if (-not (Test-Path -LiteralPath $archive)) {
  Invoke-WebRequest -Uri $url -OutFile $archive
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHash) {
  throw "Media tool archive checksum mismatch. The downloaded file was preserved; do not execute it."
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead($archive)
try {
  $allowedRoot = [IO.Path]::GetFullPath($packageRoot)
  $allowedPrefix = $allowedRoot + [IO.Path]::DirectorySeparatorChar
  $binPrefix = [IO.Path]::GetFullPath((Join-Path $packageRoot "bin")) + [IO.Path]::DirectorySeparatorChar
  $paths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  $binaries = @{}
  # Validate the ZIP even when reusing an already extracted package.
  foreach ($entry in $zip.Entries) {
    if ([IO.Path]::IsPathRooted($entry.FullName) -or $entry.FullName.Contains(":")) {
      throw "Unsafe archive path."
    }
    $entryPath = [IO.Path]::GetFullPath((Join-Path $runtimeRoot $entry.FullName))
    if (($entryPath -ne $allowedRoot -and -not $entryPath.StartsWith($allowedPrefix, [StringComparison]::OrdinalIgnoreCase)) -or -not $paths.Add($entryPath)) {
      throw "Unsafe or duplicate archive path."
    }
    if ($entryPath.StartsWith($binPrefix, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetExtension($entryPath) -in @(".exe", ".dll")) {
      $binaries[$entryPath] = $entry
    }
  }
  if (-not $binaries.ContainsKey($ffmpeg) -or -not $binaries.ContainsKey($ffprobe)) {
    throw "Approved archive is missing required media tools."
  }
  Assert-MediaToolPath $packageRoot -AllowMissing
  if (-not (Test-Path -LiteralPath $packageRoot)) {
    Expand-Archive -LiteralPath $archive -DestinationPath $runtimeRoot
  }
  foreach ($path in $binaries.Keys) {
    Assert-MediaToolBinary $binaries[$path] $path
  }
  # Extra local DLLs could load even when all approved files match.
  foreach ($item in (Get-ChildItem -LiteralPath (Join-Path $packageRoot "bin") -Force)) {
    Assert-MediaToolPath $item.FullName
    if ($item.PSIsContainer -or ($item.Extension -in @(".exe", ".dll") -and -not $binaries.ContainsKey($item.FullName))) {
      throw "Unexpected media tool entry. Preserved package requires inspection."
    }
  }
} finally { $zip.Dispose() }
Write-Output "Recording merge tools ready: $packageRoot"

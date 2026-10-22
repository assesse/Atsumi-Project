param([switch]$VerifyOnly, [switch]$ReturnManifest)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'package_manifest.ps1')

# Requirements R002/R003/R015, DOC-049/DOC-055/DOC-080: managed local tools,
# reproducible portable resources, no PATH install, no unpinned self-update.
$platformRoot = Split-Path -Parent $PSScriptRoot
$platformCache = Join-Path $platformRoot '.runtime/platform-tool-cache'
$platformPackage = Join-Path $platformRoot '.runtime/platform-tools'

function Assert-PlatformPath([string]$Path) {
  $candidate = [IO.Path]::GetFullPath($Path)
  $allowed = [IO.Path]::GetFullPath((Join-Path $platformRoot '.runtime')) + [IO.Path]::DirectorySeparatorChar
  if (-not $candidate.StartsWith($allowed, [StringComparison]::OrdinalIgnoreCase)) { throw 'Platform tool path escaped the workspace runtime.' }
  while ($candidate) {
    if (Test-Path -LiteralPath $candidate) {
      if (((Get-Item -LiteralPath $candidate -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Platform tool path must not contain a reparse point.' }
    }
    $candidate = [IO.Path]::GetDirectoryName($candidate)
  }
}

function Get-VerifiedPlatformFile([string]$Url, [string]$Path, [string]$Hash) {
  Assert-PlatformPath $Path
  if (Test-Path -LiteralPath $Path) {
    if ((Get-AtsumiFileProof $Path).sha256 -cne $Hash.ToLowerInvariant()) { throw 'Platform tool checksum mismatch; existing file preserved.' }
    return
  }
  if ($VerifyOnly) { throw 'Missing platform tool input; verification never downloads or repairs.' }
  $pending = $Path + '.download-' + [Guid]::NewGuid().ToString('N')
  Assert-PlatformPath $pending
  $lease = Lock-AtsumiPackageDirectories @($Path)
  try {
    $request = [Net.HttpWebRequest]::Create($Url)
    $request.Timeout = 180000; $request.ReadWriteTimeout = 180000
    $response = $request.GetResponse()
    try {
      $stream = $response.GetResponseStream()
      try { Copy-AtsumiVerifiedStream $stream $pending @{length=-1;sha256=$Hash} } finally { $stream.Dispose() }
    } finally { $response.Dispose() }
    [IO.File]::Move($pending, $Path)
  } finally { $lease.Dispose() }
}

foreach ($directory in @($platformCache, $platformPackage)) {
  Assert-PlatformPath $directory
}
if (-not $VerifyOnly) { New-AtsumiSafeDirectory $platformCache }
$denoArchive = Join-Path $platformCache 'deno-v2.9.7-x86_64-pc-windows-msvc.zip'
Get-VerifiedPlatformFile 'https://github.com/denoland/deno/releases/download/v2.9.7/deno-x86_64-pc-windows-msvc.zip' $denoArchive 'A0C3101B4158D1DFB7D6A78A7BF0F3DE80C96BB423C152BEEC8BEB22786F2238'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archiveLease = Lock-AtsumiPackageDirectories @($denoArchive)
$archiveStream = $null
try {
  $archiveStream = [Atsumi.PackageDirectoryLease]::OpenRegularRead($denoArchive)
  if ((Get-AtsumiStreamProof $archiveStream $archiveStream.Length).sha256 -cne 'a0c3101b4158d1dfb7d6a78a7bf0f3de80c96bb423c152beec8beb22786f2238') { throw 'Approved Deno archive changed before use.' }
  $archiveStream.Position = 0
  $denoZip = [IO.Compression.ZipArchive]::new($archiveStream,[IO.Compression.ZipArchiveMode]::Read,$true)
  try {
  if ($denoZip.Entries.Count -ne 1 -or $denoZip.Entries[0].FullName -cne 'deno.exe') { throw 'Unexpected Deno archive entries.' }
  $denoEntry = $denoZip.Entries[0]
  $unixType = ($denoEntry.ExternalAttributes -shr 16) -band 0xf000
  if ($unixType -notin @(0,0x8000) -or ($denoEntry.ExternalAttributes -band 0x410)) { throw 'Deno archive entry is not a regular file.' }
  $entryStream = $denoEntry.Open()
  try { $denoProof = Get-AtsumiStreamProof $entryStream $denoEntry.Length } finally { $entryStream.Dispose() }
  $sources = @(
    @{path='bin/yt-dlp.exe';length=17840399;sha256='66674953FE251B89F4D08C5F0E35E0728679BD67AB3D7D05C0562AF101DD3E7A';url='https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp.exe'},
    @{path='licenses/yt-dlp-LICENSE.txt';length=1211;sha256='7E12E5DF4BAE12CB21581BA157CED20E1986A0508DD10D0E8A4AB9A4CF94E85C';url='https://raw.githubusercontent.com/yt-dlp/yt-dlp/2026.08.19/LICENSE'},
    # PyInstaller's combined binary is GPLv3+, not merely the source Unlicense.
    @{path='licenses/yt-dlp-THIRD_PARTY_LICENSES.txt';length=243550;sha256='472AEFE951C7DB35E1657C1D13FD337140511ED6F2B329205105AD441C5A02B7';url='https://raw.githubusercontent.com/yt-dlp/yt-dlp/2026.08.19/THIRD_PARTY_LICENSES.txt'},
    @{path='licenses/deno-LICENSE.txt';length=1074;sha256='F62497FFFECC0852960C8D3E6934B9DB86D16396E9B604072E923892CAE3A588';url='https://raw.githubusercontent.com/denoland/deno/v2.9.7/LICENSE.md'}
  )
  $manifest = New-AtsumiPackageManifest (@($sources) + @([pscustomobject]@{path='bin/deno.exe';length=$denoProof.length;sha256=$denoProof.sha256}))
  if (Test-Path -LiteralPath $platformPackage) {
    # A reused package is never repaired piecemeal, including missing licenses.
    Assert-AtsumiPackageTree $platformPackage $manifest
  } else {
    if ($VerifyOnly) { throw 'Missing platform package; verification never downloads or repairs.' }
    New-AtsumiPackageDirectories $platformPackage $manifest
    foreach ($source in $sources) { Get-VerifiedPlatformFile $source.url (Join-Path $platformPackage $source.path) $source.sha256 }
    $stream = $denoEntry.Open()
    try { Copy-AtsumiVerifiedStream $stream (Join-Path $platformPackage 'bin/deno.exe') $denoProof } finally { $stream.Dispose() }
    Assert-AtsumiPackageTree $platformPackage $manifest
  }
  } finally { $denoZip.Dispose() }
} finally { if ($archiveStream) { $archiveStream.Dispose() }; $archiveLease.Dispose() }
if ($ReturnManifest) { return [pscustomobject]@{ package=$platformPackage; manifest=$manifest } }
[pscustomobject]@{ package=$platformPackage; ytDlp='2026.08.19'; deno='2.9.7'; manifestVerified=$true } | ConvertTo-Json -Compress

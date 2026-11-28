param([switch]$VerifyOnly, [switch]$ReturnManifest)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'package_manifest.ps1')

$root = Split-Path -Parent $PSScriptRoot
$cache = Join-Path $root '.runtime/browser-tool-cache'
$directory = Join-Path $root '.runtime/browser-tools'

function Assert-BrowserPath([string]$Path) {
  $candidate = [IO.Path]::GetFullPath($Path)
  $allowed = [IO.Path]::GetFullPath((Join-Path $root '.runtime')) + [IO.Path]::DirectorySeparatorChar
  if (-not $candidate.StartsWith($allowed, [StringComparison]::OrdinalIgnoreCase)) { throw 'Browser tools escaped the workspace.' }
  while ($candidate) {
    if ((Test-Path -LiteralPath $candidate) -and ((Get-Item -LiteralPath $candidate -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Browser tool path is redirected.' }
    $candidate = [IO.Path]::GetDirectoryName($candidate)
  }
}

Assert-BrowserPath $cache
Assert-BrowserPath $directory
$engine = Join-Path $directory 'wireproxy.exe'
$archive = Join-Path $cache 'wireproxy-1.1.3-windows-amd64.tar.gz'
$license = Join-Path $directory 'wireproxy-LICENSE.txt'
$engineHash = 'b176b561fd8bf15d828fcab484cfd5b4fb941cb9f61807901ca64b955af27e1f'
$archiveHash = 'bce041ea9fe0f8a3351301dcbe29cdf6a523bb25cf9c62f17ebb5699a8051d0f'
$licenseHash = '12e05cf70e2876d0cfd1c649e1c0032f82a370b365dcccd7d5a03feee17f2625'
$manifest = New-AtsumiPackageManifest @(
  @{path='wireproxy.exe';length=10666496;sha256=$engineHash},
  @{path='wireproxy-LICENSE.txt';length=745;sha256=$licenseHash}
)

function Assert-BrowserHash([string]$Path, [string]$Hash) {
  Assert-BrowserPath $Path
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Missing browser tool: $Path" }
  if ((Get-Item -LiteralPath $Path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Redirected browser tool.' }
  if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $Hash) { throw 'Browser tool hash mismatch; existing file preserved.' }
}

if (Test-Path -LiteralPath $directory) {
  Assert-AtsumiPackageTree $directory $manifest
} else {
  if ($VerifyOnly) { throw 'Browser engine is missing; verification never downloads.' }
  New-AtsumiSafeDirectory $cache
  New-AtsumiPackageDirectories $directory $manifest
  if (-not (Test-Path -LiteralPath $archive)) { Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/windtf/wireproxy/releases/download/v1.1.3/wireproxy_windows_amd64.tar.gz' -OutFile $archive }
  Assert-BrowserHash $archive $archiveHash
  $entries = @(& tar.exe -tzf $archive)
  if ($LASTEXITCODE -ne 0 -or $entries.Count -ne 1 -or $entries[0] -cne 'wireproxy.exe') { throw 'Unexpected browser engine archive entries.' }
  & tar.exe -xzf $archive -C $directory
  if ($LASTEXITCODE -ne 0) { throw 'Browser engine extraction failed.' }
  Assert-BrowserHash $engine $engineHash
  Invoke-WebRequest -UseBasicParsing -Uri 'https://raw.githubusercontent.com/windtf/wireproxy/v1.1.3/LICENSE' -OutFile $license
  Assert-BrowserHash $license $licenseHash
  Assert-AtsumiPackageTree $directory $manifest
}

if ($ReturnManifest) { return [pscustomobject]@{ package=$directory; manifest=$manifest } }
if (-not $VerifyOnly) { Write-Output 'Browser engine v1.1.3 verified.' }

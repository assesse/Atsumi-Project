param(
  [string]$OutputDirectory,
  [string]$WebView2RuntimeDirectory
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$configuration = Get-Content -LiteralPath (Join-Path $projectRoot 'src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json
$executable = Join-Path $projectRoot 'src-tauri/target/release/atsumi.exe'
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw 'Build the release executable first. No files were removed.' }
$sourceVersion = (Get-Item -LiteralPath $executable).VersionInfo.ProductVersion
if ($sourceVersion -and $sourceVersion -ne $configuration.version) { throw 'The release executable version does not match the source version.' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $projectRoot ('.runtime/portable-v' + $configuration.version) }
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
$payload = Join-Path $outputRoot 'Atsumi'
$archive = Join-Path $outputRoot 'Atsumi-Portable.zip'
# Never replace an old portable build, executable, or user folder.
if ((Test-Path -LiteralPath $payload) -or (Test-Path -LiteralPath $archive)) { throw 'Output already exists. Use a new empty output directory; existing files are preserved.' }
if ($WebView2RuntimeDirectory) {
  $runtime = (Resolve-Path -LiteralPath $WebView2RuntimeDirectory).Path
  if (-not (Test-Path -LiteralPath (Join-Path $runtime 'msedgewebview2.exe') -PathType Leaf)) { throw 'Use an extracted Microsoft Fixed Version WebView2 runtime directory.' }
}
# Validates the pinned archive, all bundled tools, and license files.
& (Join-Path $PSScriptRoot 'prepare_media_tools.ps1')
$resources = $configuration.bundle.resources.PSObject.Properties
foreach ($resource in $resources) {
  $source = [IO.Path]::GetFullPath((Join-Path (Join-Path $projectRoot 'src-tauri') $resource.Name))
  if (-not (Test-Path -LiteralPath $source -PathType Container)) { throw "Missing resource directory: $($resource.Value)" }
  $destination = [IO.Path]::GetFullPath((Join-Path $payload $resource.Value))
  if (-not $destination.StartsWith($payload + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Resource target is outside the portable package.' }
}
New-Item -ItemType Directory -Path $payload | Out-Null
Copy-Item -LiteralPath $executable -Destination (Join-Path $payload 'atsumi.exe')
foreach ($resource in $resources) {
  $source = [IO.Path]::GetFullPath((Join-Path (Join-Path $projectRoot 'src-tauri') $resource.Name))
  $destination = [IO.Path]::GetFullPath((Join-Path $payload $resource.Value))
  New-Item -ItemType Directory -Path $destination -Force | Out-Null
  Get-ChildItem -LiteralPath $source -Force | Copy-Item -Destination $destination -Recurse
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'portable/atsumi-portable.json') -Destination $payload
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'portable/Start-Atsumi.cmd') -Destination $payload
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'portable/Readme.txt') -Destination $payload
if ($WebView2RuntimeDirectory) { Copy-Item -LiteralPath $runtime -Destination (Join-Path $payload 'WebView2') -Recurse }
# Every DLL and license is retained, not only ffmpeg.exe/ffprobe.exe.
$mediaBin = Join-Path $payload 'media-tools/bin'
foreach ($name in @('ffmpeg.exe','ffprobe.exe')) {
  if (-not (Test-Path -LiteralPath (Join-Path $mediaBin $name) -PathType Leaf)) { throw "Incomplete portable media tools: $name" }
}
if (-not (Get-ChildItem -LiteralPath $mediaBin -Filter '*.dll')) { throw 'Missing shared media DLLs.' }
Compress-Archive -LiteralPath $payload -DestinationPath $archive -CompressionLevel Optimal
[pscustomobject]@{ archive=$archive; bytes=(Get-Item -LiteralPath $archive).Length; sha256=(Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash; bundledWebView2=[bool]$WebView2RuntimeDirectory } | ConvertTo-Json -Compress

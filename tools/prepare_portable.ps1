param(
  [Parameter(Mandatory=$true)][string]$BuildReceipt,
  [string]$OutputDirectory,
  [string]$WebView2RuntimeDirectory
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'build_receipt.ps1')
$projectRoot = Split-Path -Parent $PSScriptRoot
$configuration = Get-Content -LiteralPath (Join-Path $projectRoot 'src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json
$buildProof = Assert-AtsumiBuildReceipt $projectRoot $BuildReceipt
$executable = Join-Path $buildProof.stage 'atsumi.exe'
$artifact = Get-AtsumiPortableArtifact $configuration.version ([bool]$WebView2RuntimeDirectory)
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $projectRoot ('.runtime/' + $artifact.directory) }
$outputRoot = Get-AtsumiPackageDirectoryPath $OutputDirectory
$payload = Join-Path $outputRoot 'Atsumi'
$archive = Join-Path $outputRoot $artifact.name
$metadataPath = $archive + '.metadata.json'
Assert-AtsumiPackagePath $outputRoot -AllowMissing
# Never replace an old portable build, executable, or user folder.
if ((Test-Path -LiteralPath $payload) -or (Test-Path -LiteralPath $archive) -or (Test-Path -LiteralPath $metadataPath)) { throw 'Output already exists. Use a new empty output directory; existing files are preserved.' }
if ($WebView2RuntimeDirectory) {
  $runtime = (Resolve-Path -LiteralPath $WebView2RuntimeDirectory).Path
  if (-not (Test-Path -LiteralPath (Join-Path $runtime 'msedgewebview2.exe') -PathType Leaf)) { throw 'Use an extracted Microsoft Fixed Version WebView2 runtime directory.' }
  $runtimeManifest = Get-AtsumiDirectoryManifest $runtime
}
# Build admission never downloads, repairs or substitutes resource inputs.
$media = & (Join-Path $PSScriptRoot 'prepare_media_tools.ps1') -VerifyOnly -ReturnManifest
$platform = & (Join-Path $PSScriptRoot 'prepare_platform_tools.ps1') -VerifyOnly -ReturnManifest
$approved = @{}
$approved[(Get-AtsumiPackageDirectoryPath $media.package)] = $media
$approved[(Get-AtsumiPackageDirectoryPath $platform.package)] = $platform
$resources = $configuration.bundle.resources.PSObject.Properties
$expectedFiles = [Collections.Generic.List[object]]::new()
$expectedDirectories = [Collections.Generic.List[string]]::new()
foreach ($resource in $resources) {
  $source = Get-AtsumiPackageDirectoryPath (Join-Path (Join-Path $projectRoot 'src-tauri') $resource.Name)
  if (-not (Test-Path -LiteralPath $source -PathType Container)) { throw "Missing resource directory: $($resource.Value)" }
  if (-not $approved.ContainsKey($source)) { throw 'Resource has no approved positive manifest.' }
  $destination = Get-AtsumiPackageDirectoryPath (Join-Path $payload $resource.Value)
  if (-not $destination.StartsWith($payload + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Resource target is outside the portable package.' }
  # 2.1.1-2.2.1 updaters accept only their existing application-owned roots.
  # New managed tools must stay inside media-tools; broadening today's updater
  # would not make an already-installed updater accept a new top-level folder.
  $relativeResource = $destination.Substring($payload.Length + 1).Replace('\', '/')
  $relativeResource = ConvertTo-AtsumiPackagePath $relativeResource
  if ($relativeResource.Split('/')[0] -ne 'media-tools') { throw 'Portable resources must remain under the legacy media-tools update root.' }
  $expectedDirectories.Add($relativeResource)
  foreach ($directory in $approved[$source].manifest.directories) { $expectedDirectories.Add($relativeResource + '/' + $directory) }
  foreach ($file in $approved[$source].manifest.files) { $expectedFiles.Add([pscustomobject]@{path=$relativeResource + '/' + $file.path;length=$file.length;sha256=$file.sha256}) }
}
# Read-only revalidation immediately before copy: no missing input is fetched now.
$null = & (Join-Path $PSScriptRoot 'prepare_media_tools.ps1') -VerifyOnly -ReturnManifest
$null = & (Join-Path $PSScriptRoot 'prepare_platform_tools.ps1') -VerifyOnly -ReturnManifest
$null = Assert-AtsumiBuildReceipt $projectRoot $BuildReceipt
Assert-AtsumiPackagePath $payload -AllowMissing
New-AtsumiSafeDirectory $payload -MustCreate
foreach ($resource in $resources) {
  $source = Get-AtsumiPackageDirectoryPath (Join-Path (Join-Path $projectRoot 'src-tauri') $resource.Name)
  $destination = Get-AtsumiPackageDirectoryPath (Join-Path $payload $resource.Value)
  Copy-AtsumiPackageTree $source $destination $approved[$source].manifest
}
foreach ($item in @(@{path='atsumi.exe';source=$executable},@{path='Start-Atsumi.cmd';source=(Join-Path $PSScriptRoot 'portable/Start-Atsumi.cmd')},@{path='Readme.txt';source=(Join-Path $PSScriptRoot 'portable/Readme.txt')})) {
  $proof = Get-AtsumiFileProof $item.source
  Copy-AtsumiVerifiedFile $item.source (Join-Path $payload $item.path) $proof
  $expectedFiles.Add([pscustomobject]@{path=$item.path;length=$proof.length;sha256=$proof.sha256})
}
$markerPath = Join-Path $PSScriptRoot 'portable/atsumi-portable.json'
Assert-AtsumiPackagePath $markerPath
$marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
$marker | Add-Member -NotePropertyName version -NotePropertyValue $configuration.version
$marker | Add-Member -NotePropertyName packageKind -NotePropertyValue $artifact.kind
$marker | Add-Member -NotePropertyName updaterCompatible -NotePropertyValue $artifact.updaterCompatible
$utf8 = [Text.UTF8Encoding]::new($false)
$proof = Write-AtsumiPackageBytes (Join-Path $payload 'atsumi-portable.json') ($utf8.GetBytes(($marker | ConvertTo-Json -Compress)))
$expectedFiles.Add([pscustomobject]@{path='atsumi-portable.json';length=$proof.length;sha256=$proof.sha256})
if ($WebView2RuntimeDirectory) {
  Copy-AtsumiPackageTree $runtime (Join-Path $payload 'WebView2') $runtimeManifest
  $expectedDirectories.Add('WebView2')
  foreach ($directory in $runtimeManifest.directories) { $expectedDirectories.Add('WebView2/' + $directory) }
  foreach ($file in $runtimeManifest.files) { $expectedFiles.Add([pscustomobject]@{path='WebView2/' + $file.path;length=$file.length;sha256=$file.sha256}) }
}
$manifest = New-AtsumiPackageManifest $expectedFiles.ToArray() $expectedDirectories.ToArray()
$finalProof = Assert-AtsumiBuildReceipt $projectRoot $BuildReceipt
if ($finalProof.receiptSha256 -cne $buildProof.receiptSha256) { throw 'Build receipt changed during packaging.' }
$exeProof = @($buildProof.outputs.files | Where-Object { $_.path -ceq 'atsumi.exe' })[0]
$packagedExe = @($manifest.files | Where-Object { $_.path -ceq 'atsumi.exe' })[0]
if ($packagedExe.sha256 -cne $exeProof.sha256 -or $packagedExe.length -ne $exeProof.length) { throw 'Captured build executable changed before portable copy.' }
New-AtsumiPackageArchive $payload $archive $manifest
$proof = Get-AtsumiFileProof $archive
$null = Assert-AtsumiBuildReceipt $projectRoot $BuildReceipt
$metadata = [pscustomobject]@{format=1;version=$configuration.version;buildId=$buildProof.buildId;buildReceiptSha256=$buildProof.receiptSha256;kind=$artifact.kind;updaterCompatible=$artifact.updaterCompatible;archiveName=$artifact.name;bytes=$proof.length;sha256=$proof.sha256;bundledWebView2=[bool]$WebView2RuntimeDirectory;manifest=$manifest}
$null = Write-AtsumiPackageBytes $metadataPath ($utf8.GetBytes(($metadata | ConvertTo-Json -Depth 8)))
if ($artifact.updaterCompatible) { Assert-AtsumiUpdaterArchive $archive $configuration.version }
[pscustomobject]@{ archive=$archive; metadata=$metadataPath; bytes=$proof.length; sha256=$proof.sha256; bundledWebView2=[bool]$WebView2RuntimeDirectory; updaterCompatible=$artifact.updaterCompatible } | ConvertTo-Json -Compress

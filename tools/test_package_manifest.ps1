param()
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'package_manifest.ps1')
$fixtureRoot = Join-Path (Split-Path -Parent $PSScriptRoot) ('.runtime/package-manifest-tests-' + [Guid]::NewGuid().ToString('N'))
New-AtsumiSafeDirectory $fixtureRoot -MustCreate
$script:checks = [Collections.Generic.List[string]]::new()
$utf8 = [Text.UTF8Encoding]::new($false)

function Assert-Fixture([bool]$Condition, [string]$Name) {
  if (-not $Condition) { throw "Synthetic check failed: $Name" }
  $script:checks.Add($Name)
}
function Assert-Rejected([scriptblock]$Action, [string]$Name) {
  $rejected = $false
  try { & $Action | Out-Null } catch { $rejected = $true }
  Assert-Fixture $rejected $Name
}
function New-FixtureArchive([string]$Name, [object[]]$Entries) {
  $path = Join-Path $fixtureRoot ($Name + '.zip')
  $output = [IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
  $zip = [IO.Compression.ZipArchive]::new($output,[IO.Compression.ZipArchiveMode]::Create,$true)
  try {
    foreach ($item in $Entries) {
      $entry = $zip.CreateEntry($item.path)
      if ($item.ContainsKey('attributes')) { $entry.ExternalAttributes = $item.attributes }
      $stream = $entry.Open()
      try { $bytes=$utf8.GetBytes([string]$item.text); $stream.Write($bytes,0,$bytes.Length) } finally { $stream.Dispose() }
    }
  } finally { $zip.Dispose(); $output.Dispose() }
  return $path
}
function Write-FixtureMetadata([string]$Archive, $Manifest, [string]$Kind='portable-updater', [bool]$Runtime=$false) {
  $proof = Get-AtsumiFileProof $Archive
  $metadata = @{format=1;version='9.8.7';kind=$Kind;updaterCompatible=(-not $Runtime);bundledWebView2=$Runtime;archiveName=[IO.Path]::GetFileName($Archive);bytes=$proof.length;sha256=$proof.sha256;manifest=$Manifest}
  $null = Write-AtsumiPackageBytes ($Archive+'.metadata.json') ($utf8.GetBytes(($metadata | ConvertTo-Json -Depth 8)))
}

$approved = New-FixtureArchive 'approved' @(
  @{path='tools/';text=''},@{path='tools/bin/tool.exe';text='synthetic never executed'},
  @{path='tools/LICENSE.txt';text='test license'},@{path='tools/doc/readme.txt';text='test documentation'},@{path='tools/empty/';text=''}
)
$manifest = Get-AtsumiArchiveManifest $approved 'tools'
$mappedSource = Get-AtsumiPackageDirectoryPath (Join-Path $fixtureRoot 'source/')
$mappedDestination = Get-AtsumiPackageDirectoryPath (Join-Path $fixtureRoot 'Atsumi/media-tools/')
Assert-Fixture ($mappedSource -ceq (Join-Path $fixtureRoot 'source') -and (ConvertTo-AtsumiPackagePath $mappedDestination.Substring($fixtureRoot.Length + 1)) -ceq 'Atsumi/media-tools') 'configured resource directory trailing separators normalize before exact lookup and manifest path validation'
$approvedProof = Get-AtsumiFileProof $approved
Assert-AtsumiManifestEqual $manifest (Get-AtsumiArchiveManifest $approved 'tools' $approvedProof.sha256)
Assert-Rejected { Get-AtsumiArchiveManifest $approved 'tools' ('0' * 64) } 'archive hash is checked on the same handle used to derive the manifest'
Assert-Fixture ($manifest.files.Count -eq 3 -and 'doc' -cin $manifest.directories -and 'empty' -cin $manifest.directories) 'archive manifest includes licenses, docs and empty directories'
$source = Join-Path $fixtureRoot 'source'
Expand-AtsumiPackageArchive $approved 'tools' $source $manifest
Assert-AtsumiPackageTree $source $manifest
Assert-Fixture ([IO.File]::ReadAllText((Join-Path $source 'LICENSE.txt')) -ceq 'test license') 'exact extraction and immutable-source verification'
$copy = Join-Path $fixtureRoot 'copy'
Copy-AtsumiPackageTree $source $copy $manifest
$roundtrip = Join-Path $fixtureRoot 'roundtrip.zip'
New-AtsumiPackageArchive $copy $roundtrip $manifest
Assert-AtsumiManifestEqual $manifest (Get-AtsumiArchiveManifest $roundtrip 'Atsumi')
Assert-Fixture $true 'manifest-selected copy and ZIP byte-proof roundtrip'
Assert-Rejected { Copy-AtsumiPackageTree $source $copy $manifest } 'existing destination rejected without replacement'
Assert-Fixture ([IO.File]::ReadAllText((Join-Path $copy 'LICENSE.txt')) -ceq 'test license') 'existing destination bytes preserved'

$sourceProof = Get-AtsumiFileProof (Join-Path $source 'LICENSE.txt')
$lease = Lock-AtsumiPackageDirectories @($source)
try { Assert-Rejected { [IO.Directory]::Move($source, (Join-Path $fixtureRoot 'renamed-source')) } 'held ancestor prevents rename during copy' }
finally { $lease.Dispose() }
Assert-Fixture ([IO.Directory]::Exists($source)) 'held source directory remains in place'
$regular = [Atsumi.PackageDirectoryLease]::OpenRegularRead((Join-Path $source 'LICENSE.txt'))
try { Assert-Rejected { [IO.File]::WriteAllText((Join-Path $source 'LICENSE.txt'),'mutation',$utf8) } 'held regular source prevents concurrent content mutation' }
finally { $regular.Dispose() }
$unframed = [IO.MemoryStream]::new($utf8.GetBytes('test license'),$false)
try { Copy-AtsumiVerifiedStream $unframed (Join-Path $fixtureRoot 'stream-response.txt') @{length=-1;sha256=$sourceProof.sha256} }
finally { $unframed.Dispose() }
Assert-Fixture ((Get-AtsumiFileProof (Join-Path $fixtureRoot 'stream-response.txt')).sha256 -ceq $sourceProof.sha256) 'unknown-length response is hash verified without buffering or overwrite'
$unframed = [IO.MemoryStream]::new($utf8.GetBytes('test license'),$false)
try { Assert-Rejected { Copy-AtsumiVerifiedStream $unframed (Join-Path $fixtureRoot 'stream-response.txt') @{length=-1;sha256=$sourceProof.sha256} } 'existing download response destination is never truncated' }
finally { $unframed.Dispose() }
$unframed = [IO.MemoryStream]::new($utf8.GetBytes('test licensX'),$false)
try { Assert-Rejected { Copy-AtsumiVerifiedStream $unframed (Join-Path $fixtureRoot 'bad-stream-response.txt') @{length=-1;sha256=$sourceProof.sha256} } 'unknown-length response with wrong hash fails before admission' }
finally { $unframed.Dispose() }

[IO.File]::WriteAllText((Join-Path $source 'LICENSE.txt'),'test licensX',$utf8)
Assert-Rejected { Assert-AtsumiPackageTree $source $manifest } 'same-size license mutation rejected'
Assert-Rejected { Copy-AtsumiVerifiedFile (Join-Path $source 'LICENSE.txt') (Join-Path $fixtureRoot 'changed-copy.txt') $sourceProof } 'changed source cannot pass copied-byte hash verification'
Assert-Fixture ([IO.File]::Exists((Join-Path $fixtureRoot 'changed-copy.txt'))) 'failed copy is retained, not deleted'
[IO.File]::Move((Join-Path $source 'LICENSE.txt'),(Join-Path $fixtureRoot 'retained-license.txt'))
Assert-Rejected { Assert-AtsumiPackageTree $source $manifest } 'missing license rejected without repair'
Assert-Fixture (-not [IO.File]::Exists((Join-Path $source 'LICENSE.txt'))) 'missing license not silently restored'
[IO.File]::WriteAllText((Join-Path $copy 'unexpected.txt'),'unknown',$utf8)
Assert-Rejected { Assert-AtsumiPackageTree $copy $manifest } 'unexpected non-bin file rejected'
Assert-Fixture ([IO.File]::ReadAllText((Join-Path $copy 'unexpected.txt')) -ceq 'unknown') 'unknown file preserved'

$withExtraDirectory = Join-Path $fixtureRoot 'extra-directory'
Expand-AtsumiPackageArchive $approved 'tools' $withExtraDirectory $manifest
New-AtsumiSafeDirectory (Join-Path $withExtraDirectory 'unexpected') -MustCreate
Assert-Rejected { Assert-AtsumiPackageTree $withExtraDirectory $manifest } 'unexpected empty directory rejected'
$withWrongType = Join-Path $fixtureRoot 'wrong-type'
Expand-AtsumiPackageArchive $approved 'tools' $withWrongType $manifest
[IO.File]::Move((Join-Path $withWrongType 'LICENSE.txt'),(Join-Path $fixtureRoot 'retained-type-license.txt'))
New-AtsumiSafeDirectory (Join-Path $withWrongType 'LICENSE.txt') -MustCreate
Assert-Rejected { Assert-AtsumiPackageTree $withWrongType $manifest } 'directory in place of a file rejected'

$unsafeArchives = @(
  @(@{path='tools/../escape.txt';text='x'}),@(@{path='/tools/file';text='x'}),
  @(@{path='tools/file:stream';text='x'}),@(@{path='tools/CON.txt';text='x'}),
  @(@{path='tools/file.';text='x'}),@(@{path='tools//file';text='x'}),
  @(@{path='tools/a';text='x'},@{path='tools/a';text='y'}),
  @(@{path='tools/A';text='x'},@{path='tools/a';text='y'}),
  @(@{path='tools/doc/a';text='x'},@{path='tools/DOC/b';text='y'}),
  @(@{path='tools/file';text='x'},@{path='tools/file/nested';text='y'}),
  @(@{path='tools/link';text='elsewhere';attributes=-1610612736}),
  @(@{path='tools/reparse';text='x';attributes=0x400}),
  @(@{path='tools/';text='nonempty root directory'})
)
$index=0
foreach ($entries in $unsafeArchives) {
  $index++
  $bad = New-FixtureArchive "unsafe-$index" $entries
  Assert-Rejected { Get-AtsumiArchiveManifest $bad 'tools' } "unsafe archive path/type/collision $index"
}
$junction = Join-Path $fixtureRoot 'linked-source'
New-Item -ItemType Junction -Path $junction -Target $withExtraDirectory | Out-Null
Assert-Rejected { Get-AtsumiDirectoryManifest $junction } 'directory junction source rejected'
Assert-Rejected { Copy-AtsumiVerifiedFile (Join-Path $junction 'LICENSE.txt') (Join-Path $fixtureRoot 'linked-copy.txt') (Get-AtsumiFileProof (Join-Path $withExtraDirectory 'LICENSE.txt')) } 'junction ancestor rejected before copy'
Assert-Fixture (-not [IO.File]::Exists((Join-Path $fixtureRoot 'linked-copy.txt'))) 'linked copy creates no output'
$hardlinkSource = Join-Path $fixtureRoot 'hardlink-source.txt'
$null = Write-AtsumiPackageBytes $hardlinkSource ($utf8.GetBytes('preserved hardlink fixture'))
$hardlinkProof = Get-AtsumiFileProof $hardlinkSource
$hardlinkAlias = Join-Path $fixtureRoot 'hardlink-alias.txt'
New-Item -ItemType HardLink -Path $hardlinkAlias -Target $hardlinkSource | Out-Null
Assert-Rejected { Get-AtsumiFileProof $hardlinkSource } 'hardlinked regular source proof rejected by actual handle link count'
Assert-Rejected { Copy-AtsumiVerifiedFile $hardlinkAlias (Join-Path $fixtureRoot 'hardlink-copy.txt') $hardlinkProof } 'hardlinked regular source copy rejected before destination creation'
Assert-Fixture (([IO.File]::ReadAllText($hardlinkSource) -ceq 'preserved hardlink fixture') -and ([IO.File]::ReadAllText($hardlinkAlias) -ceq 'preserved hardlink fixture')) 'both hardlink source names and bytes preserved'
Assert-Fixture (-not [IO.File]::Exists((Join-Path $fixtureRoot 'hardlink-copy.txt'))) 'hardlink rejection creates no output'

$standard = Get-AtsumiPortableArtifact '9.8.7' $false
$initial = Get-AtsumiPortableArtifact '9.8.7' $true
Assert-Fixture ($standard.name -cne $initial.name -and $standard.directory -cne $initial.directory -and $standard.updaterCompatible -and -not $initial.updaterCompatible) 'runtime-inclusive artifact has distinct name, directory and metadata'
$payload = Join-Path $fixtureRoot 'updater-payload'
New-AtsumiSafeDirectory (Join-Path $payload 'media-tools')
foreach ($name in @('atsumi.exe','Start-Atsumi.cmd','Readme.txt')) { $null=Write-AtsumiPackageBytes (Join-Path $payload $name) ($utf8.GetBytes('synthetic never executed')) }
$marker='{"format":1,"distribution":"portable","version":"9.8.7","packageKind":"portable-updater","updaterCompatible":true}'
$null=Write-AtsumiPackageBytes (Join-Path $payload 'atsumi-portable.json') ($utf8.GetBytes($marker))
$updaterManifest = Get-AtsumiDirectoryManifest $payload
$updater = Join-Path $fixtureRoot 'Atsumi-Portable.zip'
New-AtsumiPackageArchive $payload $updater $updaterManifest
Write-FixtureMetadata $updater $updaterManifest
Assert-AtsumiUpdaterArchive $updater '9.8.7'
Assert-Fixture $true 'standard updater archive metadata and marker accepted'
$releaseProof = Assert-AtsumiUpdaterArchive $updater '9.8.7' -PassThru
Assert-Fixture ($releaseProof.sha256 -ceq (Get-AtsumiFileProof $updater).sha256 -and $releaseProof.updaterCompatible) 'release receives the exact protected archive proof for staging bytes'
Assert-Rejected { Assert-AtsumiUpdaterArchive $updater '9.8.8' } 'wrong version rejected'
New-AtsumiSafeDirectory (Join-Path $payload 'WebView2')
$null=Write-AtsumiPackageBytes (Join-Path $payload 'WebView2/msedgewebview2.exe') ($utf8.GetBytes('synthetic never executed'))
$runtimeManifest = Get-AtsumiDirectoryManifest $payload
$initialZip = Join-Path $fixtureRoot $initial.name
New-AtsumiPackageArchive $payload $initialZip $runtimeManifest
Write-FixtureMetadata $initialZip $runtimeManifest $initial.kind $true
Assert-Rejected { Assert-AtsumiUpdaterArchive $initialZip '9.8.7' } 'runtime-inclusive initial artifact rejected as updater'
$renamedRoot=Join-Path $fixtureRoot 'renamed-runtime'
New-AtsumiSafeDirectory $renamedRoot
$renamed=Join-Path $renamedRoot 'Atsumi-Portable.zip'
[IO.File]::Copy($initialZip,$renamed,$false)
Write-FixtureMetadata $renamed $runtimeManifest
Assert-Rejected { Assert-AtsumiUpdaterArchive $renamed '9.8.7' } 'renamed runtime ZIP rejected even with misleading sidecar'
$absentMetadata=Join-Path $fixtureRoot 'no-metadata'
New-AtsumiSafeDirectory $absentMetadata
[IO.File]::Copy($updater,(Join-Path $absentMetadata 'Atsumi-Portable.zip'),$false)
Assert-Rejected { Assert-AtsumiUpdaterArchive (Join-Path $absentMetadata 'Atsumi-Portable.zip') '9.8.7' } 'missing sidecar rejected'
[IO.File]::AppendAllText($updater,'tamper',$utf8)
Assert-Rejected { Assert-AtsumiUpdaterArchive $updater '9.8.7' } 'archive mutation rejected by metadata proof'

[pscustomobject]@{passed=$script:checks.Count;checks=$script:checks;fixture=$fixtureRoot;syntheticOnly=$true;downloads=0;toolsExecuted=0;deletedFiles=0} | ConvertTo-Json -Depth 4 -Compress

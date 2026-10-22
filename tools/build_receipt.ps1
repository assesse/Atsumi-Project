param([string]$VerifyReceipt, [string]$PortableArchive)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'package_manifest.ps1')

# These are build inputs, not a recursive snapshot of the workspace or process.
function Get-AtsumiBuildSourceManifest([string]$Root) {
  $rootPath = Get-AtsumiPackageDirectoryPath $Root
  # Vite consumes root .env files implicitly. Never hash/export them or silently
  # omit a build input: require a reviewed, non-secret source configuration.
  if (@(Get-ChildItem -LiteralPath $rootPath -Force -File | Where-Object { $_.Name -match '^\.env(?:\.|$)' }).Count) { throw 'Implicit environment files are not accepted for a provenance release build.' }
  $trees = @('src','public','shared','tools','src-tauri/src','src-tauri/vendor','src-tauri/icons','src-tauri/capabilities','.github/workflows','docs/releases')
  $single = @('package.json','pnpm-lock.yaml','pnpm-workspace.yaml','.npmrc','index.html','vite.config.ts','tsconfig.json','tsconfig.app.json','tsconfig.node.json','src-tauri/Cargo.toml','src-tauri/Cargo.lock','src-tauri/build.rs','src-tauri/tauri.conf.json','THIRD_PARTY_NOTICES.md')
  $optionalTrees = @('.cargo','src-tauri/.cargo')
  $optionalFiles = @('rust-toolchain','rust-toolchain.toml','src-tauri/rust-toolchain','src-tauri/rust-toolchain.toml','src-tauri/tauri.windows.conf.json','src-tauri/tauri.windows.conf.json5')
  foreach ($config in Get-ChildItem -LiteralPath $rootPath -File -Force) {
    if ($config.Name -match '^tsconfig.*\.json$' -and $config.Name -notin $single) { $optionalFiles += $config.Name }
    if ($config.Name -match '^vite\.config\.' -and $config.Name -cne 'vite.config.ts') { throw 'Unexpected frontend build configuration; source inventory requires review.' }
  }
  $files = [Collections.Generic.List[object]]::new()
  $directories = [Collections.Generic.List[string]]::new()
  # This reviewed Rust module is source code, not a browser/user profile. Keep
  # the exception exact: new children require review instead of being hashed.
  $profileSource = 'src-tauri/src/platform_media/profile'
  $profileSourceFiles = @(($profileSource+'/current.rs'), ($profileSource+'/snapshot.rs'), ($profileSource+'/snapshot/tests.rs'))
  $profileSourceDirectories = @($profileSource, ($profileSource+'/snapshot'))
  foreach ($tree in $trees + $optionalTrees) {
    $path = Join-Path $rootPath $tree
    if (-not (Test-Path -LiteralPath $path)) {
      if ($tree -in $trees) { throw 'Required build input directory is missing.' }
      continue
    }
    # Reject sensitive names before any file bytes are hashed. Walk without
    # following links; do not traverse a broad runtime/profile directory.
    $pending = [Collections.Generic.Queue[string]]::new()
    $pending.Enqueue($path)
    $directories.Add($tree)
    while ($pending.Count) {
      $directory = $pending.Dequeue()
      Assert-AtsumiPackagePath $directory
      foreach ($entry in Get-ChildItem -LiteralPath $directory -Force) {
        Assert-AtsumiPackagePath $entry.FullName
        $relative = ConvertTo-AtsumiPackagePath $entry.FullName.Substring($rootPath.Length+1)
        if ($relative -match '(^|/)\.env(?:\.|$)|\.(?:key|pem|pfx|p12)$') { throw 'Sensitive-named file in build inputs; review required, contents not recorded.' }
        if ($relative.StartsWith($profileSource+'/', [StringComparison]::Ordinal)) {
          if (($entry.PSIsContainer -and $relative -cnotin $profileSourceDirectories) -or (-not $entry.PSIsContainer -and $relative -cnotin $profileSourceFiles)) { throw 'Unreviewed entry in source profile module; contents not recorded.' }
        }
        if ($entry.PSIsContainer -and $entry.Name -match '^(?:secrets?|profiles?|cookies?)$') {
          if ($relative -cne $profileSource) { throw 'Sensitive-named file in build inputs; review required, contents not recorded.' }
        }
        if ($entry.PSIsContainer -and $relative -cin $profileSourceDirectories) {
          $expectedMembers = if ($relative -ceq $profileSource) { @('current.rs','snapshot.rs','snapshot') } else { @('tests.rs') }
          $members = @(Get-ChildItem -LiteralPath $entry.FullName -Force)
          if ($members.Count -ne @($expectedMembers).Count) { throw 'Source profile module inventory changed; review required.' }
          foreach ($member in $members) {
            Assert-AtsumiPackagePath $member.FullName
            $memberRelative = $relative+'/'+$member.Name
            if ($member.Name -cnotin $expectedMembers -or ($member.PSIsContainer -and $memberRelative -cnotin $profileSourceDirectories) -or (-not $member.PSIsContainer -and $memberRelative -cnotin $profileSourceFiles)) { throw 'Unreviewed entry in source profile module; contents not recorded.' }
          }
        }
        if ($entry.PSIsContainer) { $directories.Add($relative); $pending.Enqueue($entry.FullName) }
        else { $proof=Get-AtsumiFileProof $entry.FullName; $files.Add([pscustomobject]@{path=$relative;length=$proof.length;sha256=$proof.sha256}) }
        if ($files.Count+$directories.Count -gt 4095) { throw 'Build input manifest exceeds entry bound.' }
      }
    }
  }
  foreach ($name in $single + $optionalFiles) {
    $path = Join-Path $rootPath $name
    if (-not (Test-Path -LiteralPath $path)) {
      if ($name -in $single) { throw 'Required build input file is missing.' }
      continue
    }
    $proof = Get-AtsumiFileProof $path
    $files.Add([pscustomobject]@{path=$name;length=$proof.length;sha256=$proof.sha256})
  }
  return New-AtsumiPackageManifest $files.ToArray() $directories.ToArray()
}

function Read-AtsumiReceiptJson([string]$Path,[switch]$WithProof) {
  $lease = Lock-AtsumiPackageDirectories @($Path)
  try {
    $stream = [Atsumi.PackageDirectoryLease]::OpenRegularRead([IO.Path]::GetFullPath($Path))
    try {
      if ($stream.Length -gt 8MB) { throw 'Build receipt exceeds its bound.' }
      $reader = [IO.StreamReader]::new($stream,[Text.UTF8Encoding]::new($false,$true),$true,4096,$true)
      try { $value=$reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
      if (-not $WithProof) { return $value }
      $stream.Position=0
      return [pscustomobject]@{value=$value;proof=(Get-AtsumiStreamProof $stream $stream.Length)}
    } finally { $stream.Dispose() }
  } finally { $lease.Dispose() }
}

function Get-AtsumiReleaseVersion([string]$Root) {
  $config = Read-AtsumiReceiptJson (Join-Path $Root 'src-tauri/tauri.conf.json')
  $package = Read-AtsumiReceiptJson (Join-Path $Root 'package.json')
  $cargo = [IO.File]::ReadAllText((Join-Path $Root 'src-tauri/Cargo.toml'))
  $match = [regex]::Match($cargo,'(?m)^version = "([^"]+)"\r?$')
  $version = [string]$config.version
  if ($version -notmatch '^\d+\.\d+\.\d+$' -or $package.version -cne $version -or -not $match.Success -or $match.Groups[1].Value -cne $version) { throw 'Release source versions are absent or inconsistent.' }
  return $version
}

function Get-AtsumiBuildResources([string]$Root) {
  $media = & (Join-Path $Root 'tools/prepare_media_tools.ps1') -VerifyOnly -ReturnManifest
  $platform = & (Join-Path $Root 'tools/prepare_platform_tools.ps1') -VerifyOnly -ReturnManifest
  $config = Read-AtsumiReceiptJson (Join-Path $Root 'src-tauri/tauri.conf.json')
  $approved = @{}
  $approved[(Get-AtsumiPackageDirectoryPath $media.package)] = $media.manifest
  $approved[(Get-AtsumiPackageDirectoryPath $platform.package)] = $platform.manifest
  $result = @()
  foreach ($mapping in $config.bundle.resources.PSObject.Properties) {
    $source = Get-AtsumiPackageDirectoryPath (Join-Path (Join-Path $Root 'src-tauri') $mapping.Name)
    if (-not $approved.ContainsKey($source)) { throw 'Build resource has no approved manifest.' }
    $relative = $source.Substring((Get-AtsumiPackageDirectoryPath $Root).Length+1).Replace('\','/')
    $target = ConvertTo-AtsumiPackagePath (([string]$mapping.Value).TrimEnd('/'))
    $result += [pscustomobject]@{path=$relative;target=$target;manifest=$approved[$source]}
  }
  if ($result.Count -ne 2 -or @($result.path|Sort-Object -Unique).Count -ne 2) { throw 'Build requires the two approved resource mappings.' }
  return @($result | Sort-Object path)
}

function Get-AtsumiBuildInputs([string]$Root, [scriptblock]$ResourceProvider) {
  $resources = if ($ResourceProvider) { @(& $ResourceProvider $Root) } else { @(Get-AtsumiBuildResources $Root) }
  return [pscustomobject]@{format=1;version=(Get-AtsumiReleaseVersion $Root);source=(Get-AtsumiBuildSourceManifest $Root);resources=$resources}
}

function Get-AtsumiReceiptDigest($Value) {
  $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 16 -Compress))
  $sha = [Security.Cryptography.SHA256]::Create()
  try { return [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','').ToLowerInvariant() }
  finally { $sha.Dispose() }
}

function Assert-AtsumiBuildInputsEqual($Expected, $Actual) {
  if ((Get-AtsumiReceiptDigest $Expected) -cne (Get-AtsumiReceiptDigest $Actual)) {
    # Only bounded, validated relative paths and change categories, never source bytes or
    # environment values. Keep equality byte-exact, including line endings.
    $before = @{}; $after = @{}
    foreach ($file in $Expected.source.files) { $before[$file.path] = $file }
    foreach ($file in $Actual.source.files) { $after[$file.path] = $file }
    $changes = [Collections.Generic.List[string]]::new()
    foreach ($name in @(@($before.Keys) + @($after.Keys) | Sort-Object -Unique)) {
      $displayName = 'invalid source path'
      if ($name -is [string] -and $name.Length -le 256) {
        try { $displayName = ConvertTo-AtsumiPackagePath $name }
        catch { $displayName = 'invalid source path' }
      }
      if (-not $before.ContainsKey($name)) { $changes.Add('added: '+$displayName) }
      elseif (-not $after.ContainsKey($name)) { $changes.Add('removed: '+$displayName) }
      elseif ((Get-AtsumiReceiptDigest $before[$name]) -cne (Get-AtsumiReceiptDigest $after[$name])) { $changes.Add('changed: '+$displayName) }
    }
    if ((Get-AtsumiReceiptDigest $Expected.source.directories) -cne (Get-AtsumiReceiptDigest $Actual.source.directories)) { $changes.Add('source directories changed') }
    if ((Get-AtsumiReceiptDigest $Expected.resources) -cne (Get-AtsumiReceiptDigest $Actual.resources)) { $changes.Add('approved resources changed') }
    if ($Expected.version -cne $Actual.version) { $changes.Add('version changed') }
    $details = (@($changes | Select-Object -First 16) -join '; ')
    throw ('Build inputs changed; no release receipt can be accepted. '+$changes.Count+' differences: '+$details)
  }
}

function Initialize-AtsumiOwnedBuildHandle {
  if ('Atsumi.OwnedCargoOutput' -as [type]) { return }
  Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
namespace Atsumi {
  public sealed class OwnedCargoOutput : IDisposable {
    [StructLayout(LayoutKind.Sequential)] struct Info { public uint Attributes,C0,C1,A0,A1,W0,W1,Volume,SizeHigh,SizeLow,Links,IndexHigh,IndexLow; }
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern SafeFileHandle CreateFileW(string p,uint a,uint s,IntPtr sec,uint d,uint f,IntPtr t);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetFileInformationByHandle(SafeFileHandle h,out Info i);
    SafeFileHandle alias;
    public FileStream Stream {get;private set;}
    uint expectedLinks;
    static Info Read(SafeFileHandle h) { Info i; if(h.IsInvalid || !GetFileInformationByHandle(h,out i) || (i.Attributes & (0x400|0x10))!=0) throw new IOException("Invalid owned build file handle."); return i; }
    static SafeFileHandle OpenFile(string p) { var h=CreateFileW(p,0x80000000,1,IntPtr.Zero,3,0x00200000,IntPtr.Zero); if(h.IsInvalid){h.Dispose();throw new Win32Exception(Marshal.GetLastWin32Error(),"Cannot hold owned build output.");} return h; }
    static bool Same(Info a,Info b) {return a.Volume==b.Volume && a.IndexHigh==b.IndexHigh && a.IndexLow==b.IndexLow;}
    public static OwnedCargoOutput Open(string attempt) {
      var output=new OwnedCargoOutput(); SafeFileHandle primary=null;
      try {
        var release=Path.Combine(attempt,"target","x86_64-pc-windows-msvc","release");
        primary=OpenFile(Path.Combine(release,"atsumi.exe")); var first=Read(primary);
        if(first.Links!=1 && first.Links!=2) throw new IOException("Unexpected Cargo output link count.");
        var peer=Path.Combine(release,"deps","atsumi.exe");
        if(first.Links==2){output.alias=OpenFile(peer); var second=Read(output.alias); if(second.Links!=2 || !Same(first,second)) throw new IOException("Cargo output alias identity mismatch.");}
        else if(File.Exists(peer)) throw new IOException("Unexpected distinct Cargo output alias.");
        output.expectedLinks=first.Links; output.Stream=new FileStream(primary,FileAccess.Read,65536,false);primary=null;output.Validate();return output;
      } catch {if(primary!=null)primary.Dispose();output.Dispose();throw;}
    }
    public void Validate(){var first=Read(Stream.SafeFileHandle);if(first.Links!=expectedLinks)throw new IOException("Cargo output links changed.");if(alias!=null){var second=Read(alias);if(second.Links!=expectedLinks || !Same(first,second))throw new IOException("Cargo output identity changed.");}}
    public void Dispose(){if(Stream!=null){Stream.Dispose();Stream=null;}if(alias!=null){alias.Dispose();alias=null;}}
  }
}
'@
}

function Assert-AtsumiBuildMetadata($Metadata) {
  $expected = @('rustc','cargo','node','pnpm','windowsSdk','gitCommit')
  $names = @($Metadata.PSObject.Properties.Name | Sort-Object)
  if (($names -join ',') -cne (($expected|Sort-Object)-join ',')) { throw 'Unexpected build metadata fields.' }
  foreach ($key in $expected) {
    $value = $Metadata.$key
    if ($value -isnot [string] -or -not $value -or $value.Length -gt 512 -or $value -match '[\x00-\x08\x0b\x0c\x0e-\x1f]') { throw 'Invalid bounded compiler version metadata.' }
  }
  if ($Metadata.gitCommit -cnotmatch '^[a-f0-9]{40}$') { throw 'A resolved source commit is required alongside dirty-input hashes.' }
}

# Callback injection is confined to this library for tiny unit fixtures. The
# production CLI below never accepts a callback, output path or success flag.
function Invoke-AtsumiBuildReceipt([string]$Root, [scriptblock]$Build, [scriptblock]$Versions, [scriptblock]$ResourceProvider, [scriptblock]$ExecutableVersion) {
  $rootPath = Get-AtsumiPackageDirectoryPath $Root
  $id = [Guid]::NewGuid().ToString('N')
  $attempt = Join-Path $rootPath ('.runtime/release-builds/'+$id)
  $stage = Join-Path $rootPath ('.runtime/release-inputs/'+$id)
  $inputs = Get-AtsumiBuildInputs $rootPath $ResourceProvider
  $metadata = & $Versions
  Assert-AtsumiBuildMetadata $metadata
  New-AtsumiSafeDirectory $attempt -MustCreate
  $attemptLease = Lock-AtsumiPackageDirectories @($attempt)
  try {
    $exitCode = & $Build $attempt $inputs.version
    if ($exitCode -isnot [int] -or $exitCode -ne 0) { throw 'Release build failed; no success receipt was written.' }
    Assert-AtsumiBuildInputsEqual $inputs (Get-AtsumiBuildInputs $rootPath $ResourceProvider)
    $release = Join-Path $attempt 'target/x86_64-pc-windows-msvc/release'
    $exe = Join-Path $release 'atsumi.exe'
    $installer = Join-Path $release ('bundle/nsis/Atsumi_'+$inputs.version+'_x64-setup.exe')
    $sourceLease = Lock-AtsumiPackageDirectories @($exe,(Join-Path $release 'deps/atsumi.exe'),$installer,($installer+'.sig'))
    try {
      Initialize-AtsumiOwnedBuildHandle
      $owned = [Atsumi.OwnedCargoOutput]::Open($attempt)
      try {
        $actualVersion = if ($ExecutableVersion) { & $ExecutableVersion $exe } else { (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion }
        if (-not $actualVersion -or $actualVersion -cne $inputs.version) { throw 'Built executable ProductVersion is absent or mismatched.' }
        $exeProof = Get-AtsumiStreamProof $owned.Stream $owned.Stream.Length
        $owned.Stream.Position=0
        New-AtsumiSafeDirectory $stage -MustCreate
        Copy-AtsumiVerifiedStream $owned.Stream (Join-Path $stage 'atsumi.exe') $exeProof
        $owned.Validate()
      } finally { $owned.Dispose() }
      $outputs = @([pscustomobject]@{path='atsumi.exe';length=$exeProof.length;sha256=$exeProof.sha256})
      foreach ($item in @(@{source=$installer;path='Atsumi-Setup.exe'},@{source=($installer+'.sig');path='Atsumi-Setup.exe.sig'})) {
        $proof = Get-AtsumiFileProof $item.source
        if ($proof.length -eq 0 -or ($item.path.EndsWith('.sig') -and $proof.length -gt 16KB)) { throw 'Empty or oversized build output.' }
        Copy-AtsumiVerifiedFile $item.source (Join-Path $stage $item.path) $proof
        $outputs += [pscustomobject]@{path=$item.path;length=$proof.length;sha256=$proof.sha256}
      }
    } finally { $sourceLease.Dispose() }
    Assert-AtsumiBuildInputsEqual $inputs (Get-AtsumiBuildInputs $rootPath $ResourceProvider)
    $outputManifest = New-AtsumiPackageManifest $outputs
    Assert-AtsumiPackageTree $stage $outputManifest
    $receipt = [pscustomobject][ordered]@{format=1;kind='atsumi-successful-build';buildId=$id;version=$inputs.version;target='x86_64-pc-windows-msvc';buildArguments=@('build','--target','x86_64-pc-windows-msvc','--bundles','nsis','--','--locked');completed=$true;metadata=$metadata;inputs=$inputs;inputSha256=(Get-AtsumiReceiptDigest $inputs);outputs=$outputManifest}
    $path = Join-Path $stage 'build-receipt.json'
    $null = Write-AtsumiPackageBytes $path ([Text.UTF8Encoding]::new($false).GetBytes(($receipt|ConvertTo-Json -Depth 16)))
    return $path
  } finally { $attemptLease.Dispose() }
}

function Assert-AtsumiBuildReceipt([string]$Root,[string]$ReceiptPath,[scriptblock]$ResourceProvider) {
  if (-not $ReceiptPath) { throw 'An explicit successful-build receipt is required.' }
  $rootPath = Get-AtsumiPackageDirectoryPath $Root
  $document = Read-AtsumiReceiptJson $ReceiptPath -WithProof
  $receipt = $document.value
  $keys=@('format','kind','buildId','version','target','buildArguments','completed','metadata','inputs','inputSha256','outputs')|Sort-Object
  if ((@($receipt.PSObject.Properties.Name|Sort-Object)-join ',') -cne ($keys-join ',')) { throw 'Unexpected receipt fields.' }
  if ($receipt.format -ne 1 -or $receipt.kind -cne 'atsumi-successful-build' -or $receipt.completed -isnot [bool] -or -not $receipt.completed -or $receipt.buildId -cnotmatch '^[0-9a-f]{32}$' -or $receipt.target -cne 'x86_64-pc-windows-msvc' -or ($receipt.buildArguments -join ' ') -cne 'build --target x86_64-pc-windows-msvc --bundles nsis -- --locked') { throw 'Invalid successful-build receipt.' }
  $stage = Join-Path $rootPath ('.runtime/release-inputs/'+$receipt.buildId)
  if ([IO.Path]::GetFullPath($ReceiptPath) -cne (Join-Path $stage 'build-receipt.json')) { throw 'Receipt is outside its owned capture stage.' }
  Assert-AtsumiBuildMetadata $receipt.metadata
  $current = Get-AtsumiBuildInputs $rootPath $ResourceProvider
  if ($receipt.version -cne $current.version -or $receipt.inputSha256 -cne (Get-AtsumiReceiptDigest $receipt.inputs)) { throw 'Receipt version or input digest mismatch.' }
  Assert-AtsumiBuildInputsEqual $receipt.inputs $current
  $outputNames = (@('Atsumi-Setup.exe','Atsumi-Setup.exe.sig','atsumi.exe')|Sort-Object) -join ','
  if (@($receipt.outputs.directories).Count -ne 0 -or (@($receipt.outputs.files.path|Sort-Object)-join ',') -cne $outputNames) { throw 'Unexpected captured output manifest.' }
  $proof = Get-AtsumiFileProof $ReceiptPath
  if ($proof.sha256 -cne $document.proof.sha256 -or $proof.length -ne $document.proof.length) { throw 'Build receipt changed after parsing.' }
  $expected = New-AtsumiPackageManifest (@($receipt.outputs.files)+@([pscustomobject]@{path='build-receipt.json';length=$proof.length;sha256=$proof.sha256}))
  Assert-AtsumiPackageTree $stage $expected
  return [pscustomobject]@{version=$receipt.version;buildId=$receipt.buildId;receiptSha256=$proof.sha256;receiptBytes=$proof.length;receiptPath=(Join-Path $stage 'build-receipt.json');stage=$stage;outputs=$receipt.outputs;inputs=$receipt.inputs}
}

function Assert-AtsumiPortableBuildBinding([string]$Archive,$BuildProof) {
  $metadataPath=$Archive+'.metadata.json'
  $lease=Lock-AtsumiPackageDirectories @($Archive,$metadataPath)
  $held=$null
  try {
  $held=[Atsumi.PackageDirectoryLease]::OpenRegularRead([IO.Path]::GetFullPath($metadataPath))
  Assert-AtsumiUpdaterArchive $Archive $BuildProof.version
  $metadata = Read-AtsumiReceiptJson $metadataPath
  if ($metadata.buildId -cne $BuildProof.buildId -or $metadata.buildReceiptSha256 -cne $BuildProof.receiptSha256) { throw 'Portable package is not bound to this successful build.' }
  $exe = @($metadata.manifest.files|Where-Object{$_.path -ceq 'atsumi.exe'})
  $expected = @($BuildProof.outputs.files|Where-Object{$_.path -ceq 'atsumi.exe'})
  if ($exe.Count -ne 1 -or $exe[0].length -ne $expected[0].length -or $exe[0].sha256 -cne $expected[0].sha256) { throw 'Portable executable differs from the successful build.' }
  } finally { if($held){$held.Dispose()};$lease.Dispose() }
}

if ($VerifyReceipt) {
  $result = Assert-AtsumiBuildReceipt (Split-Path -Parent $PSScriptRoot) $VerifyReceipt
  if ($PortableArchive) { Assert-AtsumiPortableBuildBinding $PortableArchive $result }
  $result | Select-Object version,buildId,receiptSha256,receiptBytes,receiptPath,stage,outputs | ConvertTo-Json -Depth 8 -Compress
}

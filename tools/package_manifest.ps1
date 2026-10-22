param([string]$VerifyPortableArchive, [string]$ExpectedVersion)

# Pure local verification/copy helpers. No downloads, process execution or cleanup.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.IO.Compression

function Lock-AtsumiPackageDirectories([string[]]$Paths) {
  # Keep every existing ancestor named in the operation from being renamed into
  # a junction between path validation and create-new/copy. No FILE_SHARE_DELETE.
  if (-not ('Atsumi.PackageDirectoryLease' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
namespace Atsumi {
  public sealed class PackageDirectoryLease : IDisposable {
    readonly List<SafeFileHandle> handles = new List<SafeFileHandle>();
    [StructLayout(LayoutKind.Sequential)] struct FileInfo {
      public uint Attributes, C0, C1, A0, A1, W0, W1, Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFileW(string p, uint access, uint share, IntPtr sec, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool GetFileInformationByHandle(SafeFileHandle handle, out FileInfo info);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool CreateDirectoryW(string path, IntPtr sec);
    public static void CreateNewDirectory(string path) {
      if (!CreateDirectoryW(path, IntPtr.Zero)) throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot create a new package directory; existing path preserved.");
    }
    public static FileStream OpenRegularRead(string path) {
      var handle = CreateFileW(path, 0x80000000, 1, IntPtr.Zero, 3, 0x00200000, IntPtr.Zero);
      try {
        if (handle.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot hold regular package file.");
        FileInfo info;
        if (!GetFileInformationByHandle(handle, out info) || (info.Attributes & (0x400|0x10)) != 0 || info.Links != 1) throw new IOException("Linked or non-regular package file rejected before reading.");
        return new FileStream(handle, FileAccess.Read, 65536, false);
      } catch { handle.Dispose(); throw; }
    }
    public static PackageDirectoryLease Acquire(string[] paths) {
      var lease = new PackageDirectoryLease();
      var names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
      try {
        foreach (var value in paths) {
          for (var path = Path.GetFullPath(value); path != null; path = Path.GetDirectoryName(path)) {
            FileAttributes attributes;
            try { attributes = File.GetAttributes(path); }
            catch (FileNotFoundException) { continue; }
            catch (DirectoryNotFoundException) { continue; }
            if ((attributes & FileAttributes.ReparsePoint) != 0) throw new IOException("Linked package ancestor rejected.");
            if ((attributes & FileAttributes.Directory) != 0) names.Add(path);
          }
        }
        var ordered = new List<string>(names);
        ordered.Sort((left,right) => left.Length.CompareTo(right.Length));
        foreach (var path in ordered) {
          var handle = CreateFileW(path, 0x80000000, 3, IntPtr.Zero, 3, 0x02200000, IntPtr.Zero);
          if (handle.IsInvalid) { handle.Dispose(); throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot hold package ancestor."); }
          lease.handles.Add(handle);
          FileInfo info;
          if (!GetFileInformationByHandle(handle, out info) || (info.Attributes & 0x400) != 0 || (info.Attributes & 0x10) == 0) throw new IOException("Invalid package directory handle.");
        }
        return lease;
      } catch { lease.Dispose(); throw; }
    }
    public void Dispose() { for (int i=handles.Count-1;i>=0;i--) handles[i].Dispose(); handles.Clear(); }
  }
}
'@
  }
  return [Atsumi.PackageDirectoryLease]::Acquire($Paths)
}

function New-AtsumiSafeDirectory([string]$Path, [switch]$MustCreate) {
  $full = [IO.Path]::GetFullPath($Path)
  Assert-AtsumiPackagePath $full -AllowMissing
  if ([IO.Directory]::Exists($full)) {
    if ($MustCreate) { throw 'Package directory already exists; preserved.' }
    return
  }
  $parent = [IO.Path]::GetDirectoryName($full)
  if (-not $parent) { throw 'Cannot create a filesystem root.' }
  New-AtsumiSafeDirectory $parent
  $lease = Lock-AtsumiPackageDirectories @($parent)
  try { [Atsumi.PackageDirectoryLease]::CreateNewDirectory($full); Assert-AtsumiPackagePath $full }
  finally { $lease.Dispose() }
}

function ConvertTo-AtsumiPackagePath([string]$Path) {
  $normalized = $Path.Replace('\', '/')
  if (-not $normalized -or $normalized.StartsWith('/') -or $normalized.EndsWith('/')) { throw 'Invalid package path.' }
  foreach ($part in $normalized.Split('/')) {
    if (-not $part -or $part -in @('.', '..') -or $part -match '[\x00-\x1f<>:"|?*]' -or $part.EndsWith('.') -or $part.EndsWith(' ') -or $part.Split('.')[0] -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$') { throw 'Unsafe package path.' }
  }
  return $normalized
}

function Get-AtsumiPackageDirectoryPath([string]$Path) {
  $full = [IO.Path]::GetFullPath($Path)
  $root = [IO.Path]::GetPathRoot($full)
  if ($full.Length -eq $root.Length) { return $root }
  return $full.TrimEnd([char[]]'\/')
}

function Assert-AtsumiPackagePath([string]$Path, [switch]$AllowMissing) {
  $candidate = [IO.Path]::GetFullPath($Path)
  $first = $true
  while ($candidate) {
    try { $attributes = [IO.File]::GetAttributes($candidate) }
    catch {
      $code = $_.Exception.GetBaseException().HResult -band 0xffff
      if ($code -notin @(2,3) -or ($first -and -not $AllowMissing)) { throw }
      $attributes = $null
    }
    if ($null -ne $attributes -and ($attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Package path contains a link or reparse point.' }
    $first = $false
    $candidate = [IO.Path]::GetDirectoryName($candidate)
  }
}

function Get-AtsumiStreamProof($Stream, [long]$ExpectedLength = -1) {
  $sha = [Security.Cryptography.SHA256]::Create()
  $buffer = New-Object byte[] 65536
  [long]$length = 0
  try {
    while (($read = $Stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
      $length += $read
      if ($length -gt 1GB -or ($ExpectedLength -ge 0 -and $length -gt $ExpectedLength)) { throw 'Package file exceeds declared length.' }
      [void]$sha.TransformBlock($buffer, 0, $read, $buffer, 0)
    }
    [void]$sha.TransformFinalBlock($buffer, 0, 0)
    if ($ExpectedLength -ge 0 -and $length -ne $ExpectedLength) { throw 'Package file length changed.' }
    return [pscustomobject]@{ length=$length; sha256=[BitConverter]::ToString($sha.Hash).Replace('-', '').ToLowerInvariant() }
  } finally { $sha.Dispose() }
}

function Get-AtsumiFileProof([string]$Path) {
  $lease = Lock-AtsumiPackageDirectories @($Path)
  try {
  Assert-AtsumiPackagePath $Path
  if (([IO.File]::GetAttributes($Path) -band [IO.FileAttributes]::Directory)) { throw 'Expected a regular package file.' }
  $stream = [Atsumi.PackageDirectoryLease]::OpenRegularRead([IO.Path]::GetFullPath($Path))
  try { $proof = Get-AtsumiStreamProof $stream $stream.Length; Assert-AtsumiPackagePath $Path; return $proof }
  finally { $stream.Dispose() }
  } finally { $lease.Dispose() }
}

function New-AtsumiPackageManifest([object[]]$Files, [string[]]$Directories = @()) {
  $names = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
  $directoryNames = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
  $output = [Collections.Generic.List[object]]::new()
  foreach ($directory in $Directories) {
    $path = ConvertTo-AtsumiPackagePath $directory
    if ($directoryNames.ContainsKey($path) -and $directoryNames[$path] -cne $path) { throw 'Case-colliding package directories.' }
    $directoryNames[$path] = $path
  }
  foreach ($file in $Files) {
    $path = ConvertTo-AtsumiPackagePath ([string]$file.path)
    if ($names.ContainsKey($path)) { throw 'Duplicate or case-colliding package file.' }
    if ([long]$file.length -lt 0 -or [long]$file.length -gt 1GB -or [string]$file.sha256 -notmatch '^[a-fA-F0-9]{64}$') { throw 'Invalid package file proof.' }
    $names.Add($path, $path)
    $output.Add([pscustomobject]@{ path=$path; length=[long]$file.length; sha256=([string]$file.sha256).ToLowerInvariant() })
  }
  foreach ($path in @($names.Values) + @($directoryNames.Values)) {
    $parent = $path
    while ($parent.Contains('/')) {
      $parent = $parent.Substring(0, $parent.LastIndexOf('/'))
      if ($directoryNames.ContainsKey($parent) -and $directoryNames[$parent] -cne $parent) { throw 'Case-colliding package directory path.' }
      $directoryNames[$parent] = $parent
    }
  }
  foreach ($directory in $directoryNames.Keys) { if ($names.ContainsKey($directory)) { throw 'Package file/directory collision.' } }
  if ($output.Count + $directoryNames.Count -gt 4095 -or ($output | Measure-Object length -Sum).Sum -gt 1GB) { throw 'Package manifest exceeds updater bounds.' }
  return [pscustomobject]@{ format=1; files=@($output | Sort-Object path); directories=@($directoryNames.Values | Sort-Object) }
}

function Get-AtsumiArchiveManifest([string]$Archive, [string]$Prefix, [string]$ExpectedArchiveSha256 = '') {
  $lease = Lock-AtsumiPackageDirectories @($Archive)
  try {
  Assert-AtsumiPackagePath $Archive
  $prefixPath = ConvertTo-AtsumiPackagePath $Prefix
  $archiveStream = [Atsumi.PackageDirectoryLease]::OpenRegularRead([IO.Path]::GetFullPath($Archive))
  try {
  if ($ExpectedArchiveSha256) {
    $proof = Get-AtsumiStreamProof $archiveStream $archiveStream.Length
    if ($proof.sha256 -cne $ExpectedArchiveSha256.ToLowerInvariant()) { throw 'Approved archive changed before manifest derivation.' }
    $archiveStream.Position = 0
  }
  $zip = [IO.Compression.ZipArchive]::new($archiveStream,[IO.Compression.ZipArchiveMode]::Read,$true)
  $files = [Collections.Generic.List[object]]::new()
  $directories = [Collections.Generic.List[string]]::new()
  $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  [long]$totalBytes = 0
  try {
    if ($zip.Entries.Count -gt 4096) { throw 'Too many package archive entries.' }
    foreach ($entry in $zip.Entries) {
      $full = $entry.FullName.Replace('\', '/')
      $directory = $full.EndsWith('/')
      $path = ConvertTo-AtsumiPackagePath $(if ($directory) { $full.Substring(0,$full.Length-1) } else { $full })
      if (-not $seen.Add($path)) { throw 'Duplicate or case-colliding archive entry.' }
      $unixType = ($entry.ExternalAttributes -shr 16) -band 0xf000
      if ($unixType -notin @(0,0x4000,0x8000) -or ($entry.ExternalAttributes -band 0x400) -ne 0 -or ((($entry.ExternalAttributes -band 0x10) -ne 0 -or $unixType -eq 0x4000) -and -not $directory) -or ($unixType -eq 0x8000 -and $directory)) { throw 'Archive contains a link or unexpected file type.' }
      if ($path -ceq $prefixPath) { if (-not $directory -or $entry.Length -ne 0) { throw 'Archive root must be an empty directory entry.' }; continue }
      if (-not $path.StartsWith($prefixPath + '/', [StringComparison]::Ordinal)) { throw 'Archive path is outside its approved root.' }
      $relative = $path.Substring($prefixPath.Length + 1)
      if ($directory) { if ($entry.Length -ne 0) { throw 'Nonempty archive directory.' }; $directories.Add($relative); continue }
      if ($entry.Length -gt 1GB) { throw 'Archive file exceeds size bound.' }
      $totalBytes += $entry.Length
      if ($totalBytes -gt 1GB) { throw 'Archive exceeds total decompressed size bound.' }
      $stream = $entry.Open()
      try { $proof = Get-AtsumiStreamProof $stream $entry.Length } finally { $stream.Dispose() }
      $files.Add([pscustomobject]@{ path=$relative; length=$proof.length; sha256=$proof.sha256 })
    }
    Assert-AtsumiPackagePath $Archive
    return New-AtsumiPackageManifest $files.ToArray() $directories.ToArray()
  } finally { $zip.Dispose() }
  } finally { $archiveStream.Dispose() }
  } finally { $lease.Dispose() }
}

function Assert-AtsumiManifestEqual($Expected, $Actual) {
  $expectedNormalized = New-AtsumiPackageManifest @($Expected.files) @($Expected.directories)
  $actualNormalized = New-AtsumiPackageManifest @($Actual.files) @($Actual.directories)
  if (($expectedNormalized | ConvertTo-Json -Depth 6 -Compress) -cne ($actualNormalized | ConvertTo-Json -Depth 6 -Compress)) { throw 'Package manifest mismatch; existing files preserved.' }
}

function Get-AtsumiDirectoryManifest([string]$Root, $ExpectedManifest = $null) {
  Assert-AtsumiPackagePath $Root
  if (-not ([IO.File]::GetAttributes($Root) -band [IO.FileAttributes]::Directory)) { throw 'Expected package directory.' }
  $rootPath = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar)
  $files = [Collections.Generic.List[object]]::new()
  $directories = [Collections.Generic.List[string]]::new()
  $pending = [Collections.Generic.Queue[string]]::new()
  [long]$totalBytes = 0
  $expectedFiles = @{}
  $expectedDirectories = @{}
  if ($ExpectedManifest) {
    foreach ($file in $ExpectedManifest.files) { $expectedFiles[$file.path] = $file }
    foreach ($directory in $ExpectedManifest.directories) { $expectedDirectories[$directory] = $directory }
  }
  $pending.Enqueue($rootPath)
  while ($pending.Count) {
    $directory = $pending.Dequeue()
    Assert-AtsumiPackagePath $directory
    foreach ($entry in Get-ChildItem -LiteralPath $directory -Force) {
      Assert-AtsumiPackagePath $entry.FullName
      $relative = ConvertTo-AtsumiPackagePath ($entry.FullName.Substring($rootPath.Length + 1))
      if ($entry.PSIsContainer) {
        if ($ExpectedManifest -and (-not $expectedDirectories.ContainsKey($relative) -or $expectedDirectories[$relative] -cne $relative)) { throw 'Unexpected package directory; preserved.' }
        $directories.Add($relative); $pending.Enqueue($entry.FullName)
      } else {
        if ($ExpectedManifest -and (-not $expectedFiles.ContainsKey($relative) -or $expectedFiles[$relative].path -cne $relative)) { throw 'Unexpected package file; preserved.' }
        $proof = Get-AtsumiFileProof $entry.FullName
        $totalBytes += $proof.length
        if ($totalBytes -gt 1GB) { throw 'Package tree exceeds total size bound.' }
        $files.Add([pscustomobject]@{ path=$relative; length=$proof.length; sha256=$proof.sha256 })
      }
      if ($files.Count + $directories.Count -gt 4095) { throw 'Too many package files.' }
    }
  }
  return New-AtsumiPackageManifest $files.ToArray() $directories.ToArray()
}

function Assert-AtsumiPackageTree([string]$Root, $Manifest) {
  Assert-AtsumiManifestEqual $Manifest (Get-AtsumiDirectoryManifest $Root $Manifest)
}

function Copy-AtsumiVerifiedStream($Stream, [string]$Destination, $Proof) {
  if ([long]$Proof.length -lt -1 -or [long]$Proof.length -gt 1GB -or [string]$Proof.sha256 -notmatch '^[a-fA-F0-9]{64}$') { throw 'Invalid expected copy proof.' }
  $lease = Lock-AtsumiPackageDirectories @($Destination)
  try {
  Assert-AtsumiPackagePath $Destination -AllowMissing
  $output = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
  try {
    $buffer = New-Object byte[] 65536
    [long]$length = 0
    while (($read = $Stream.Read($buffer,0,$buffer.Length)) -gt 0) {
      $length += $read
      if ($length -gt 1GB -or ([long]$Proof.length -ge 0 -and $length -gt [long]$Proof.length)) { throw 'Source changed during package copy.' }
      $output.Write($buffer,0,$read)
    }
    $output.Flush($true)
    if ([long]$Proof.length -ge 0 -and $length -ne [long]$Proof.length) { throw 'Source length changed during package copy.' }
  } finally { $output.Dispose() }
  $actual = Get-AtsumiFileProof $Destination
  if (([long]$Proof.length -ge 0 -and $actual.length -ne [long]$Proof.length) -or $actual.sha256 -cne ([string]$Proof.sha256).ToLowerInvariant()) { throw 'Copied package bytes mismatch; partial output preserved.' }
  } finally { $lease.Dispose() }
}

function Copy-AtsumiVerifiedFile([string]$Source, [string]$Destination, $Proof) {
  $lease = Lock-AtsumiPackageDirectories @($Source,$Destination)
  try {
  Assert-AtsumiPackagePath $Source
  if (([IO.File]::GetAttributes($Source) -band [IO.FileAttributes]::Directory)) { throw 'Expected regular source file.' }
  $input = [Atsumi.PackageDirectoryLease]::OpenRegularRead([IO.Path]::GetFullPath($Source))
  try { Copy-AtsumiVerifiedStream $input $Destination $Proof; Assert-AtsumiPackagePath $Source }
  finally { $input.Dispose() }
  } finally { $lease.Dispose() }
}

function New-AtsumiPackageDirectories([string]$Destination, $Manifest) {
  Assert-AtsumiPackagePath $Destination -AllowMissing
  if (Test-Path -LiteralPath $Destination) { throw 'Package destination already exists; preserved.' }
  New-AtsumiSafeDirectory $Destination -MustCreate
  foreach ($directory in @($Manifest.directories | Sort-Object Length)) {
    $path = Join-Path $Destination (ConvertTo-AtsumiPackagePath $directory)
    Assert-AtsumiPackagePath $path -AllowMissing
    New-AtsumiSafeDirectory $path -MustCreate
    Assert-AtsumiPackagePath $path
  }
}

function Copy-AtsumiPackageTree([string]$Source, [string]$Destination, $Manifest) {
  $lease = Lock-AtsumiPackageDirectories @($Source,$Destination)
  try {
  Assert-AtsumiPackageTree $Source $Manifest
  New-AtsumiPackageDirectories $Destination $Manifest
  foreach ($file in $Manifest.files) { Copy-AtsumiVerifiedFile (Join-Path $Source $file.path) (Join-Path $Destination $file.path) $file }
  Assert-AtsumiPackageTree $Source $Manifest
  Assert-AtsumiPackageTree $Destination $Manifest
  } finally { $lease.Dispose() }
}

function Expand-AtsumiPackageArchive([string]$Archive, [string]$Prefix, [string]$Destination, $Manifest) {
  $lease = Lock-AtsumiPackageDirectories @($Archive,$Destination)
  try {
  $archiveStream = [Atsumi.PackageDirectoryLease]::OpenRegularRead([IO.Path]::GetFullPath($Archive))
  try {
  Assert-AtsumiManifestEqual $Manifest (Get-AtsumiArchiveManifest $Archive $Prefix)
  $zip = [IO.Compression.ZipArchive]::new($archiveStream,[IO.Compression.ZipArchiveMode]::Read,$true)
  try {
    $entries = @{}
    foreach ($entry in $zip.Entries) { $entries[$entry.FullName.Replace('\','/')] = $entry }
    New-AtsumiPackageDirectories $Destination $Manifest
    foreach ($file in $Manifest.files) {
      $entry = $entries[$Prefix + '/' + $file.path]
      $input = $entry.Open()
      try { Copy-AtsumiVerifiedStream $input (Join-Path $Destination $file.path) $file } finally { $input.Dispose() }
    }
  } finally { $zip.Dispose() }
  Assert-AtsumiPackageTree $Destination $Manifest
  } finally { $archiveStream.Dispose() }
  } finally { $lease.Dispose() }
}

function New-AtsumiPackageArchive([string]$Root, [string]$Archive, $Manifest) {
  $lease = Lock-AtsumiPackageDirectories @($Root,$Archive)
  try {
  Assert-AtsumiPackageTree $Root $Manifest
  Assert-AtsumiPackagePath $Archive -AllowMissing
  $output = [IO.File]::Open($Archive, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
  $zip = $null
  try {
    $zip = [IO.Compression.ZipArchive]::new($output, [IO.Compression.ZipArchiveMode]::Create, $true)
    [void]$zip.CreateEntry('Atsumi/')
    foreach ($directory in $Manifest.directories) { [void]$zip.CreateEntry('Atsumi/' + $directory + '/') }
    foreach ($file in $Manifest.files) {
      $path = Join-Path $Root $file.path
      $entryLease = Lock-AtsumiPackageDirectories @($path)
      try {
      Assert-AtsumiPackagePath $path
      $input = [Atsumi.PackageDirectoryLease]::OpenRegularRead([IO.Path]::GetFullPath($path))
      $sink = $null
      try {
      $entry = $zip.CreateEntry('Atsumi/' + $file.path, [IO.Compression.CompressionLevel]::Optimal)
      $sink = $entry.Open()
        $buffer = New-Object byte[] 65536
        [long]$length = 0
        while (($read = $input.Read($buffer,0,$buffer.Length)) -gt 0) {
          $length += $read
          if ($length -gt [long]$file.length) { throw 'Source changed during archive creation.' }
          $sink.Write($buffer,0,$read)
        }
        if ($length -ne [long]$file.length) { throw 'Archive source length mismatch.' }
      } finally { if ($sink) { $sink.Dispose() }; $input.Dispose() }
      } finally { $entryLease.Dispose() }
    }
  } finally { if ($zip) { $zip.Dispose() }; $output.Dispose() }
  Assert-AtsumiPackageTree $Root $Manifest
  Assert-AtsumiManifestEqual $Manifest (Get-AtsumiArchiveManifest $Archive 'Atsumi')
  } finally { $lease.Dispose() }
}

function Write-AtsumiPackageBytes([string]$Path, [byte[]]$Bytes) {
  $stream = [IO.MemoryStream]::new($Bytes, $false)
  try {
    $proof = Get-AtsumiStreamProof $stream $Bytes.Length
    $stream.Position = 0
    Copy-AtsumiVerifiedStream $stream $Path $proof
    return $proof
  } finally { $stream.Dispose() }
}

function Get-AtsumiPortableArtifact([string]$Version, [bool]$WithWebView2) {
  if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid portable version.' }
  if ($WithWebView2) { return [pscustomobject]@{name='Atsumi-Portable-With-WebView2.zip';directory="portable-with-webview2-v$Version";kind='portable-initial-with-webview2';updaterCompatible=$false} }
  return [pscustomobject]@{name='Atsumi-Portable.zip';directory="portable-v$Version";kind='portable-updater';updaterCompatible=$true}
}

function Assert-AtsumiUpdaterArchive([string]$Archive, [string]$Version, [switch]$PassThru) {
  $metadataPath = $Archive + '.metadata.json'
  $lease = Lock-AtsumiPackageDirectories @($Archive,$metadataPath)
  try {
  $archiveStream = [Atsumi.PackageDirectoryLease]::OpenRegularRead([IO.Path]::GetFullPath($Archive))
  try {
  Assert-AtsumiPackagePath $metadataPath
  $metadataStream = [Atsumi.PackageDirectoryLease]::OpenRegularRead([IO.Path]::GetFullPath($metadataPath))
  try {
    if ($metadataStream.Length -gt 2MB) { throw 'Portable metadata too large.' }
    $metadataReader = [IO.StreamReader]::new($metadataStream)
    try { $metadata = $metadataReader.ReadToEnd() | ConvertFrom-Json } finally { $metadataReader.Dispose() }
  } finally { $metadataStream.Dispose() }
  if ($metadata.format -ne 1 -or $metadata.kind -cne 'portable-updater' -or $metadata.updaterCompatible -isnot [bool] -or -not $metadata.updaterCompatible -or $metadata.bundledWebView2 -isnot [bool] -or $metadata.bundledWebView2 -or $metadata.version -cne $Version -or $metadata.archiveName -cne 'Atsumi-Portable.zip' -or [IO.Path]::GetFileName($Archive) -cne 'Atsumi-Portable.zip') { throw 'Not an updater-compatible portable artifact.' }
  $proof = Get-AtsumiFileProof $Archive
  if ($proof.length -gt 512MB -or $metadata.bytes -ne $proof.length -or $metadata.sha256 -cne $proof.sha256) { throw 'Portable archive proof mismatch.' }
  $actual = Get-AtsumiArchiveManifest $Archive 'Atsumi' $metadata.sha256
  Assert-AtsumiManifestEqual $metadata.manifest $actual
  $owned = @('atsumi.exe','media-tools','atsumi-portable.json','Start-Atsumi.cmd','Readme.txt')
  foreach ($path in @($actual.files | ForEach-Object { $_.path }) + @($actual.directories)) {
    if ($path.Split('/')[0] -cnotin $owned) { throw 'Portable archive contains a non-updater root.' }
  }
  foreach ($name in $owned) { if ($name -cnotin @($actual.files | ForEach-Object { $_.path }) -and $name -cnotin @($actual.directories)) { throw 'Required updater root missing.' } }
  $zip = [IO.Compression.ZipArchive]::new($archiveStream,[IO.Compression.ZipArchiveMode]::Read,$true)
  try {
    $entry = $zip.Entries | Where-Object { $_.FullName.Replace('\','/') -ceq 'Atsumi/atsumi-portable.json' }
    if (-not $entry -or $entry.Length -gt 65536) { throw 'Invalid portable marker.' }
    $reader = [IO.StreamReader]::new($entry.Open())
    try { $marker = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
    if ($marker.version -cne $Version -or $marker.distribution -cne 'portable' -or $marker.updaterCompatible -isnot [bool] -or -not $marker.updaterCompatible -or $marker.packageKind -cne 'portable-updater') { throw 'Portable marker does not match updater metadata.' }
  } finally { $zip.Dispose() }
  if ($PassThru) { return [pscustomobject]@{ version=$Version; bytes=$proof.length; sha256=$proof.sha256; updaterCompatible=$true } }
  } finally { $archiveStream.Dispose() }
  } finally { $lease.Dispose() }
}

if ($VerifyPortableArchive) {
  if (-not $ExpectedVersion) { throw 'ExpectedVersion is required.' }
  Assert-AtsumiUpdaterArchive $VerifyPortableArchive $ExpectedVersion -PassThru | ConvertTo-Json -Compress
}

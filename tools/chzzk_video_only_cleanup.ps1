[CmdletBinding()]
param(
    [ValidateSet('Plan', 'Apply')][string]$Action = 'Plan',
    [Parameter(Mandatory)][string]$CatalogPath,
    [Parameter(Mandatory)][string]$RecordingRoot,
    [Parameter(Mandatory)][string]$ReportDirectory,
    [string]$PlanPath
)
# Offline, explicitly requested maintenance. Never remove recording directories,
# chat, metadata, timelines, profiles, concat lists, or diagnostic partial files.
$ErrorActionPreference = 'Stop'
$cutoffBytes = 1000000000L
$mediaName = '^(segment-[0-9]{12}|merged-[a-f0-9]{32})\.(mp4|webm)(\.partial)?$'

function Assert-Stopped {
    if (Get-Process -Name atsumi -ErrorAction SilentlyContinue) { throw 'Close Atsumi before maintenance.' }
}
function Resolve-Plain([string]$Path) {
    if ($Path.StartsWith('\\?\')) { $Path = $Path.Substring(4) }
    if (-not [IO.Path]::IsPathFullyQualified($Path)) { throw "Not absolute: $Path" }
    $full = [IO.Path]::GetFullPath($Path)
    $ancestor = $full
    while ($ancestor) {
        if (Test-Path -LiteralPath $ancestor) {
            $item = Get-Item -LiteralPath $ancestor -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Reparse point refused: $ancestor" }
        }
        $ancestor = [IO.Path]::GetDirectoryName($ancestor)
    }
    return $full
}
function Get-Digest([string]$Path) { return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash }
function Get-Inventory([string]$Root) {
    $pending = [Collections.Generic.Queue[string]]::new()
    $pending.Enqueue($Root)
    while ($pending.Count) {
        foreach ($item in Get-ChildItem -LiteralPath $pending.Dequeue() -Force) {
            # The root/ancestors were verified by Read-Facts. Traverse only
            # direct children and reject reparse entries before descending.
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Reparse entry refused: $($item.FullName)" }
            $resolved = [IO.Path]::GetFullPath($item.FullName)
            if (-not $resolved.StartsWith($Root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'File escaped recording root.' }
            if ($item.PSIsContainer) { $pending.Enqueue($resolved); continue }
            $relative = [IO.Path]::GetRelativePath($Root, $resolved)
            $media = $item.DirectoryName -eq $Root -and $item.Name -cmatch $mediaName
            if (-not $media -and $item.Name -match '\.(mp4|webm|m4s|ts)(\.partial)?$') { throw "Unrecognized media file; review manually: $resolved" }
            [pscustomobject][ordered]@{
                relativePath = $relative; bytes = $item.Length; modifiedTicks = $item.LastWriteTimeUtc.Ticks
                createdTicks = $item.CreationTimeUtc.Ticks; media = $media
                # Never hash large videos. Small evidence files are verified byte-for-byte.
                sha256 = $(if (-not $media) { Get-Digest $resolved } else { $null })
            }
        }
    }
}
function Read-Facts([string]$Catalog, [string]$Root) {
    Assert-Stopped
    $entries = @(Get-Content -LiteralPath $Catalog | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })
    $ids = [Collections.Generic.HashSet[string]]::new()
    $targets = @(); $kept = @()
    foreach ($entry in $entries) {
        if ($entry.id -cnotmatch '^[a-f0-9]{32}$' -or -not $ids.Add($entry.id)) { throw 'Invalid/duplicate catalog ID.' }
        $expected = Resolve-Plain (Join-Path $Root $entry.id)
        if ((Resolve-Plain $entry.outputDir) -ne $expected) { throw "Unexpected recording path: $($entry.id)" }
        $metaPath = Join-Path $expected 'recording.json'
        $meta = Get-Content -LiteralPath $metaPath -Raw | ConvertFrom-Json
        if ($meta.id -ne $entry.id -or (Resolve-Plain $meta.outputDir) -ne $expected) { throw 'Metadata identity mismatch.' }
        if ($meta.status -eq 'recording' -or $entry.deletionPending) { throw "Unsettled recording: $($entry.id)" }
        $logicalBytes = [Math]::Max(([long]$meta.bytesWritten + [long]$meta.partial.bytes), [long]$meta.merge.bytes)
        if ($logicalBytes -lt 0) { throw 'Invalid recording size.' }
        $files = @(Get-Inventory $expected | Sort-Object relativePath)
        $record = [pscustomobject][ordered]@{ id = $entry.id; root = $expected; logicalBytes = $logicalBytes; files = $files }
        # An individual unexpectedly large derivative is also protected.
        if ($logicalBytes -lt $cutoffBytes -and -not $entry.mediaRemovedAt -and
            -not @($files | Where-Object { $_.media -and $_.bytes -ge $cutoffBytes }).Count) { $targets += $record }
        else { $kept += $record }
    }
    [pscustomobject][ordered]@{
        catalog = $Catalog; recordingRoot = $Root; cutoffBytes = $cutoffBytes
        catalogSha256 = Get-Digest $Catalog; targets = $targets; kept = $kept
    }
}
function Save-JsonNew([string]$Path, $Value) {
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 16))
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $stream.Write($bytes); $stream.Flush($true) } finally { $stream.Dispose() }
}

$catalog = Resolve-Plain $CatalogPath
$archiveRoot = Resolve-Plain $RecordingRoot
$reports = Resolve-Plain $ReportDirectory
if ((Split-Path $archiveRoot -Leaf) -ne 'BrowserCapture' -or -not (Test-Path -LiteralPath $archiveRoot -PathType Container)) { throw 'An existing BrowserCapture directory is required.' }
if ($reports -eq $archiveRoot -or $reports.StartsWith($archiveRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Keep audit reports outside recording folders.' }
$null = New-Item -ItemType Directory -Path $reports -Force
$facts = Read-Facts $catalog $archiveRoot
$targetFiles = @($facts.targets | ForEach-Object { $_.files | Where-Object media })
$summary = [ordered]@{
    selectedRecords = $facts.targets.Count
    recordsWithVideo = @($facts.targets | Where-Object { @($_.files | Where-Object media).Count }).Count
    videoFiles = $targetFiles.Count; videoBytes = [long]($targetFiles | Measure-Object bytes -Sum).Sum
    protectedRecords = $facts.kept.Count
}
if ($Action -eq 'Plan') {
    $destination = Join-Path $reports ('plan-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N') + '.json')
    Save-JsonNew $destination ([ordered]@{ version = 1; createdAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds(); summary = $summary; facts = $facts })
    [pscustomobject]@{ action = 'planned'; plan = $destination; summary = $summary } | ConvertTo-Json -Depth 4
    exit 0
}
if (-not $PlanPath) { throw 'Apply requires the exact reviewed PlanPath.' }
$plan = Get-Content -LiteralPath (Resolve-Plain $PlanPath) -Raw | ConvertFrom-Json
if ($plan.version -ne 1 -or ($plan.facts | ConvertTo-Json -Depth 16 -Compress) -cne ($facts | ConvertTo-Json -Depth 16 -Compress)) { throw 'Files/catalog changed since plan; generate and review a new plan.' }
Assert-Stopped
$run = Join-Path $reports ('applied-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $run
Save-JsonNew (Join-Path $run 'intent.json') $plan
Copy-Item -LiteralPath $catalog -Destination (Join-Path $run 'catalog.before.jsonl')
# The development launcher replaces these on the next launch. Keep this run's
# error evidence independently; never change the originals.
foreach ($logName in @('debug-app.stdout.log', 'debug-app.stderr.log')) {
    $log = Join-Path (Split-Path $PSScriptRoot) ('.runtime\' + $logName)
    if (Test-Path -LiteralPath $log -PathType Leaf) { Copy-Item -LiteralPath (Resolve-Plain $log) -Destination (Join-Path $run $logName) }
}
$index = Join-Path (Split-Path $catalog) 'startup-index.json'
if (Test-Path -LiteralPath $index) { Copy-Item -LiteralPath (Resolve-Plain $index) -Destination (Join-Path $run 'startup-index.before.json') }
$stamp = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$selected = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
foreach ($record in $facts.targets) { $null = $selected.Add($record.id) }
$updatedLines = @(Get-Content -LiteralPath $catalog | Where-Object { $_.Trim() } | ForEach-Object {
    $entry = $_ | ConvertFrom-Json
    if ($selected.Contains($entry.id)) { $entry | Add-Member -NotePropertyName mediaRemovedAt -NotePropertyValue $stamp -Force }
    $entry | ConvertTo-Json -Compress
})
# Publish cleanup intent before any unlink. A crash cannot trigger a merge of
# intentionally missing media or overwrite its original error/recording.json.
$temporaryCatalog = $catalog + '.' + [Guid]::NewGuid().ToString('N') + '.partial'
[IO.File]::WriteAllLines($temporaryCatalog, $updatedLines, [Text.UTF8Encoding]::new($false))
if ((Get-Digest $catalog) -ne $facts.catalogSha256) { throw 'Catalog changed before publication.' }
Assert-Stopped
[IO.File]::Replace($temporaryCatalog, $catalog, (Join-Path $run 'catalog.replaced.jsonl'))
$journal = [IO.StreamWriter]::new((Join-Path $run 'deleted-files.jsonl'), $false, [Text.UTF8Encoding]::new($false))
$deletedCount = 0; $deletedBytes = 0L; $failures = @()
try {
    foreach ($record in $facts.targets) {
        Assert-Stopped
        foreach ($file in @($record.files | Where-Object media)) {
            $exact = Resolve-Plain (Join-Path $record.root $file.relativePath)
            if ((Split-Path $exact) -ne $record.root -or (Split-Path $exact -Leaf) -cnotmatch $mediaName) { throw 'Deletion target escaped the reviewed media allowlist.' }
            $item = Get-Item -LiteralPath $exact -Force
            if ($item.Length -ne $file.bytes -or $item.LastWriteTimeUtc.Ticks -ne $file.modifiedTicks -or $item.CreationTimeUtc.Ticks -ne $file.createdTicks) { throw 'Video changed since plan.' }
            try {
                Remove-Item -LiteralPath $exact -Force -ErrorAction Stop
                $deletedCount++; $deletedBytes += $file.bytes
                $journal.WriteLine((@{ recordingId = $record.id; file = $file.relativePath; bytes = $file.bytes; deletedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() } | ConvertTo-Json -Compress))
                $journal.Flush(); $journal.BaseStream.Flush($true)
            } catch { $failures += @{ recordingId = $record.id; file = $file.relativePath; error = $_.Exception.Message } }
        }
    }
} finally { $journal.Dispose() }
$verifiedEvidence = 0; $verifiedProtected = 0
foreach ($record in @($facts.targets) + @($facts.kept)) {
    foreach ($file in $record.files) {
        if ($file.media -and $selected.Contains($record.id)) { continue }
        $exact = Resolve-Plain (Join-Path $record.root $file.relativePath)
        $item = Get-Item -LiteralPath $exact -Force
        if ($item.Length -ne $file.bytes -or $item.LastWriteTimeUtc.Ticks -ne $file.modifiedTicks) { throw "Protected file changed: $exact" }
        if (-not $file.media) {
            if ((Get-Digest $exact) -ne $file.sha256) { throw "Evidence changed: $exact" }
            $verifiedEvidence++
        } else { $verifiedProtected++ }
    }
}
$result = [ordered]@{ action = 'applied'; report = $run; removedVideoFiles = $deletedCount; removedVideoBytes = $deletedBytes; verifiedEvidenceFiles = $verifiedEvidence; verifiedProtectedVideos = $verifiedProtected; failures = $failures; summary = $summary }
Save-JsonNew (Join-Path $run 'result.json') $result
$result | ConvertTo-Json -Depth 6
if ($failures.Count) { exit 2 }

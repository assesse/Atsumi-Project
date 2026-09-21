$ErrorActionPreference = 'Stop'
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('atsumi-cleanup-test-' + [Guid]::NewGuid().ToString('N'))
$recordingRoot = Join-Path $fixtureRoot 'CHZZK\BrowserCapture'
$reportRoot = Join-Path $fixtureRoot 'reports'
$null = New-Item -ItemType Directory -Path $recordingRoot -Force
$catalog = Join-Path $fixtureRoot 'catalog.jsonl'
$entries = @()
foreach ($id in @(('a' * 32), ('b' * 32))) {
    $root = Join-Path $recordingRoot $id
    $null = New-Item -ItemType Directory -Path $root
    $meta = @{ id=$id; outputDir=$root; status='interrupted'; bytesWritten=$(if($id.StartsWith('a')){100}else{1000000001L}); lastError='keep exact error'; segmentCount=1 }
    [IO.File]::WriteAllText((Join-Path $root 'recording.json'), ($meta | ConvertTo-Json))
    [IO.File]::WriteAllText((Join-Path $root 'chat.jsonl'), 'keep chat')
    [IO.File]::WriteAllText((Join-Path $root 'channel-profile.json'), 'keep profile')
    [IO.File]::WriteAllText((Join-Path $root 'merged-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.timeline.jsonl.partial'), 'keep unfinished timeline')
    [IO.File]::WriteAllText((Join-Path $root 'segment-000000000000.mp4'), 'video')
    [IO.File]::WriteAllText((Join-Path $root 'merged-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.mp4.partial'), 'partial video')
    $entries += (@{ id=$id; outputDir=$root } | ConvertTo-Json -Compress)
}
[IO.File]::WriteAllLines($catalog, $entries)
$script = Join-Path $PSScriptRoot 'chzzk_video_only_cleanup.ps1'
$argsForScript = @{ CatalogPath=$catalog; RecordingRoot=$recordingRoot; ReportDirectory=$reportRoot }
$planned = & $script @argsForScript -Action Plan | ConvertFrom-Json
if ($planned.summary.videoFiles -ne 2 -or $planned.summary.protectedRecords -ne 1) { throw 'Wrong deletion scope.' }
$applied = & $script @argsForScript -Action Apply -PlanPath $planned.plan | ConvertFrom-Json
if ($applied.removedVideoFiles -ne 2 -or $applied.verifiedEvidenceFiles -ne 8 -or $applied.verifiedProtectedVideos -ne 2) { throw 'Evidence/large-record preservation failed.' }
if (Test-Path -LiteralPath (Join-Path $recordingRoot (('a' * 32) + '\segment-000000000000.mp4'))) { throw 'Selected video remains.' }
$marked = @(Get-Content -LiteralPath $catalog | ForEach-Object { $_ | ConvertFrom-Json })
if (-not $marked[0].mediaRemovedAt -or $marked[1].mediaRemovedAt) { throw 'Wrong catalog markers.' }
$repeat = & $script @argsForScript -Action Plan | ConvertFrom-Json
if ($repeat.summary.selectedRecords -ne 0) { throw 'Cleanup not idempotent.' }
# A stale plan must be refused before changing the catalog or any files.
$staleCatalog = [IO.File]::ReadAllBytes($catalog)
$rejected = $false
try { $null = & $script @argsForScript -Action Apply -PlanPath $planned.plan } catch { $rejected = $true }
if (-not $rejected -or [Convert]::ToBase64String($staleCatalog) -ne [Convert]::ToBase64String([IO.File]::ReadAllBytes($catalog))) { throw 'Stale plan was accepted.' }
[pscustomobject]@{ passed=$true; fixture=$fixtureRoot; checked='video-only deletion, timeline partial retention, exact evidence hashes, large-record preservation, markers, idempotence, stale-plan refusal' } | ConvertTo-Json

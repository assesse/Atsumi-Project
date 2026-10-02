param(
  [ValidateSet('Status','Queue','Watch','Recover','Cancel')][string]$Action = 'Status',
  [string[]]$EntryIds = @(),
  [switch]$ConfirmAction,
  [ValidateRange(1,100000)][int]$Page = 1,
  [switch]$IncludeSettled,
  [ValidateRange(2,60)][int]$IntervalSeconds = 5,
  [ValidateRange(1,720)][int]$Samples = 12
)
$ErrorActionPreference = 'Stop'
$endpointFile = Join-Path $env:APPDATA 'local.atsumi.next\diagnostics\control-endpoint.json'
$endpoint = Get-Content -LiteralPath $endpointFile -Raw | ConvertFrom-Json
if ($endpoint.url -notmatch '^http://127\.0\.0\.1:[0-9]{1,5}$' -or $endpoint.token -notmatch '^[a-f0-9]{64}$') { throw 'Invalid local control descriptor' }
$appProcess = Get-Process -Id $endpoint.pid -ErrorAction Stop
if ($appProcess.ProcessName -ne 'atsumi') { throw 'Stale endpoint: Atsumi is not running at this process ID' }
$requestHeaders = @{ Authorization = "Bearer $($endpoint.token)" }
function Read-Control([string]$Route) {
  Invoke-RestMethod -Uri "$($endpoint.url)$Route" -Headers $requestHeaders -TimeoutSec 5
}
if ($Action -eq 'Status') { Read-Control '/status' | ConvertTo-Json -Depth 12; return }
$queueRoute = "/queue?page=$Page&includeSettled=$($IncludeSettled.IsPresent.ToString().ToLowerInvariant())"
if ($Action -eq 'Queue') { Read-Control $queueRoute | ConvertTo-Json -Depth 12; return }
if ($Action -eq 'Watch') {
  for ($sample = 0; $sample -lt $Samples; $sample++) {
    $health = Read-Control '/status'
    $queue = Read-Control $queueRoute
    [pscustomobject]@{ Time=(Get-Date).ToString('o'); Pid=$health.pid; PulseAgeMs=$health.health.pulseAgeMs; NativeUnresponsive=$health.health.nativeUnresponsive; ReloadPending=$health.health.reloadPending; RendererMiB=[math]::Round($health.health.rendererPrivateBytes / 1MB); Active=$queue.globalActive; Counts=$queue.counts; CheckpointSavedAt=$health.checkpointSavedAt } | ConvertTo-Json -Depth 5 -Compress
    if ($sample + 1 -lt $Samples) { Start-Sleep -Seconds $IntervalSeconds }
  }
  return
}
if (!$ConfirmAction) { throw 'Mutating actions require -ConfirmAction. Recover reloads only the UI; Cancel preserves received files and history.' }
$health = Read-Control '/status'
$payload = @{ confirm=$true; epoch=$health.health.epoch }
$route = '/recover-ui'
if ($Action -eq 'Cancel') {
  if ($EntryIds.Count -lt 1 -or $EntryIds.Count -gt 200) { throw 'Provide 1..200 explicit EntryIds from Queue output' }
  $payload.entryIds = $EntryIds
  $route = '/cancel-downloads'
}
Invoke-RestMethod -Method Post -Uri "$($endpoint.url)$route" -Headers $requestHeaders -ContentType 'application/json' -Body ($payload | ConvertTo-Json -Compress) -TimeoutSec 10 | ConvertTo-Json -Depth 12
if ($Action -eq 'Recover') { Write-Output 'Reload was requested, not yet confirmed. Verify that Status shows a NEW epoch and a recent pulse; do not assume HTTP acceptance means recovery.' }

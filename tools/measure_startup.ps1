param(
  [string]$Executable = (Join-Path (Split-Path -Parent $PSScriptRoot) 'src-tauri\target\release\atsumi.exe'),
  [int]$TimeoutSeconds = 30,
  [ValidateSet('Release', 'Development')]
  [string]$LaunchMode = 'Release'
)
$ErrorActionPreference = 'Stop'
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
if (Get-Process atsumi -ErrorAction SilentlyContinue) {
  throw 'Close Atsumi normally before measuring. This tool never stops a recording or process.'
}
$log = Join-Path $env:LOCALAPPDATA 'Atsumi\Logs\startup-timings.jsonl'
$prefetchArgument = if ($LaunchMode -eq 'Development') { '/prefetch:2' } else { '/prefetch:1' }
$requested = [DateTime]::UtcNow
$process = Start-Process -FilePath $executablePath -ArgumentList $prefetchArgument -PassThru
$deadline = $requested.AddSeconds($TimeoutSeconds)
do {
  Start-Sleep -Milliseconds 100
  $rows = @()
  if (Test-Path -LiteralPath $log) {
    $rows = @(Get-Content -LiteralPath $log -Tail 100 | ForEach-Object {
      try { $_ | ConvertFrom-Json } catch { }
    } | Where-Object { $_.pid -eq $process.Id -and $_.unixMs -ge ([DateTimeOffset]$requested).ToUnixTimeMilliseconds() })
  }
  $interactive = $rows | Where-Object stage -eq 'interactive_frame' | Select-Object -First 1
  $process.Refresh()
} until ($interactive -or $process.HasExited -or [DateTime]::UtcNow -ge $deadline)
$creation = $process.StartTime.ToUniversalTime()
[ordered]@{
  processId = $process.Id
  requestedAtUtc = $requested.ToString('o')
  processCreatedAtUtc = $creation.ToString('o')
  executable = $executablePath
  prefetchArgument = $prefetchArgument
  version = ($rows | Select-Object -First 1).version
  interactive = [bool]$interactive
  processToInteractiveMs = if ($interactive) { $interactive.elapsedMs } else { $null }
  launchToInteractiveMs = if ($interactive) { [math]::Round(($creation - $requested).TotalMilliseconds + $interactive.elapsedMs) } else { $null }
  stages = @($rows | Select-Object stage,elapsedMs)
} | ConvertTo-Json -Depth 4
if (-not $interactive) { exit 1 }
# Leave the actual app open for manual input/quit verification.

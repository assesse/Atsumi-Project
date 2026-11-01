param(
    [switch]$DrainWork,
    [ValidateRange(0, 2147483647)][int]$TargetProcessId = 0,
    [ValidateRange(1, 120)][int]$WaitSeconds = 45,
    [string]$EndpointFile = (Join-Path $env:APPDATA 'local.atsumi.next\diagnostics\control-endpoint.json'),
    [string]$ExpectedExecutable = (Join-Path (Split-Path -Parent $PSScriptRoot) 'src-tauri\target\debug\atsumi.exe')
)
$ErrorActionPreference = 'Stop'
# Each app also owns a per-PID descriptor; a newer/temporary instance must not
# hide the running target by overwriting the backwards-compatible latest file.
$discover = (Split-Path -Leaf $EndpointFile) -eq 'control-endpoint.json'
$descriptorPaths = @()
$directory = Split-Path -Parent $EndpointFile
if ($discover -and (Test-Path -LiteralPath $directory -PathType Container)) {
    $descriptorPaths += @(Get-ChildItem -LiteralPath $directory -Filter 'control-endpoint-*.json' -File |
        Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 64 -ExpandProperty FullName)
}
if (Test-Path -LiteralPath $EndpointFile -PathType Leaf) { $descriptorPaths += $EndpointFile }
$controlMatches = @{}
foreach ($descriptorPath in $descriptorPaths) {
    $candidateProcess = $null
    try {
        if ((Get-Item -LiteralPath $descriptorPath).Length -gt 4096) { throw 'Oversized control descriptor' }
        $candidate = [IO.File]::ReadAllText($descriptorPath) | ConvertFrom-Json
        $candidateEndpoint = $null
        if ($candidate.version -ne 1 -or $candidate.token -cnotmatch '^[0-9a-f]{64}$' -or
            -not [Uri]::TryCreate([string]$candidate.url, [UriKind]::Absolute, [ref]$candidateEndpoint) -or
            $candidateEndpoint.Scheme -cne 'http' -or $candidateEndpoint.Host -cne '127.0.0.1' -or
            $candidateEndpoint.Port -lt 1 -or $candidateEndpoint.UserInfo -or $candidateEndpoint.Query -or $candidateEndpoint.Fragment -or
            $candidateEndpoint.AbsolutePath -ne '/' -or [long]$candidate.pid -lt 1 -or [long]$candidate.pid -gt [int]::MaxValue) {
            throw 'Invalid Atsumi local-control descriptor.'
        }
        if ($TargetProcessId -and $candidate.pid -ne $TargetProcessId) { continue }
        if ($controlMatches.ContainsKey([int]$candidate.pid)) { continue }
        try { $candidateProcess = [Diagnostics.Process]::GetProcessById([int]$candidate.pid) }
        catch [ArgumentException] { continue }
        $null = $candidateProcess.Handle
        if (-not [string]::Equals($candidateProcess.MainModule.FileName, [IO.Path]::GetFullPath($ExpectedExecutable), [StringComparison]::OrdinalIgnoreCase)) {
            $candidateProcess.Dispose()
            throw 'The control descriptor does not identify the expected Atsumi executable.'
        }
        $controlMatches[[int]$candidate.pid] = @{ Descriptor = $candidate; Endpoint = $candidateEndpoint; Process = $candidateProcess }
    } catch {
        if ($candidateProcess) { $candidateProcess.Dispose() }
        if (-not $discover) { throw }
    }
}
if ($controlMatches.Count -gt 1) {
    foreach ($entry in $controlMatches.Values) { $entry.Process.Dispose() }
    throw 'Multiple matching Atsumi instances; specify -TargetProcessId.'
}
if ($controlMatches.Count -eq 0) {
    # A stale descriptor does not prove absence when another exact app is alive.
    $live = @(Get-Process -Name 'atsumi' -ErrorAction SilentlyContinue | Where-Object {
        (!$TargetProcessId -or $_.Id -eq $TargetProcessId) -and
        [string]::Equals($_.Path, [IO.Path]::GetFullPath($ExpectedExecutable), [StringComparison]::OrdinalIgnoreCase)
    })
    if ($discover -and $live.Count) { throw 'The running Atsumi instance has no current authenticated control descriptor.' }
    Write-Output '{"accepted":false,"phase":"not_running"}'
    exit 0
}
$selected = @($controlMatches.Values)[0]
$descriptor = $selected.Descriptor
$endpoint = $selected.Endpoint
$targetProcess = $selected.Process
Add-Type -AssemblyName System.Net.Http
$handler = [Net.Http.HttpClientHandler]::new()
$handler.UseProxy = $false
$handler.AllowAutoRedirect = $false
$client = [Net.Http.HttpClient]::new($handler)
$client.Timeout = [TimeSpan]::FromSeconds(5)
$client.MaxResponseContentBufferSize = 65536
$client.DefaultRequestHeaders.Authorization = [Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $descriptor.token)
function Send-Control([string]$Method, [string]$Path, [string]$Body) {
    $request = [Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::new($Method), [Uri]::new($endpoint, $Path))
    try {
        if ($Body) { $request.Content = [Net.Http.StringContent]::new($Body, [Text.Encoding]::UTF8, 'application/json') }
        $response = $client.SendAsync($request).GetAwaiter().GetResult()
        try {
            $data = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult() | ConvertFrom-Json
            return @{ Status = [int]$response.StatusCode; Data = $data }
        } finally { $response.Dispose() }
    } finally { $request.Dispose() }
}
try {
    $accepted = $false
    for ($attempt = 0; $attempt -lt 3; $attempt++) {
        $status = Send-Control 'GET' '/status' ''
        if ($status.Status -ne 200 -or $status.Data.pid -ne $descriptor.pid -or -not $status.Data.health.epoch) {
            throw 'The local-control status did not match the expected Atsumi process.'
        }
        $payload = @{ confirm = $true; epoch = [string]$status.Data.health.epoch; pid = [int]$descriptor.pid; drainWork = [bool]$DrainWork } | ConvertTo-Json -Compress
        $result = Send-Control 'POST' '/shutdown' $payload
        if ($result.Status -eq 202 -and $result.Data.accepted -eq $true -and $result.Data.pid -eq $descriptor.pid) { $accepted = $true; break }
        if ($result.Status -eq 403 -or ($DrainWork -and $result.Status -eq 409 -and $result.Data.reason -eq 'active_work_changed')) { continue }
        # Fixed public outcome only; response text may contain private work information.
        [ordered]@{ accepted = $false; pid = [int]$descriptor.pid; phase = 'refused'; status = $result.Status; reason = $result.Data.reason } | ConvertTo-Json -Compress
        exit 2
    }
    if (-not $accepted) { throw 'Atsumi changed state repeatedly; shutdown was not accepted.' }
    $exited = $targetProcess.WaitForExit($WaitSeconds * 1000)
    [ordered]@{ accepted = $true; pid = [int]$descriptor.pid; phase = $(if ($exited) { 'exited' } else { 'draining' }) } | ConvertTo-Json -Compress
    if (-not $exited) { exit 3 }
} finally {
    $client.Dispose()
    $handler.Dispose()
    $targetProcess.Dispose()
}
exit 0

$ErrorActionPreference = 'Stop'
$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
. (Join-Path $PSScriptRoot 'development_port.ps1')
$occupied = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$occupied.ExclusiveAddressUse = $true
$occupied.Start()
try {
  $occupiedPort = $occupied.LocalEndpoint.Port
  $chosen = Get-AtsumiDevelopmentPort -Candidates (@($occupiedPort) + @(14200..14220))
  if ($chosen -eq $occupiedPort) { throw 'Used an occupied port.' }
  $failed = $false
  try { Get-AtsumiDevelopmentPort -Candidates @($occupiedPort) | Out-Null } catch { $failed = $true }
  if (-not $failed) { throw 'An exhausted list must fail without stopping its owner.' }
} finally { $occupied.Stop() }
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('atsumi debug launcher test ' + [guid]::NewGuid().ToString('N'))
$fixtureTools = Join-Path $fixtureRoot 'tools'
New-Item -ItemType Directory -Path $fixtureTools -Force | Out-Null
$fixtureRunner = Join-Path $fixtureTools 'run_tauri_dev.ps1'
$fixtureFrontend = Join-Path $fixtureTools 'run_frontend.ps1'
$fixturePort = Join-Path $fixtureTools 'development_port.ps1'
$fixtureRuntime = Join-Path $fixtureRoot '.runtime'
$fixtureConfig = Join-Path $fixtureRuntime 'tauri-dev-config.json'
$fixtureErrors = Join-Path $fixtureRoot 'stderr.log'
$previousPort = $env:ATSUMI_DEV_PORT
try {
  $env:ATSUMI_DEV_PORT = [string]$chosen
  Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'run_tauri_dev.ps1') -Destination $fixtureRunner
  Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'development_port.ps1') -Destination $fixturePort
  # Reuse the real binding contract. Do not start Node, Cargo, an app or a
  # recording: only the body is replaced with a test result and exit code.
  $tokens = $null
  $parseErrors = $null
  $runnerAst = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'run_frontend.ps1'), [ref]$tokens, [ref]$parseErrors)
  if ($parseErrors.Count -or -not $runnerAst.ParamBlock) { throw 'Invalid frontend runner parameter contract.' }
  $body = @'
@{ Action = $Action; ExtraArgs = @($ExtraArgs); Port = $env:ATSUMI_DEV_PORT } | ConvertTo-Json -Compress
exit 37
'@
  [IO.File]::WriteAllText($fixtureFrontend, $runnerAst.ParamBlock.Extent.Text + "`r`n" + $body)
  $output = & $windowsPowerShell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $fixtureRunner 2> $fixtureErrors
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 37) { throw "Child exit code was not preserved: $exitCode. $(Get-Content -LiteralPath $fixtureErrors -Raw)" }
  $received = $output[-1] | ConvertFrom-Json
  $expectedArgs = @('dev', '--config', $fixtureConfig, '--', '--', '/prefetch:2') | ConvertTo-Json -Compress
  if ($received.Action -cne 'tauri' -or
      ($received.ExtraArgs | ConvertTo-Json -Compress) -cne $expectedArgs -or $received.Port -ne $chosen) {
    throw "PowerShell changed the development arguments: $output"
  }
  $config = Get-Content -LiteralPath $fixtureConfig -Raw | ConvertFrom-Json
  if ($config.build.devUrl -cne "http://127.0.0.1:$chosen") { throw 'Vite and Tauri origins differ.' }
  $written = (Get-Item -LiteralPath $fixtureConfig).LastWriteTimeUtc
  $null = & $windowsPowerShell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $fixtureRunner 2> $fixtureErrors
  if ($LASTEXITCODE -ne 37 -or (Get-Item -LiteralPath $fixtureConfig).LastWriteTimeUtc -ne $written) { throw 'Unchanged config must keep the Cargo cache valid.' }
  $launcher = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'start_debug_app_hidden.ps1') -Raw
  if ($launcher -notmatch '-File \$developmentRunner' -or $launcher -match 'tauri dev --') {
    throw 'The desktop launcher still sends bare separators across the -File boundary.'
  }
  $launcherAst = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'start_debug_app_hidden.ps1'), [ref]$tokens, [ref]$parseErrors)
  if ($parseErrors.Count) { throw 'Invalid launcher syntax.' }
  $buildGuard = $launcherAst.FindAll({param($node)
    $node -is [Management.Automation.Language.PipelineAst] -and
    $node.Extent.Text -eq '$_.Path -ine $expectedDebugExecutable'
  }, $true)
  if ($buildGuard.Count -ne 1) { throw 'Debug launch must not redirect to another Atsumi build.' }
  # Exercise the same predicate without launching apps or showing dialogs.
  $expectedDebugExecutable = Join-Path (Split-Path -Parent $PSScriptRoot) 'src-tauri\target\debug\atsumi.exe'
  $guardPredicate = [scriptblock]::Create($buildGuard[0].Extent.Text)
  $exampleProcesses = @(
    [pscustomobject]@{ Path = $expectedDebugExecutable },
    [pscustomobject]@{ Path = $expectedDebugExecutable.ToUpperInvariant() },
    [pscustomobject]@{ Path = 'C:\Example\Atsumi\atsumi.exe' },
    [pscustomobject]@{ Path = $null }
  )
  $differentBuilds = @($exampleProcesses | Where-Object $guardPredicate)
  if ($differentBuilds.Count -ne 2 -or $differentBuilds[0].Path -cne $exampleProcesses[2].Path -or $null -ne $differentBuilds[1].Path) {
    throw 'Debug launch must accept this checkout only and safely refuse unknown/other executables.'
  }
  $windowSource = $launcherAst.FindAll({param($node) $node -is [Management.Automation.Language.StringConstantExpressionAst] -and $node.Value.Contains('public static class AtsumiLauncherWindow')}, $true)
  if ($windowSource.Count -ne 1 -or $windowSource[0].Value -match '\$developmentRunner|Test-Path') { throw 'PowerShell code leaked into the window-helper C# source.' }
  Write-Output 'PASS: occupied-port fallback, no process termination, matching Vite/Tauri origins, unchanged-config cache, Windows PowerShell 5.1 arguments, paths with spaces, exit code and launcher syntax. No app was launched.'
} finally {
  $env:ATSUMI_DEV_PORT = $previousPort
  # Exact test-created files only. No recursive removal or user paths.
  foreach ($fixtureFile in @($fixtureRunner, $fixtureFrontend, $fixturePort, $fixtureErrors, $fixtureConfig)) {
    if (Test-Path -LiteralPath $fixtureFile) { Remove-Item -LiteralPath $fixtureFile -Force }
  }
  foreach ($fixtureDirectory in @($fixtureTools, $fixtureRuntime, $fixtureRoot)) {
    if (Test-Path -LiteralPath $fixtureDirectory) { Remove-Item -LiteralPath $fixtureDirectory }
  }
}
exit 0

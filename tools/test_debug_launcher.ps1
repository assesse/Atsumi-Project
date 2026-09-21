$ErrorActionPreference = 'Stop'
$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('atsumi debug launcher test ' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
$fixtureRunner = Join-Path $fixtureRoot 'run_tauri_dev.ps1'
$fixtureFrontend = Join-Path $fixtureRoot 'run_frontend.ps1'
$fixtureErrors = Join-Path $fixtureRoot 'stderr.log'
try {
  Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'run_tauri_dev.ps1') -Destination $fixtureRunner
  # Reuse the real binding contract. Do not start Node, Cargo, an app or a
  # recording: only the body is replaced with a test result and exit code.
  $tokens = $null
  $parseErrors = $null
  $runnerAst = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'run_frontend.ps1'), [ref]$tokens, [ref]$parseErrors)
  if ($parseErrors.Count -or -not $runnerAst.ParamBlock) { throw 'Invalid frontend runner parameter contract.' }
  $body = @'
@{ Action = $Action; ExtraArgs = @($ExtraArgs) } | ConvertTo-Json -Compress
exit 37
'@
  [IO.File]::WriteAllText($fixtureFrontend, $runnerAst.ParamBlock.Extent.Text + "`r`n" + $body)
  $output = & $windowsPowerShell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $fixtureRunner 2> $fixtureErrors
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 37) { throw "Child exit code was not preserved: $exitCode. $(Get-Content -LiteralPath $fixtureErrors -Raw)" }
  $received = ($output -join "`n") | ConvertFrom-Json
  if ($received.Action -cne 'tauri' -or
      (($received.ExtraArgs | ConvertTo-Json -Compress) -cne '["dev","--","--","/prefetch:2"]')) {
    throw "PowerShell changed the development arguments: $output"
  }
  $launcher = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'start_debug_app_hidden.ps1') -Raw
  if ($launcher -notmatch '-File \$developmentRunner' -or $launcher -match 'tauri dev --') {
    throw 'The desktop launcher still sends bare separators across the -File boundary.'
  }
  Write-Output 'PASS: Windows PowerShell 5.1 child process, path with spaces, exact Tauri/app arguments, and exit-code propagation. No app was launched.'
} finally {
  # Exact test-created files only. No recursive removal or user paths.
  foreach ($fixtureFile in @($fixtureRunner, $fixtureFrontend, $fixtureErrors)) {
    if (Test-Path -LiteralPath $fixtureFile) { Remove-Item -LiteralPath $fixtureFile -Force }
  }
  Remove-Item -LiteralPath $fixtureRoot
}
exit 0

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$setShortcut = Join-Path $PSScriptRoot 'set_app_prefetch_shortcut.ps1'
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('atsumi-prefetch-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
$target = Join-Path $fixtureRoot 'atsumi.exe'
New-Item -ItemType File -Path $target | Out-Null
$shortcutPath = Join-Path $fixtureRoot 'Atsumi test.lnk'
$backupRoot = Join-Path $fixtureRoot 'backups'
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $target
$shortcut.WorkingDirectory = $fixtureRoot
$shortcut.Description = 'Preserve this metadata'
$shortcut.WindowStyle = 7
$shortcut.Save()
$before = (Get-FileHash -LiteralPath $shortcutPath).Hash
& $setShortcut -ShortcutPath $shortcutPath -ExpectedTarget $target -BackupDirectory $backupRoot -WhatIf
if ((Get-FileHash -LiteralPath $shortcutPath).Hash -ne $before -or (Test-Path -LiteralPath $backupRoot)) {
  throw 'WhatIf modified a shortcut or created a backup.'
}
$result = & $setShortcut -ShortcutPath $shortcutPath -ExpectedTarget $target -BackupDirectory $backupRoot
if (-not $result.Changed -or (Get-FileHash -LiteralPath $result.Backup).Hash -ne $before) {
  throw 'Shortcut update did not preserve a recoverable backup.'
}
$updated = $shell.CreateShortcut($shortcutPath)
if ($updated.Arguments -cne '/prefetch:1' -or $updated.TargetPath -cne $target -or
    $updated.Description -cne 'Preserve this metadata' -or $updated.WindowStyle -ne 7) {
  throw 'Shortcut properties were not preserved.'
}
$after = (Get-FileHash -LiteralPath $shortcutPath).Hash
$second = & $setShortcut -ShortcutPath $shortcutPath -ExpectedTarget $target -BackupDirectory $backupRoot
if ($second.Changed -or (Get-FileHash -LiteralPath $shortcutPath).Hash -ne $after) {
  throw 'Repeated configuration was not idempotent.'
}
$updated.Arguments = '--user-option keep'
$updated.Save()
$customHash = (Get-FileHash -LiteralPath $shortcutPath).Hash
$rejected = $false
try { & $setShortcut -ShortcutPath $shortcutPath -ExpectedTarget $target -BackupDirectory $backupRoot }
catch { $rejected = $_.Exception.Message -match 'custom arguments' }
if (-not $rejected -or (Get-FileHash -LiteralPath $shortcutPath).Hash -ne $customHash) {
  throw 'Custom arguments were overwritten.'
}
$otherTarget = Join-Path $fixtureRoot 'other.exe'
New-Item -ItemType File -Path $otherTarget | Out-Null
$updated.TargetPath = $otherTarget
$updated.Arguments = ''
$updated.Save()
$foreignHash = (Get-FileHash -LiteralPath $shortcutPath).Hash
$rejected = $false
try { & $setShortcut -ShortcutPath $shortcutPath -ExpectedTarget $target -BackupDirectory $backupRoot }
catch { $rejected = $_.Exception.Message -match 'another application' }
if (-not $rejected -or (Get-FileHash -LiteralPath $shortcutPath).Hash -ne $foreignHash) {
  throw 'Another application shortcut was modified.'
}

# Parse without running the real app launchers. Verify the argument reaches the
# application (after both Tauri separators), not PowerShell or the Cargo runner.
foreach ($name in @('start_app_hidden.ps1', 'start_debug_app_hidden.ps1', 'run_tauri_dev.ps1', 'measure_startup.ps1', 'set_app_prefetch_shortcut.ps1')) {
  $tokens = $null
  $parseErrors = $null
  [void][Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $name), [ref]$tokens, [ref]$parseErrors)
  if ($parseErrors.Count) { throw "$name has a syntax error: $parseErrors" }
}
$release = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'start_app_hidden.ps1') -Raw
$development = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'start_debug_app_hidden.ps1') -Raw
$developmentRunner = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'run_tauri_dev.ps1') -Raw
$autostart = Get-Content -LiteralPath (Join-Path $projectRoot 'src-tauri/src/autostart.rs') -Raw
if ($release -notmatch '-ArgumentList "/prefetch:1"' -or
    $development -notmatch '-File \$developmentRunner' -or
    $developmentRunner -notmatch '-Action tauri -ExtraArgs @\("dev", "--", "--", "/prefetch:2"\)' -or
    $autostart -notmatch 'RELEASE_PREFETCH_ARGUMENT: &str = "/prefetch:1"') {
  throw 'Launch paths disagree with the fixed prefetch policy.'
}
& (Join-Path $PSScriptRoot 'test_debug_launcher.ps1')
Write-Output 'PASS: shortcut backup, WhatIf, idempotence, metadata preservation, custom/foreign rejection, and launcher argument checks. No app was launched.'
# Remove only this test's explicitly created files; never recurse over user data.
Get-ChildItem -LiteralPath $backupRoot -File | Remove-Item -Force
Remove-Item -LiteralPath $backupRoot,$shortcutPath,$target,$otherTarget,$fixtureRoot -Force

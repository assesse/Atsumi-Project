$ErrorActionPreference = 'Stop'
$taskbarTestRoot = Join-Path ([IO.Path]::GetTempPath()) ('atsumi taskbar test ' + [guid]::NewGuid().ToString('N'))
$taskbarTestBackups = Join-Path $taskbarTestRoot 'backups'
$taskbarTestLink = Join-Path $taskbarTestRoot 'debug.lnk'
$taskbarTestSetter = Join-Path $PSScriptRoot 'set_debug_taskbar_shortcut.ps1'
$taskbarTestShell = New-Object -ComObject WScript.Shell
$taskbarTestPassed = $false
New-Item -ItemType Directory -Path $taskbarTestRoot | Out-Null
try {
  $taskbarTestShortcut = $taskbarTestShell.CreateShortcut($taskbarTestLink)
  $taskbarTestShortcut.TargetPath = Join-Path $env:SystemRoot 'System32\notepad.exe'
  $taskbarTestShortcut.Save()
  $unrelatedHash = (Get-FileHash -LiteralPath $taskbarTestLink).Hash
  $rejected = $false
  try { & $taskbarTestSetter -ShortcutPath $taskbarTestLink -BackupDirectory $taskbarTestBackups } catch { $rejected = $true }
  if (-not $rejected -or (Get-FileHash -LiteralPath $taskbarTestLink).Hash -cne $unrelatedHash) {
    throw 'Unrelated shortcuts must be refused without changes.'
  }

  $taskbarTestShortcut.TargetPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $taskbarTestShortcut.Arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "' + (Join-Path $PSScriptRoot 'start_debug_app_hidden.ps1') + '"'
  $taskbarTestShortcut.WorkingDirectory = Split-Path -Parent $PSScriptRoot
  $taskbarTestShortcut.Description = 'Shortcut preservation test'
  $taskbarTestShortcut.WindowStyle = 7
  $taskbarTestShortcut.IconLocation = Join-Path (Split-Path -Parent $PSScriptRoot) 'src-tauri\icons\icon.ico,0'
  $taskbarTestShortcut.Save()
  $originalHash = (Get-FileHash -LiteralPath $taskbarTestLink).Hash
  & $taskbarTestSetter -ShortcutPath $taskbarTestLink -BackupDirectory $taskbarTestBackups -WhatIf
  if ((Get-FileHash -LiteralPath $taskbarTestLink).Hash -cne $originalHash -or (Test-Path -LiteralPath $taskbarTestBackups)) {
    throw 'WhatIf must not change the shortcut or create backups.'
  }

  [AtsumiDebugShortcutIdentity]::Write($taskbarTestLink, 'local.atsumi.next')
  $originalHash = (Get-FileHash -LiteralPath $taskbarTestLink).Hash
  $result = & $taskbarTestSetter -ShortcutPath $taskbarTestLink -BackupDirectory $taskbarTestBackups
  if (-not $result.Changed -or $result.PreviousAppId -cne 'local.atsumi.next' -or
      $result.AppId -cne 'local.atsumi.next.debug' -or
      (Get-FileHash -LiteralPath $result.Backup).Hash -cne $originalHash) {
    throw 'Debug identity migration or original backup failed.'
  }
  $updated = $taskbarTestShell.CreateShortcut($taskbarTestLink)
  foreach ($field in @('TargetPath', 'Arguments', 'WorkingDirectory', 'Description', 'IconLocation', 'Hotkey', 'WindowStyle')) {
    if ($updated.$field -cne $taskbarTestShortcut.$field) { throw "Lost shortcut property: $field" }
  }
  $updatedHash = (Get-FileHash -LiteralPath $taskbarTestLink).Hash
  $repeat = & $taskbarTestSetter -ShortcutPath $taskbarTestLink -BackupDirectory $taskbarTestBackups
  if ($repeat.Changed -or (Get-FileHash -LiteralPath $taskbarTestLink).Hash -cne $updatedHash -or
      @(Get-ChildItem -LiteralPath $taskbarTestBackups -File).Count -ne 1) {
    throw 'Repeated setup must be a no-op.'
  }
  $mainSource = Get-Content -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) 'src-tauri\src\main.rs') -Raw
  if (-not $mainSource.Contains('"' + $result.AppId + '"')) { throw 'Shortcut and process taskbar identities differ.' }
  Write-Output 'PASS: wrong-target refusal, dry run, backup, AppID migration, preserved shortcut fields, idempotence, matching process identity. No app launched.'
  $taskbarTestPassed = $true
} finally {
  # Only this test's exact files, no recursive cleanup or real shortcut changes.
  if ($taskbarTestPassed) {
    if ($result -and (Test-Path -LiteralPath $result.Backup)) { Remove-Item -LiteralPath $result.Backup -Force }
    if (Test-Path -LiteralPath $taskbarTestLink) { Remove-Item -LiteralPath $taskbarTestLink -Force }
    if (Test-Path -LiteralPath $taskbarTestBackups) { Remove-Item -LiteralPath $taskbarTestBackups }
    if (Test-Path -LiteralPath $taskbarTestRoot) { Remove-Item -LiteralPath $taskbarTestRoot }
  } else {
    Write-Warning "Failed test fixtures preserved at $taskbarTestRoot"
  }
}

[CmdletBinding(SupportsShouldProcess = $true)]
param(
  [Parameter(Mandatory = $true)][string]$ShortcutPath,
  [Parameter(Mandatory = $true)][string]$ExpectedTarget,
  [Parameter(Mandatory = $true)][string]$BackupDirectory
)

# This only changes an existing, explicitly selected Atsumi executable shortcut.
# No process is launched, no registration is enabled, and no OS cache is touched.
$ErrorActionPreference = 'Stop'
$shortcutFullPath = (Resolve-Path -LiteralPath $ShortcutPath).Path
$targetFullPath = (Resolve-Path -LiteralPath $ExpectedTarget).Path
if ([IO.Path]::GetExtension($shortcutFullPath) -ine '.lnk' -or
    [IO.Path]::GetFileName($targetFullPath) -ine 'atsumi.exe') {
  throw 'Expected an existing .lnk and an Atsumi executable.'
}
if ((Get-Item -LiteralPath $shortcutFullPath).Attributes -band [IO.FileAttributes]::ReparsePoint) {
  throw 'Refusing a redirected shortcut.'
}
$shortcutShell = New-Object -ComObject WScript.Shell
$link = $shortcutShell.CreateShortcut($shortcutFullPath)
if (-not [IO.Path]::IsPathRooted($link.TargetPath) -or
    [IO.Path]::GetFullPath($link.TargetPath) -ine $targetFullPath) {
  throw 'Shortcut target changed or belongs to another application; nothing was written.'
}
$argument = '/prefetch:1'
$previousArguments = $link.Arguments
if ($previousArguments -ceq $argument) {
  [pscustomobject]@{ Changed = $false; Shortcut = $shortcutFullPath; Arguments = $argument }
  return
}
if ($previousArguments.Trim() -and $previousArguments.Trim() -notmatch '^/prefetch:\d+$') {
  throw 'Shortcut contains custom arguments; refusing to overwrite them.'
}
if (-not $PSCmdlet.ShouldProcess($shortcutFullPath, 'Back up and select Atsumi UI prefetch scenario 1')) {
  return
}
New-Item -ItemType Directory -Path $BackupDirectory -Force | Out-Null
$backupRoot = (Resolve-Path -LiteralPath $BackupDirectory).Path
$suffix = [guid]::NewGuid().ToString('N')
$backup = Join-Path $backupRoot (([IO.Path]::GetFileName($shortcutFullPath)) + '.' + $suffix + '.backup')
$temporary = Join-Path ([IO.Path]::GetDirectoryName($shortcutFullPath)) ('.atsumi-prefetch-' + $suffix + '.lnk')
$beforeHash = (Get-FileHash -LiteralPath $shortcutFullPath -Algorithm SHA256).Hash
Copy-Item -LiteralPath $shortcutFullPath -Destination $backup
if ((Get-FileHash -LiteralPath $backup -Algorithm SHA256).Hash -cne $beforeHash) {
  throw 'Shortcut backup verification failed.'
}
try {
  Copy-Item -LiteralPath $shortcutFullPath -Destination $temporary
  $updated = $shortcutShell.CreateShortcut($temporary)
  $updated.Arguments = $argument
  $updated.Save()
  $verified = $shortcutShell.CreateShortcut($temporary)
  foreach ($field in @('TargetPath', 'WorkingDirectory', 'Description', 'IconLocation', 'Hotkey', 'WindowStyle')) {
    if ($verified.$field -cne $link.$field) { throw "Unexpected shortcut change: $field" }
  }
  if ($verified.Arguments -cne $argument) { throw 'Prefetch argument did not persist.' }
  if ((Get-FileHash -LiteralPath $shortcutFullPath -Algorithm SHA256).Hash -cne $beforeHash) {
    throw 'Shortcut was edited concurrently; refusing replacement.'
  }
  # Windows PowerShell 5.1 coerces $null to an invalid empty filename here.
  [IO.File]::Replace($temporary, $shortcutFullPath, [NullString]::Value)
  $installed = $shortcutShell.CreateShortcut($shortcutFullPath)
  if ($installed.Arguments -cne $argument) { throw 'Final shortcut verification failed; backup is preserved.' }
  [pscustomobject]@{ Changed = $true; Shortcut = $shortcutFullPath; Arguments = $argument; Backup = $backup }
} finally {
  if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
}

[CmdletBinding(SupportsShouldProcess = $true)]
param(
  [Parameter(Mandatory = $true)][string]$ShortcutPath,
  [Parameter(Mandatory = $true)][string]$BackupDirectory
)

# Update only an existing shortcut to this checkout's hidden debug launcher.
# Never pin/unpin other apps, restart Explorer, launch Atsumi or change its data.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$launcher = Join-Path $PSScriptRoot 'start_debug_app_hidden.ps1'
$expectedTarget = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$expectedArguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "' + $launcher + '"'
$appId = 'local.atsumi.next.debug'
$shortcutFullPath = (Resolve-Path -LiteralPath $ShortcutPath).Path
if ([IO.Path]::GetExtension($shortcutFullPath) -ine '.lnk' -or
    ((Get-Item -LiteralPath $shortcutFullPath).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
  throw 'Expected an existing, non-redirected .lnk file.'
}
$shortcutShell = New-Object -ComObject WScript.Shell
$link = $shortcutShell.CreateShortcut($shortcutFullPath)
if ($link.TargetPath -ine $expectedTarget -or
    $link.Arguments -cne $expectedArguments -or
    $link.WorkingDirectory -ine $projectRoot) {
  throw 'Not this checkout''s debug launcher shortcut; nothing was written.'
}

if (-not ('AtsumiDebugShortcutIdentity' -as [type])) {
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;

public static class AtsumiDebugShortcutIdentity {
  [ComImport, Guid("00021401-0000-0000-C000-000000000046")]
  private class ShellLink { }

  [StructLayout(LayoutKind.Sequential)]
  private struct PropertyKey {
    public Guid format;
    public uint id;
  }

  // The PROPVARIANT union occupies 16 bytes on x64 and 8 bytes on x86.
  [StructLayout(LayoutKind.Sequential)]
  private struct PropertyVariant {
    public ushort kind, reserved1, reserved2, reserved3;
    public IntPtr value, reserved4;
  }

  [ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  private interface IPropertyStore {
    void GetCount(out uint count);
    void GetAt(uint index, out PropertyKey key);
    void GetValue(ref PropertyKey key, out PropertyVariant value);
    void SetValue(ref PropertyKey key, ref PropertyVariant value);
    void Commit();
  }

  [DllImport("ole32.dll")]
  private static extern int PropVariantClear(ref PropertyVariant value);
  [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
  private static extern void SHChangeNotify(uint eventId, uint flags, string item, IntPtr other);

  private static PropertyKey AppIdKey() {
    return new PropertyKey { format = new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), id = 5 };
  }

  public static string Read(string path) {
    object link = new ShellLink();
    try {
      ((IPersistFile)link).Load(path, 0);
      var key = AppIdKey();
      PropertyVariant value;
      ((IPropertyStore)link).GetValue(ref key, out value);
      try {
        if (value.kind == 0) return null;
        if (value.kind == 8) return Marshal.PtrToStringBSTR(value.value);
        if (value.kind != 31) throw new InvalidOperationException("Unexpected AppID property type: " + value.kind);
        return Marshal.PtrToStringUni(value.value);
      } finally { Marshal.ThrowExceptionForHR(PropVariantClear(ref value)); }
    } finally { Marshal.FinalReleaseComObject(link); }
  }

  public static void Write(string path, string appId) {
    object link = new ShellLink();
    try {
      var file = (IPersistFile)link;
      file.Load(path, 2);
      var key = AppIdKey();
      var value = new PropertyVariant { kind = 31, value = Marshal.StringToCoTaskMemUni(appId) };
      try {
        var store = (IPropertyStore)link;
        store.SetValue(ref key, ref value);
        store.Commit();
        file.Save(path, true);
      } finally { Marshal.ThrowExceptionForHR(PropVariantClear(ref value)); }
    } finally { Marshal.FinalReleaseComObject(link); }
  }

  public static void NotifyChanged(string path) {
    // SHCNE_UPDATEITEM, SHCNF_PATHW. Let Explorer refresh without restarting it.
    SHChangeNotify(0x00002000, 0x0005, path, IntPtr.Zero);
  }
}
'@
}

$previousId = [AtsumiDebugShortcutIdentity]::Read($shortcutFullPath)
if ($previousId -ceq $appId) {
  [pscustomobject]@{ Changed = $false; Shortcut = $shortcutFullPath; AppId = $appId }
  return
}
if (-not $PSCmdlet.ShouldProcess($shortcutFullPath, 'Back up and separate the debug taskbar identity')) {
  return
}
New-Item -ItemType Directory -Path $BackupDirectory -Force | Out-Null
$backupRoot = (Resolve-Path -LiteralPath $BackupDirectory).Path
$suffix = [guid]::NewGuid().ToString('N')
$backup = Join-Path $backupRoot (([IO.Path]::GetFileName($shortcutFullPath)) + '.' + $suffix + '.backup')
$temporary = Join-Path ([IO.Path]::GetDirectoryName($shortcutFullPath)) ('.atsumi-debug-' + $suffix + '.lnk')
$beforeHash = (Get-FileHash -LiteralPath $shortcutFullPath -Algorithm SHA256).Hash
Copy-Item -LiteralPath $shortcutFullPath -Destination $backup
if ((Get-FileHash -LiteralPath $backup -Algorithm SHA256).Hash -cne $beforeHash) {
  throw 'Shortcut backup verification failed.'
}
try {
  Copy-Item -LiteralPath $shortcutFullPath -Destination $temporary
  [AtsumiDebugShortcutIdentity]::Write($temporary, $appId)
  $verified = $shortcutShell.CreateShortcut($temporary)
  foreach ($field in @('TargetPath', 'Arguments', 'WorkingDirectory', 'Description', 'IconLocation', 'Hotkey', 'WindowStyle')) {
    if ($verified.$field -cne $link.$field) { throw "Unexpected shortcut change: $field" }
  }
  if ([AtsumiDebugShortcutIdentity]::Read($temporary) -cne $appId) { throw 'Debug AppID did not persist.' }
  if ((Get-FileHash -LiteralPath $shortcutFullPath -Algorithm SHA256).Hash -cne $beforeHash) {
    throw 'Shortcut was edited concurrently; refusing replacement.'
  }
  [IO.File]::Replace($temporary, $shortcutFullPath, [NullString]::Value)
  [AtsumiDebugShortcutIdentity]::NotifyChanged($shortcutFullPath)
  if ([AtsumiDebugShortcutIdentity]::Read($shortcutFullPath) -cne $appId) {
    throw 'Final shortcut verification failed; backup is preserved.'
  }
  [pscustomobject]@{ Changed = $true; Shortcut = $shortcutFullPath; AppId = $appId; PreviousAppId = $previousId; Backup = $backup }
} finally {
  if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
}

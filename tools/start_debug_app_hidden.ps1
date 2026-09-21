[CmdletBinding()]
param(
  [switch]$CheckOnly
)

$ErrorActionPreference = "Stop"
# This launcher runs Windows PowerShell 5.1 even when started by a PowerShell 7
# host. Prefer its own built-in modules; an inherited PS7 Utility module can
# otherwise make Get-FileHash unavailable during media-tool verification.
# This changes only this launcher/child environment, not user or system settings.
$launcherBuiltinModules = [System.IO.Path]::Combine($PSHOME, "Modules")
$env:PSModulePath = (@($launcherBuiltinModules) + @(
  $env:PSModulePath -split ";" | Where-Object { $_ -and $_ -ne $launcherBuiltinModules }
)) -join ";"
$projectRoot = Split-Path -Parent $PSScriptRoot
$frontendRunner = Join-Path $PSScriptRoot "run_frontend.ps1"
$developmentRunner = Join-Path $PSScriptRoot "run_tauri_dev.ps1"
$runtimeDirectory = Join-Path $projectRoot ".runtime"
$logPath = Join-Path $runtimeDirectory "debug-launch.log"
$standardOutput = Join-Path $runtimeDirectory "debug-app.stdout.log"
$standardError = Join-Path $runtimeDirectory "debug-app.stderr.log"
$launcherMutex = $null
$ownsLauncherMutex = $false

function Show-AtsumiLaunchFailure {
  param([string]$Detail)
  # This launcher has no console. A failed dev server must be actionable,
  # not look like a desktop shortcut that did nothing. Never kill its owner.
  $hint = if ($Detail -match 'Port 1420 is already in use') {
    "Port 1420 is already in use. Close the preview/development server and try again. No existing app or recording was stopped."
  } else {
    "The development app could not start. See the launch logs for details."
  }
  Add-Type -AssemblyName System.Windows.Forms
  [void][System.Windows.Forms.MessageBox]::Show(
    "$hint`r`n`r`n$logPath`r`n$standardError",
    "Atsumi - launch failed", [System.Windows.Forms.MessageBoxButtons]::OK,
    [System.Windows.Forms.MessageBoxIcon]::Error
  )
}

New-Item -ItemType Directory -Force -Path $runtimeDirectory | Out-Null
@(
  "Atsumi - current-source development app"
  "Started: $([DateTimeOffset]::Now.ToString('O'))"
  "Branch: $(git -C $projectRoot branch --show-current)"
  "Commit: $(git -C $projectRoot rev-parse --short HEAD)"
  ""
) | Set-Content -LiteralPath $logPath -Encoding UTF8

if (-not (Test-Path -LiteralPath $frontendRunner -PathType Leaf)) {
  "Missing tools\run_frontend.ps1" | Add-Content -LiteralPath $logPath -Encoding UTF8
  exit 1
}

if (-not (Test-Path -LiteralPath (Join-Path $projectRoot "node_modules\@tauri-apps\cli\tauri.js") -PathType Leaf)) {
  "Frontend dependencies are missing. Run pnpm install once." |
    Add-Content -LiteralPath $logPath -Encoding UTF8
  exit 1
}

if ($CheckOnly) {
  "Debug launcher check completed successfully." |
    Add-Content -LiteralPath $logPath -Encoding UTF8
  exit 0
}

$runningApp = Get-Process -Name "atsumi" -ErrorAction SilentlyContinue
if ($runningApp) {
  # A desktop shortcut is also a way to bring a minimized/tray window back.
  # Never silently exit, spawn another watcher, or kill a recording here.
  Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class AtsumiLauncherWindow {
  private delegate bool WindowVisitor(IntPtr hwnd, IntPtr context);
  [DllImport("user32.dll")]
  private static extern bool EnumWindows(WindowVisitor visitor, IntPtr context);
  [DllImport("user32.dll")]
  private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  private static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int count);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  private static extern int GetClassName(IntPtr hwnd, StringBuilder text, int count);
  public static IntPtr FindMainWindow(uint processId) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((hwnd, context) => {
      uint owner;
      GetWindowThreadProcessId(hwnd, out owner);
      if (owner != processId) return true;
      var title = new StringBuilder(256);
      var kind = new StringBuilder(256);
      GetWindowText(hwnd, title, title.Capacity);
      GetClassName(hwnd, kind, kind.Capacity);
      if (kind.ToString() != "Tauri Window" || title.ToString() != "Atsumi") return true;
      found = hwnd;
      return false;
    }, IntPtr.Zero);
    return found;
  }
  [DllImport("user32.dll", SetLastError = true)]
  public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint message,
    UIntPtr wparam, IntPtr lparam, uint flags, uint timeout, out UIntPtr result);
  [DllImport("user32.dll")]
  public static extern bool ShowWindowAsync(IntPtr hwnd, int command);
  [DllImport("user32.dll")]
  public static extern bool IsIconic(IntPtr hwnd);
  [DllImport("user32.dll")]
  public static extern bool SetForegroundWindow(IntPtr hwnd);
}

if (-not (Test-Path -LiteralPath $developmentRunner -PathType Leaf)) {
  "Missing tools\run_tauri_dev.ps1" | Add-Content -LiteralPath $logPath -Encoding UTF8
  exit 1
}
'@
  foreach ($existingApp in $runningApp) {
    # Process.MainWindowHandle can pick the visible 16px single-instance helper
    # instead of the minimized main window. Resolve our exact window identity.
    $existingWindow = [AtsumiLauncherWindow]::FindMainWindow([uint32]$existingApp.Id)
    if ($existingWindow -eq [IntPtr]::Zero) { continue }
    $windowReply = [UIntPtr]::Zero
    $responsive = [AtsumiLauncherWindow]::SendMessageTimeout(
      $existingWindow, 0, [UIntPtr]::Zero, [IntPtr]::Zero, 2, 750, [ref]$windowReply
    ) -ne [IntPtr]::Zero
    if ($responsive) {
      $showCommand = if ([AtsumiLauncherWindow]::IsIconic($existingWindow)) { 9 } else { 5 }
      [void][AtsumiLauncherWindow]::ShowWindowAsync($existingWindow, $showCommand)
      [void][AtsumiLauncherWindow]::SetForegroundWindow($existingWindow)
      "Restored the existing Atsumi window (PID $($existingApp.Id))." |
        Add-Content -LiteralPath $logPath -Encoding UTF8
      exit 0
    }
  }
  "Atsumi is running but its window could not be restored. No process was terminated." |
    Add-Content -LiteralPath $logPath -Encoding UTF8
  # The launcher is hidden; an actionable notice must not disappear in its log.
  Add-Type -AssemblyName System.Windows.Forms
  [void][System.Windows.Forms.MessageBox]::Show(
    "Atsumi is already running, but its window is not responding or is still starting. Wait briefly and try again. If it stays unresponsive, close Atsumi in Task Manager after checking recording activity.",
    "Atsumi", [System.Windows.Forms.MessageBoxButtons]::OK,
    [System.Windows.Forms.MessageBoxIcon]::Warning
  )
  exit 74
}

$createdNew = $false
$launcherMutex = [System.Threading.Mutex]::new(
  $true,
  "Local\AtsumiNext.DebugLauncher",
  [ref]$createdNew
)
if (-not $createdNew) {
  $launcherMutex.Dispose()
  exit 73
}
$ownsLauncherMutex = $true

Remove-Item -LiteralPath $standardOutput, $standardError -Force -ErrorAction SilentlyContinue

Push-Location $projectRoot
try {
  "Starting Tauri development mode. Rust uses the incremental debug cache and frontend changes use Vite." |
    Add-Content -LiteralPath $logPath -Encoding UTF8

  $powershellPath = Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
  $previousErrorActionPreference = $ErrorActionPreference
  try {
    $ErrorActionPreference = "Continue"
    & $powershellPath `
      -NoLogo `
      -NoProfile `
      -NonInteractive `
      -ExecutionPolicy Bypass `
      -WindowStyle Hidden `
      -File $developmentRunner `
      1> $standardOutput `
      2> $standardError
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
  }

  if ($null -eq $exitCode) {
    $exitCode = 1
  }
  "Development process exit code: $exitCode" |
    Add-Content -LiteralPath $logPath -Encoding UTF8
  if ([int]$exitCode -ne 0) {
    $failureDetail = Get-Content -LiteralPath $standardError -Tail 80 -ErrorAction SilentlyContinue | Out-String
    Show-AtsumiLaunchFailure -Detail $failureDetail
  }
  exit ([int]$exitCode)
} catch {
  ($_ | Out-String) | Add-Content -LiteralPath $logPath -Encoding UTF8
  Show-AtsumiLaunchFailure -Detail ($_ | Out-String)
  exit 1
} finally {
  Pop-Location
  if ($ownsLauncherMutex) {
    $launcherMutex.ReleaseMutex()
  }
  if ($null -ne $launcherMutex) {
    $launcherMutex.Dispose()
  }
}

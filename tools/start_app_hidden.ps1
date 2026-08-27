[CmdletBinding()]
param(
  [switch]$CheckOnly,
  [switch]$Rebuild
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$frontendRunner = Join-Path $PSScriptRoot "run_frontend.ps1"
$runtimeDirectory = Join-Path $projectRoot ".runtime"
$releaseExecutable = Join-Path $projectRoot "src-tauri\target\release\atsumi.exe"
$logName = if ($CheckOnly) { "launcher-check.log" } else { "app-launch.log" }
$logPath = Join-Path $runtimeDirectory $logName
$launcherMutex = $null
$ownsLauncherMutex = $false

function Protect-LogText {
  param([AllowEmptyString()][string]$Text)

  $safe = $Text.Replace($projectRoot, "<project-root>")
  if (-not [string]::IsNullOrEmpty($env:USERPROFILE)) {
    $safe = $safe.Replace($env:USERPROFILE, "%USERPROFILE%")
  }
  return $safe -replace '(?i)(?:https?|file)://\S+', '<redacted-url>'
}

function Add-ProcessLog {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Path
  )

  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    return
  }

  $content = Get-Content -LiteralPath $Path -Raw
  if (-not [string]::IsNullOrEmpty($content)) {
    (Protect-LogText -Text $content) | Add-Content -LiteralPath $logPath -Encoding UTF8
  }
}

if (-not $CheckOnly) {
  $createdNew = $false
  $launcherMutex = [System.Threading.Mutex]::new(
    $true,
    "Local\AtsumiNext.AppLauncher",
    [ref]$createdNew
  )

  if (-not $createdNew) {
    $launcherMutex.Dispose()
    exit 73
  }

  $ownsLauncherMutex = $true
}

New-Item -ItemType Directory -Force -Path $runtimeDirectory | Out-Null

$mode = if ($CheckOnly) { "launcher check" } else { "desktop app" }
$header = @(
  "Atsumi - $mode"
  "Started: $([DateTimeOffset]::Now.ToString('O'))"
  ""
)
$header | Set-Content -LiteralPath $logPath -Encoding UTF8

if (-not (Test-Path -LiteralPath $frontendRunner -PathType Leaf)) {
  "Missing launcher dependency: tools\run_frontend.ps1" |
    Add-Content -LiteralPath $logPath -Encoding UTF8
  exit 1
}

Push-Location $projectRoot
try {
  if ($CheckOnly) {
    & $frontendRunner typecheck *>&1 |
      Out-File -LiteralPath $logPath -Append -Encoding UTF8
    if ($LASTEXITCODE -ne 0) {
      exit $LASTEXITCODE
    }

    & $frontendRunner tauri --version *>&1 |
      Out-File -LiteralPath $logPath -Append -Encoding UTF8
    if ($LASTEXITCODE -ne 0) {
      exit $LASTEXITCODE
    }

    "Launcher check completed successfully." |
      Add-Content -LiteralPath $logPath -Encoding UTF8
    exit 0
  }

  if ($Rebuild) {
    "An explicit release build was requested. Building the desktop application..." |
      Add-Content -LiteralPath $logPath -Encoding UTF8

    $powershellPath = Join-Path $env:SystemRoot `
      "System32\WindowsPowerShell\v1.0\powershell.exe"
    $buildStandardOutput = Join-Path $runtimeDirectory "app-build.stdout.log"
    $buildStandardError = Join-Path $runtimeDirectory "app-build.stderr.log"
    Remove-Item -LiteralPath $buildStandardOutput, $buildStandardError `
      -Force -ErrorAction SilentlyContinue

    # Invoke the build host synchronously from this already-hidden PowerShell
    # process. This preserves paths with spaces as individual arguments and
    # avoids Windows PowerShell 5.1's Start-Process process-tree wait and
    # occasionally unavailable ExitCode on the returned Process object.
    # Windows PowerShell wraps any native stderr line in a non-terminating
    # NativeCommandError. Tauri writes ordinary progress to stderr, so the
    # launcher's global Stop policy would otherwise mistake a successful build
    # for a terminating failure before LASTEXITCODE can be inspected.
    $previousErrorActionPreference = $ErrorActionPreference
    try {
      $ErrorActionPreference = "Continue"
      & $powershellPath `
        -NoLogo `
        -NoProfile `
        -NonInteractive `
        -ExecutionPolicy Bypass `
        -WindowStyle Hidden `
        -File $frontendRunner `
        tauri build --no-bundle `
        1> $buildStandardOutput `
        2> $buildStandardError
      $buildExitCode = $LASTEXITCODE
    } finally {
      $ErrorActionPreference = $previousErrorActionPreference
    }

    if ($null -eq $buildExitCode) {
      $buildExitCode = 1
    } else {
      $buildExitCode = [int]$buildExitCode
    }

    Add-ProcessLog -Path $buildStandardOutput
    Add-ProcessLog -Path $buildStandardError
    "Build process exit code: $buildExitCode" |
      Add-Content -LiteralPath $logPath -Encoding UTF8

    if ($buildExitCode -ne 0) {
      exit $buildExitCode
    }
  }

  if (-not (Test-Path -LiteralPath $releaseExecutable -PathType Leaf)) {
    "No local release exists. Run build-app.vbs once to create src-tauri\target\release\atsumi.exe." |
      Add-Content -LiteralPath $logPath -Encoding UTF8
    exit 1
  }

  "Launching the verified release application." |
    Add-Content -LiteralPath $logPath -Encoding UTF8

  $appStandardOutput = Join-Path $runtimeDirectory "app.stdout.log"
  $appStandardError = Join-Path $runtimeDirectory "app.stderr.log"
  Remove-Item -LiteralPath $appStandardOutput, $appStandardError `
    -Force -ErrorAction SilentlyContinue

  $appProcess = Start-Process `
    -FilePath $releaseExecutable `
    -WorkingDirectory $projectRoot `
    -WindowStyle Normal `
    -RedirectStandardOutput $appStandardOutput `
    -RedirectStandardError $appStandardError `
    -PassThru

  $appProcess.WaitForExit()
  $appProcess.Refresh()

  Add-ProcessLog -Path $appStandardOutput
  Add-ProcessLog -Path $appStandardError

  if ($appProcess.ExitCode -ne 0) {
    exit $appProcess.ExitCode
  }
} catch {
  (Protect-LogText -Text ($_ | Out-String)) |
    Add-Content -LiteralPath $logPath -Encoding UTF8
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

exit 0

param([Parameter(Mandatory=$true)][string]$SigningKeyPath, [string]$HelperExecutable)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$version = (Get-Content -LiteralPath (Join-Path $project 'package.json') -Raw | ConvertFrom-Json).version
$executable = if ($HelperExecutable) { (Resolve-Path -LiteralPath $HelperExecutable).Path } else { Join-Path $project 'src-tauri/target/release/atsumi.exe' }
if ((Get-Item -LiteralPath $executable).VersionInfo.ProductVersion -ne $version) { throw 'Build the current release first.' }
$testRoot = Join-Path $project ('.runtime/portable-update-smoke-' + [guid]::NewGuid().ToString())
$appRoot = Join-Path $testRoot 'Application with spaces'
$stage = Join-Path $appRoot ('.atsumi-update/' + [guid]::NewGuid().ToString())
$payload = Join-Path $testRoot 'package/Atsumi'
New-Item -ItemType Directory -Path $stage,$payload,(Join-Path $appRoot 'media-tools'),(Join-Path $payload 'media-tools') | Out-Null
$fixture = Join-Path $testRoot 'fixture.exe'
# This script is run with Windows PowerShell for the built-in .NET Framework
# compiler. The child is synthetic, not the real application.
Add-Type -Path (Join-Path $PSScriptRoot 'portable/UpdateFixture.cs') -OutputAssembly $fixture -OutputType ConsoleApplication
$utf8 = [Text.UTF8Encoding]::new($false)
$marker = @{ format=1; distribution='portable'; version=$version } | ConvertTo-Json -Compress
foreach ($root in @($appRoot,$payload)) {
    Copy-Item -LiteralPath $fixture -Destination (Join-Path $root 'atsumi.exe')
    [IO.File]::WriteAllText((Join-Path $root 'atsumi-portable.json'), $marker, $utf8)
    [IO.File]::WriteAllText((Join-Path $root 'media-tools/fixture.txt'), 'media fixture', $utf8)
    [IO.File]::WriteAllText((Join-Path $root 'Start-Atsumi.cmd'), '@exit /b 0', $utf8)
    [IO.File]::WriteAllText((Join-Path $root 'Readme.txt'), 'synthetic fixture only', $utf8)
}
[IO.File]::WriteAllText((Join-Path $appRoot 'user-data-sentinel.txt'), 'preserve me', $utf8)
New-Item -ItemType Directory -Path (Join-Path $appRoot 'WebView2') | Out-Null
[IO.File]::WriteAllText((Join-Path $appRoot 'WebView2/sentinel.txt'), 'preserve runtime', $utf8)
$archive = Join-Path $stage 'update.zip'
Compress-Archive -LiteralPath $payload -DestinationPath $archive
& (Join-Path $PSScriptRoot 'run_frontend.ps1') tauri signer sign --password= --private-key-path $SigningKeyPath $archive
if ($LASTEXITCODE -ne 0) { throw 'Fixture signature failed.' }
Copy-Item -LiteralPath $executable -Destination (Join-Path $stage 'update-helper.exe')
$parent = Start-Process -FilePath (Join-Path $appRoot 'atsumi.exe') -ArgumentList '--wait' -WorkingDirectory $appRoot -WindowStyle Hidden -PassThru
try {
    $plan = @{ parent_pid=$parent.Id; version=$version; signature=(Get-Content -LiteralPath ($archive + '.sig') -Raw).Trim() } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $stage 'plan.json'), $plan, $utf8)
    $helper = Start-Process -FilePath (Join-Path $stage 'update-helper.exe') -ArgumentList '--atsumi-portable-update' -WorkingDirectory $stage -WindowStyle Hidden -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    while (-not (Test-Path -LiteralPath (Join-Path $stage 'ready'))) {
        if ($helper.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Helper did not prepare the signed fixture.' }
        Start-Sleep -Milliseconds 100
    }
    if (Test-Path -LiteralPath (Join-Path $stage 'backup')) { throw 'Helper modified files before normal exit.' }
    [IO.File]::WriteAllText((Join-Path $stage 'apply'), 'apply', $utf8)
    [IO.File]::WriteAllText((Join-Path $appRoot 'exit-parent'), 'normal exit', $utf8)
    if (-not $parent.WaitForExit(10000)) { throw 'Synthetic parent did not exit normally.' }
    if (-not $helper.WaitForExit(30000)) { throw 'Helper did not finish.' }
    if ($helper.ExitCode -ne 0) { throw ('Helper failed: ' + (Get-Content -LiteralPath (Join-Path $stage 'result.txt') -Raw)) }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath (Join-Path $appRoot 'restarted-fixture'))) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Updated fixture did not restart.' }
        Start-Sleep -Milliseconds 100
    }
    if ((Get-Content -LiteralPath (Join-Path $appRoot 'user-data-sentinel.txt') -Raw) -ne 'preserve me') { throw 'User file changed.' }
    if ((Get-Content -LiteralPath (Join-Path $appRoot 'WebView2/sentinel.txt') -Raw) -ne 'preserve runtime') { throw 'Fixed runtime changed.' }
    if (-not (Test-Path -LiteralPath (Join-Path $stage 'backup/atsumi.exe'))) { throw 'Rollback backup missing.' }
    [pscustomobject]@{ passed=$true; signedZip=$true; waitedForNormalExit=$true; restarted=$true; userDataPreserved=$true; optionalRuntimePreserved=$true; retainedFixture=$testRoot } | ConvertTo-Json -Compress
} finally {
    # Release only this test's waiting process. No real app is touched or killed.
    [IO.File]::WriteAllText((Join-Path $appRoot 'exit-parent'), 'normal exit', $utf8)
}

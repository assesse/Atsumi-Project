param()
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'build_receipt.ps1')
# Keep nested synthetic Cargo paths below Win32 MAX_PATH in long worktree paths.
# MustCreate still refuses a collision; no existing fixture is reused/deleted.
$fixtureRoot=Join-Path (Split-Path -Parent $PSScriptRoot) ('.runtime/brt-'+[Guid]::NewGuid().ToString('N').Substring(0,12))
New-AtsumiSafeDirectory $fixtureRoot -MustCreate
$script:checks=[Collections.Generic.List[string]]::new()
$utf8=[Text.UTF8Encoding]::new($false)
function Check([bool]$Value,[string]$Name){if(-not $Value){throw "Synthetic check failed: $Name"};$script:checks.Add($Name)}
function Reject([scriptblock]$Action,[string]$Name){$failed=$false;try{& $Action|Out-Null}catch{$failed=$true};Check $failed $Name}
function Write-TestFile([string]$Path,[string]$Value){New-AtsumiSafeDirectory ([IO.Path]::GetDirectoryName($Path));$null=Write-AtsumiPackageBytes $Path ($utf8.GetBytes($Value))}
function New-TestRoot {
  $root=Join-Path $fixtureRoot ([Guid]::NewGuid().ToString('N').Substring(0,12))
  foreach($directory in @('src','public','shared','tools','src-tauri/src','src-tauri/vendor','src-tauri/icons','src-tauri/capabilities','.github/workflows','docs/releases')){New-AtsumiSafeDirectory (Join-Path $root $directory)}
  foreach($file in @('pnpm-lock.yaml','pnpm-workspace.yaml','.npmrc','index.html','vite.config.ts','tsconfig.json','tsconfig.app.json','tsconfig.node.json','src-tauri/Cargo.lock','src-tauri/build.rs','THIRD_PARTY_NOTICES.md','src/app.ts','src-tauri/src/profile.rs','public/asset.svg','src-tauri/vendor/patch.rs','tools/build.ps1')){Write-TestFile (Join-Path $root $file) ('synthetic '+$file)}
  Write-TestFile (Join-Path $root 'package.json') '{"version":"9.8.7"}'
  Write-TestFile (Join-Path $root 'src-tauri/tauri.conf.json') '{"version":"9.8.7"}'
  Write-TestFile (Join-Path $root 'src-tauri/Cargo.toml') 'version = "9.8.7"'
  Write-TestFile (Join-Path $root '.runtime/approved-tools/LICENSE.txt') 'synthetic license'
  Write-TestFile (Join-Path $root '.runtime/updater-secrets/signing.key') 'synthetic excluded key'
  Write-TestFile (Join-Path $root '.runtime/.env.local') 'synthetic excluded environment'
  Write-TestFile (Join-Path $root '.runtime/profiles/user.json') 'synthetic excluded profile'
  return $root
}
$resources={param($root) [pscustomobject]@{path='.runtime/approved-tools';target='media-tools';manifest=(Get-AtsumiDirectoryManifest (Join-Path $root '.runtime/approved-tools'))}}
$versions={ [pscustomobject][ordered]@{rustc='fixture rustc 1.0';cargo='fixture cargo 1.0';node='fixture node 24';pnpm='fixture pnpm 11';windowsSdk='fixture SDK 10';gitCommit=('a'*40)} }
$exeVersion={param($path) '9.8.7'}
function Write-TestBuild([string]$Attempt,[string]$Version,[switch]$Hardlink) {
  $release=Join-Path $Attempt 'target/x86_64-pc-windows-msvc/release'
  Write-TestFile (Join-Path $release 'atsumi.exe') 'synthetic executable never executed'
  if($Hardlink){New-AtsumiSafeDirectory (Join-Path $release 'deps');New-Item -ItemType HardLink -Path (Join-Path $release 'deps/atsumi.exe') -Target (Join-Path $release 'atsumi.exe')|Out-Null}
  Write-TestFile (Join-Path $release ('bundle/nsis/Atsumi_'+$Version+'_x64-setup.exe')) 'synthetic installer never executed'
  Write-TestFile (Join-Path $release ('bundle/nsis/Atsumi_'+$Version+'_x64-setup.exe.sig')) 'synthetic signature never used for signing'
}
$build={param($attempt,$version) Write-TestBuild $attempt $version -Hardlink;return [int]0}

$root=New-TestRoot
Write-TestFile (Join-Path $root 'src-tauri/target/release/atsumi.exe') 'old same version executable'
$receiptPath=Invoke-AtsumiBuildReceipt $root $build $versions $resources $exeVersion
$proof=Assert-AtsumiBuildReceipt $root $receiptPath $resources
Check ($proof.version -ceq '9.8.7') 'successful fixture build receipt validates'
Check ([IO.File]::ReadAllText((Join-Path $proof.stage 'atsumi.exe')) -ceq 'synthetic executable never executed') 'fresh build captured instead of old same-version output'
Check ([IO.File]::ReadAllText((Join-Path $root 'src-tauri/target/release/atsumi.exe')) -ceq 'old same version executable') 'old Cargo output remains unchanged'
$attempt=Join-Path $root ('.runtime/release-builds/'+$proof.buildId)
$release=Join-Path $attempt 'target/x86_64-pc-windows-msvc/release'
Check ((Test-Path -LiteralPath (Join-Path $release 'atsumi.exe')) -and (Test-Path -LiteralPath (Join-Path $release 'deps/atsumi.exe'))) 'both known Cargo hardlink names preserved'
Reject {Get-AtsumiFileProof (Join-Path $release 'atsumi.exe')} 'generic proof still rejects Cargo hardlink pair'
Check ((Get-AtsumiFileProof (Join-Path $proof.stage 'atsumi.exe')).length -gt 0) 'capture output is a regular single-link file'
$receipt=Read-AtsumiReceiptJson $receiptPath
$paths=@($receipt.inputs.source.files.path)
Check (-not ($paths|Where-Object{$_ -match '(^|/)\.runtime/|\.env|signing\.key|profiles/'})) 'source manifest excludes environment secrets profiles and arbitrary runtime'
Check ($paths -ccontains 'src-tauri/src/profile.rs') 'profile source module is not mistaken for user profile data'
Check ($receipt.inputs.resources[0].manifest.files[0].path -ceq 'LICENSE.txt') 'only explicitly approved runtime resource manifest is included'
Check ($receipt.metadata.rustc -ceq 'fixture rustc 1.0') 'compiler metadata uses fixture callback only'
Reject {Assert-AtsumiBuildReceipt $root '' $resources} 'missing receipt is rejected'
Reject {Assert-AtsumiBuildReceipt $root (Join-Path $root 'absent.json') $resources} 'absent receipt file is rejected'

foreach($file in @('src/app.ts','public/asset.svg','src-tauri/vendor/patch.rs','vite.config.ts','pnpm-lock.yaml','.runtime/approved-tools/LICENSE.txt')){
  $path=Join-Path $root $file;$original=[IO.File]::ReadAllText($path)
  [IO.File]::WriteAllText($path,'changed fixture',$utf8)
  Reject {Assert-AtsumiBuildReceipt $root $receiptPath $resources} ('changed input rejected: '+$file)
  [IO.File]::WriteAllText($path,$original,$utf8)
}
Write-TestFile (Join-Path $root 'src/new.ts') 'added fixture'
Reject {Assert-AtsumiBuildReceipt $root $receiptPath $resources} 'new source after build is rejected'
# Fixture is retained. Use a separate fresh source root instead of deleting it.
$root=New-TestRoot
$receiptPath=Invoke-AtsumiBuildReceipt $root $build $versions $resources $exeVersion
$proof=Assert-AtsumiBuildReceipt $root $receiptPath $resources
[IO.File]::WriteAllText((Join-Path $proof.stage 'atsumi.exe'),'same-version altered executable',$utf8)
Reject {Assert-AtsumiBuildReceipt $root $receiptPath $resources} 'same-version captured executable tamper rejected'

$root=New-TestRoot
$receiptPath=Invoke-AtsumiBuildReceipt $root $build $versions $resources $exeVersion
$changed=Read-AtsumiReceiptJson $receiptPath;$changed.completed=$false
[IO.File]::WriteAllText($receiptPath,($changed|ConvertTo-Json -Depth 16),$utf8)
Reject {Assert-AtsumiBuildReceipt $root $receiptPath $resources} 'unsuccessful receipt flag is rejected'

$root=New-TestRoot
Reject {Invoke-AtsumiBuildReceipt $root {param($a,$v) Write-TestBuild $a $v;return [int]1} $versions $resources $exeVersion} 'failed build rejected even when output exists'
Check (-not (Test-Path -LiteralPath (Join-Path $root '.runtime/release-inputs'))) 'failed build emits no capture or success receipt'
$root=New-TestRoot
Write-TestFile (Join-Path $root 'src-tauri/target/release/atsumi.exe') 'old fixture'
Reject {Invoke-AtsumiBuildReceipt $root {return [int]0} $versions $resources $exeVersion} 'zero-exit without new owned build output cannot adopt old executable'
$root=New-TestRoot
Reject {Invoke-AtsumiBuildReceipt $root $build $versions $resources {return ''}} 'missing executable ProductVersion rejected'
$root=New-TestRoot
Reject {Invoke-AtsumiBuildReceipt $root $build $versions $resources {return '1.2.3'}} 'mismatched executable ProductVersion rejected'
$root=New-TestRoot
[IO.File]::WriteAllText((Join-Path $root 'package.json'),'{"version":"1.2.3"}',$utf8)
Reject {Invoke-AtsumiBuildReceipt $root $build $versions $resources $exeVersion} 'inconsistent source versions reject before build'

$root=New-TestRoot
$changeBuild={param($a,$v) Write-TestBuild $a $v; [IO.File]::WriteAllText((Join-Path $root 'src/app.ts'),'changed during build',$utf8);return [int]0}
Reject {Invoke-AtsumiBuildReceipt $root $changeBuild $versions $resources $exeVersion} 'whole manifest mismatch during build prevents receipt'
Check (-not (Test-Path -LiteralPath (Join-Path $root '.runtime/release-inputs'))) 'concurrent edit failure creates no success stage'
$root=New-TestRoot
$extraLinkBuild={param($a,$v) Write-TestBuild $a $v -Hardlink; $r=Join-Path $a 'target/x86_64-pc-windows-msvc/release'; New-Item -ItemType HardLink -Path (Join-Path $r 'third.exe') -Target (Join-Path $r 'atsumi.exe')|Out-Null;return [int]0}
Reject {Invoke-AtsumiBuildReceipt $root $extraLinkBuild $versions $resources $exeVersion} 'third link on owned build output rejected'
$root=New-TestRoot
$wrongAliasBuild={param($a,$v) Write-TestBuild $a $v; $r=Join-Path $a 'target/x86_64-pc-windows-msvc/release'; New-Item -ItemType HardLink -Path (Join-Path $r 'foreign.exe') -Target (Join-Path $r 'atsumi.exe')|Out-Null; Write-TestFile (Join-Path $r 'deps/atsumi.exe') 'different identity';return [int]0}
Reject {Invoke-AtsumiBuildReceipt $root $wrongAliasBuild $versions $resources $exeVersion} 'same link count with foreign alias identity rejected'
$root=New-TestRoot
Write-TestFile (Join-Path $root 'src/.env.local') 'must not enter manifest'
Reject {Get-AtsumiBuildSourceManifest $root} 'sensitive source filename rejected'
$root=New-TestRoot
$profileSource='src-tauri/src/platform_media/profile'
foreach($name in @('current.rs','snapshot.rs','snapshot/tests.rs')){Write-TestFile (Join-Path $root ($profileSource+'/'+$name)) ('synthetic Rust source '+$name)}
$profileManifest=Get-AtsumiBuildSourceManifest $root
foreach($name in @('current.rs','snapshot.rs','snapshot/tests.rs')){Check (@($profileManifest.files.path) -ccontains ($profileSource+'/'+$name)) ('reviewed source profile module included: '+$name)}
$profileInputs=Get-AtsumiBuildInputs $root $resources
[IO.File]::WriteAllText((Join-Path $root ($profileSource+'/current.rs')),'changed Rust source',$utf8)
Reject {Assert-AtsumiBuildInputsEqual $profileInputs (Get-AtsumiBuildInputs $root $resources)} 'reviewed profile source changes still invalidate build inputs'
$diagnostic=''
try{Assert-AtsumiBuildInputsEqual $profileInputs (Get-AtsumiBuildInputs $root $resources)}catch{$diagnostic=$_.Exception.Message}
Check ($diagnostic.Contains('changed: '+$profileSource+'/current.rs') -and -not $diagnostic.Contains('changed Rust source')) 'mismatch diagnostics identify admitted path without file contents'
foreach($extra in @('extra.rs','account.json','.env.local','signing.key','cookies/session.json','nested/extra.rs','snapshot/extra.rs','snapshot/.env','snapshot/credentials.pem')){
  $root=New-TestRoot
  foreach($name in @('current.rs','snapshot.rs','snapshot/tests.rs')){Write-TestFile (Join-Path $root ($profileSource+'/'+$name)) 'synthetic Rust source'}
  Write-TestFile (Join-Path $root ($profileSource+'/'+$extra)) 'unreviewed fixture'
  Reject {Get-AtsumiBuildSourceManifest $root} ('unreviewed source profile child rejected: '+$extra)
}
$root=New-TestRoot
Write-TestFile (Join-Path $root ($profileSource+'/current.rs')) 'incomplete reviewed module'
Reject {Get-AtsumiBuildSourceManifest $root} 'incomplete source profile inventory rejected'
foreach($directory in @('src/profile','src/profiles','src/cookies','src/secrets','src-tauri/src/other/profile')){
  $root=New-TestRoot
  Write-TestFile (Join-Path $root ($directory+'/current.rs')) 'not an approved module'
  Reject {Get-AtsumiBuildSourceManifest $root} ('other sensitive-named directory rejected: '+$directory)
}
$root=New-TestRoot
foreach($name in @('current.rs','snapshot.rs','snapshot/tests.rs')){Write-TestFile (Join-Path $root ($profileSource+'/'+$name)) 'synthetic Rust source'}
New-Item -ItemType HardLink -Path (Join-Path $root 'src/aliased-profile-source.rs') -Target (Join-Path $root ($profileSource+'/current.rs'))|Out-Null
Reject {Get-AtsumiBuildSourceManifest $root} 'reviewed profile source hardlink still rejected'
$root=New-TestRoot
[IO.File]::WriteAllText((Join-Path $root 'src/app.ts'),"first`r`nsecond`r`n",$utf8)
$crlfInputs=Get-AtsumiBuildInputs $root $resources
[IO.File]::WriteAllText((Join-Path $root 'src/app.ts'),"first`nsecond`n",$utf8)
Reject {Assert-AtsumiBuildInputsEqual $crlfInputs (Get-AtsumiBuildInputs $root $resources)} 'line ending changes remain byte-exact input mismatches'
$root=New-TestRoot
Write-TestFile (Join-Path $root '.env.production') 'implicit private frontend configuration'
Reject {Get-AtsumiBuildSourceManifest $root} 'implicit root environment input rejects instead of hashing or silently omitting'
$root=New-TestRoot
$badMetadata={ [pscustomobject]@{rustc='fixture';cargo='fixture';node='fixture';pnpm='fixture';windowsSdk='fixture';environment='not allowed'} }
Reject {Invoke-AtsumiBuildReceipt $root $build $badMetadata $resources $exeVersion} 'arbitrary environment metadata field rejected'

$root=New-TestRoot
$receiptPath=Invoke-AtsumiBuildReceipt $root $build $versions $resources $exeVersion
$proof=Assert-AtsumiBuildReceipt $root $receiptPath $resources
$payload=Join-Path $root '.runtime/synthetic-portable/Atsumi'
New-AtsumiSafeDirectory $payload -MustCreate
New-AtsumiSafeDirectory (Join-Path $payload 'media-tools') -MustCreate
$exeProof=@($proof.outputs.files|Where-Object{$_.path -ceq 'atsumi.exe'})[0]
Copy-AtsumiVerifiedFile (Join-Path $proof.stage 'atsumi.exe') (Join-Path $payload 'atsumi.exe') $exeProof
Write-TestFile (Join-Path $payload 'atsumi-portable.json') '{"distribution":"portable","version":"9.8.7","packageKind":"portable-updater","updaterCompatible":true}'
Write-TestFile (Join-Path $payload 'Start-Atsumi.cmd') 'synthetic launcher never executed'
Write-TestFile (Join-Path $payload 'Readme.txt') 'synthetic readme'
$manifest=Get-AtsumiDirectoryManifest $payload
$archive=Join-Path ([IO.Path]::GetDirectoryName($payload)) 'Atsumi-Portable.zip'
New-AtsumiPackageArchive $payload $archive $manifest
$zipProof=Get-AtsumiFileProof $archive
$sidecar=[pscustomobject]@{format=1;version='9.8.7';kind='portable-updater';updaterCompatible=$true;archiveName='Atsumi-Portable.zip';bytes=$zipProof.length;sha256=$zipProof.sha256;bundledWebView2=$false;manifest=$manifest;buildId=$proof.buildId;buildReceiptSha256=$proof.receiptSha256}
Write-TestFile ($archive+'.metadata.json') ($sidecar|ConvertTo-Json -Depth 8)
Assert-AtsumiPortableBuildBinding $archive $proof
Check $true 'portable exact executable and receipt binding validates'
$sidecar.buildReceiptSha256='0'*64
[IO.File]::WriteAllText(($archive+'.metadata.json'),($sidecar|ConvertTo-Json -Depth 8),$utf8)
Reject {Assert-AtsumiPortableBuildBinding $archive $proof} 'portable from different build receipt rejected'
Write-TestFile (Join-Path $proof.stage 'unexpected.txt') 'unknown capture'
Reject {Assert-AtsumiBuildReceipt $root $receiptPath $resources} 'unknown capture stage file rejected'

$root=New-TestRoot
$receiptPath=Invoke-AtsumiBuildReceipt $root $build $versions $resources $exeVersion
Move-Item -LiteralPath (Join-Path $root 'src/app.ts') -Destination (Join-Path $root '.runtime/missing-input-preserved.ts')
Reject {Assert-AtsumiBuildReceipt $root $receiptPath $resources} 'missing source input is rejected without deleting its preserved bytes'
$root=New-TestRoot
$receiptPath=Invoke-AtsumiBuildReceipt $root $build $versions $resources $exeVersion
Write-TestFile (Join-Path $root 'src-tauri/tauri.windows.conf.json') '{"build":{}}'
Reject {Assert-AtsumiBuildReceipt $root $receiptPath $resources} 'new implicit Windows Tauri configuration is a changed build input'
$root=New-TestRoot
$receiptPath=Invoke-AtsumiBuildReceipt $root $build $versions $resources $exeVersion
$document=Read-AtsumiReceiptJson $receiptPath
$document|Add-Member -NotePropertyName environment -NotePropertyValue 'must not be published'
[IO.File]::WriteAllText($receiptPath,($document|ConvertTo-Json -Depth 16),$utf8)
Reject {Assert-AtsumiBuildReceipt $root $receiptPath $resources} 'receipt rejects arbitrary additional environment fields'
$root=New-TestRoot
$receiptPath=Invoke-AtsumiBuildReceipt $root $build $versions $resources $exeVersion
$proof=Assert-AtsumiBuildReceipt $root $receiptPath $resources
$lease=Lock-AtsumiPackageDirectories @($proof.stage)
try { Reject {Move-Item -LiteralPath $proof.stage -Destination ($proof.stage+'-renamed') -ErrorAction Stop} 'held capture ancestor cannot be renamed' }
finally {$lease.Dispose()}
$junctionRoot=Join-Path $root '.runtime/junction-fixture'
New-AtsumiSafeDirectory $junctionRoot -MustCreate
New-Item -ItemType Junction -Path (Join-Path $root 'src/junction') -Target $junctionRoot|Out-Null
Reject {Get-AtsumiBuildSourceManifest $root} 'source junction rejected without traversing its target'
$root=New-TestRoot
$receiptPath=Invoke-AtsumiBuildReceipt $root $build $versions $resources $exeVersion
$copy=Join-Path $root '.runtime/wrong-receipt.json'
Copy-AtsumiVerifiedFile $receiptPath $copy (Get-AtsumiFileProof $receiptPath)
Reject {Assert-AtsumiBuildReceipt $root $copy $resources} 'receipt cannot move outside its owned capture stage'

[pscustomobject]@{passed=$script:checks.Count;checks=$script:checks;syntheticOnly=$true;realBuilds=0;toolsExecuted=0;downloads=0;signaturesCreated=0;realPackages=0;deletedFiles=0;fixture=$fixtureRoot}|ConvertTo-Json -Depth 5 -Compress

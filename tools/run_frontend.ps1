param(
  [Parameter(Mandatory = $true, Position = 0)]
  [ValidateSet("dev", "build", "preview", "test", "test-watch", "typecheck", "tauri")]
  [string]$Action,

  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$ExtraArgs
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$bundledNode = Join-Path $env:USERPROFILE ".cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin\node.exe"
$systemNode = Get-Command node -ErrorAction SilentlyContinue
$nodeCandidates = @()

if ($systemNode) {
  $nodeCandidates += $systemNode.Source
}
if (Test-Path -LiteralPath $bundledNode) {
  $nodeCandidates += $bundledNode
}
if ($nodeCandidates.Count -eq 0) {
  throw "Node.js was not found. Install Node.js 22.13+ (LTS) or 24+ or run this project in Codex Desktop."
}

$node = $null
$nodeVersion = $null
$unsupportedNodes = @()
foreach ($candidate in ($nodeCandidates | Select-Object -Unique)) {
  try {
    $candidateVersion = [Version]((& $candidate --version).TrimStart("v").Split("-")[0])
  } catch {
    $unsupportedNodes += "$candidate (version unreadable)"
    continue
  }

  $candidateSupported =
    ($candidateVersion.Major -eq 22 -and $candidateVersion.Minor -ge 13) -or
    $candidateVersion.Major -ge 24
  if ($candidateSupported) {
    $node = $candidate
    $nodeVersion = $candidateVersion
    break
  }
  $unsupportedNodes += "$candidate ($candidateVersion)"
}

if (-not $node) {
  throw "A supported Node.js was not found. Need Node.js 22.13+ (LTS) or 24+. Detected: $($unsupportedNodes -join ', ')"
}

$nodeDirectory = Split-Path -Parent $node
if (($env:Path -split ";") -notcontains $nodeDirectory) {
  $env:Path = "$nodeDirectory;$env:Path"
}

if ($Action -eq "tauri") {
  $systemCargo = Get-Command cargo -ErrorAction SilentlyContinue
  $rustupCargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"

  if ($systemCargo) {
    $cargo = $systemCargo.Source
  } elseif (Test-Path -LiteralPath $rustupCargo) {
    $cargo = $rustupCargo
  } else {
    throw "Rust/Cargo was not found. Install Rust with rustup before running the Tauri desktop app."
  }

  $cargoDirectory = Split-Path -Parent $cargo
  if (($env:Path -split ";") -notcontains $cargoDirectory) {
    $env:Path = "$cargoDirectory;$env:Path"
  }
}

function Invoke-NodeScript {
  param(
    [Parameter(Mandatory = $true)] [string]$RelativePath,
    [string[]]$Arguments = @()
  )

  $entrypoint = Join-Path $projectRoot $RelativePath
  if (-not (Test-Path -LiteralPath $entrypoint)) {
    throw "Missing frontend dependency: $RelativePath. Run pnpm install first."
  }

  & $node $entrypoint @Arguments
  if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
  }
}

Push-Location $projectRoot
try {
  switch ($Action) {
    "dev" {
      Invoke-NodeScript "node_modules\vite\bin\vite.js" @("--host", "127.0.0.1", "--port", "1420")
    }
    "build" {
      Invoke-NodeScript "node_modules\typescript\bin\tsc" @("-b", "--pretty", "false")
      Invoke-NodeScript "node_modules\vite\bin\vite.js" @("build")
    }
    "preview" {
      Invoke-NodeScript "node_modules\vite\bin\vite.js" @("preview", "--host", "127.0.0.1", "--port", "1420")
    }
    "test" {
      Invoke-NodeScript "node_modules\vitest\vitest.mjs" (@("run") + $ExtraArgs)
    }
    "test-watch" {
      Invoke-NodeScript "node_modules\vitest\vitest.mjs" $ExtraArgs
    }
    "typecheck" {
      Invoke-NodeScript "node_modules\typescript\bin\tsc" @("-b", "--pretty", "false")
    }
    "tauri" {
      Invoke-NodeScript "node_modules\@tauri-apps\cli\tauri.js" $ExtraArgs
    }
  }
} finally {
  Pop-Location
}

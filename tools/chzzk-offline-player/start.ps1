$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot
$atsumiDemoNode = (Get-Command node -ErrorAction Stop).Source
& $atsumiDemoNode (Join-Path $PSScriptRoot 'server.mjs')
exit $LASTEXITCODE

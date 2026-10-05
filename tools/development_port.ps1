# Dot-sourced by the dev runners. Never change Windows exclusions or stop a server.
function Get-AtsumiDevelopmentPort {
  param([int[]]$Candidates = (@(1420) + @(14200..14220)))
  foreach ($candidate in $Candidates) {
    if ($candidate -lt 1024 -or $candidate -gt 65535) { throw 'Invalid Atsumi development port.' }
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $candidate)
    try {
      $listener.ExclusiveAddressUse = $true
      $listener.Start()
      return $candidate
    } catch [Net.Sockets.SocketException] {
      if ($_.Exception.SocketErrorCode -notin @('AccessDenied', 'AddressAlreadyInUse')) { throw }
    } finally { $listener.Stop() }
  }
  throw 'No available Atsumi development port. Windows reserved or another app uses all candidates. No existing process was stopped.'
}

function Get-AtsumiConfiguredDevelopmentPort {
  if ($env:ATSUMI_DEV_PORT) {
    $port = 0
    if (-not [int]::TryParse($env:ATSUMI_DEV_PORT, [ref]$port) -or $port -lt 1024 -or $port -gt 65535) {
      throw 'ATSUMI_DEV_PORT must be an integer from 1024 to 65535.'
    }
    return $port
  }
  return Get-AtsumiDevelopmentPort
}

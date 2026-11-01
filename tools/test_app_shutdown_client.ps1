$ErrorActionPreference = 'Stop'
# Local transport fixture only: the fake server always refuses shutdown.
# Neither Atsumi nor the fixture's actual host process is stopped.
Add-Type -AssemblyName System.Net.Http
Add-Type -TypeDefinition @'
using System;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Threading;
public sealed class AtsumiControlFixture : IDisposable {
    private TcpListener listener;
    private Thread worker;
    private int pid;
    public int Port { get; private set; }
    public int Requests;
    public bool Authorized = true;
    public AtsumiControlFixture(int processId) {
        pid = processId;
        listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        Port = ((IPEndPoint)listener.LocalEndpoint).Port;
        worker = new Thread(Run); worker.IsBackground = true; worker.Start();
    }
    private void Run() {
        try {
            for (int i = 0; i < 2; i++) using (TcpClient client = listener.AcceptTcpClient()) {
                client.ReceiveTimeout = 3000;
                using (NetworkStream stream = client.GetStream()) {
                    byte[] buffer = new byte[8192]; int count = 0, header = -1;
                    while (count < buffer.Length) {
                        int size = stream.Read(buffer, count, buffer.Length - count);
                        if (size == 0) return;
                        count += size;
                        string text = Encoding.ASCII.GetString(buffer, 0, count);
                        header = text.IndexOf("\r\n\r\n", StringComparison.Ordinal);
                        if (header < 0) continue;
                        int contentLength = 0;
                        foreach (string line in text.Substring(0, header).Split(new[]{"\r\n"}, StringSplitOptions.None))
                            if (line.StartsWith("Content-Length:", StringComparison.OrdinalIgnoreCase))
                                contentLength = int.Parse(line.Substring(15).Trim());
                        if (count >= header + 4 + contentLength) break;
                    }
                    string request = Encoding.ASCII.GetString(buffer, 0, count);
                    Authorized &= request.IndexOf("Authorization: Bearer " + new string('0', 64), StringComparison.OrdinalIgnoreCase) >= 0;
                    bool status = request.StartsWith("GET /status ", StringComparison.Ordinal);
                    string body = status ? "{\"pid\":" + pid + ",\"health\":{\"epoch\":\"fixture\"}}"
                        : "{\"accepted\":false,\"reason\":\"active_work_confirmation_required\"}";
                    byte[] data = Encoding.UTF8.GetBytes(body);
                    byte[] response = Encoding.ASCII.GetBytes("HTTP/1.1 " + (status ? "200 OK" : "409 Conflict") + "\r\nContent-Type: application/json\r\nContent-Length: " + data.Length + "\r\nConnection: close\r\n\r\n");
                    Interlocked.Increment(ref Requests);
                    stream.Write(response, 0, response.Length); stream.Write(data, 0, data.Length);
                }
            }
        } catch (SocketException) {} catch (ObjectDisposedException) {}
    }
    public void Dispose() { listener.Stop(); worker.Join(1000); }
}
'@
$workspace = Split-Path -Parent $PSScriptRoot
$fixtureRoot = Join-Path $workspace ('.runtime\qa-test-temp\app-control-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
$hostExecutable = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
foreach ($scenario in @('stale_latest', 'missing_latest')) {
    $directory = Join-Path $fixtureRoot $scenario
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $server = [AtsumiControlFixture]::new($PID)
    try {
        $descriptor = @{version=1;pid=$PID;url=('http://127.0.0.1:'+$server.Port);token=('0'*64)}
        [IO.File]::WriteAllText((Join-Path $directory ('control-endpoint-'+$PID+'.json')), ($descriptor|ConvertTo-Json -Compress))
        $latest = Join-Path $directory 'control-endpoint.json'
        if ($scenario -eq 'stale_latest') {
            $stale = @{version=1;pid=2147483647;url='http://127.0.0.1:9';token=('0'*64)}
            [IO.File]::WriteAllText($latest, ($stale|ConvertTo-Json -Compress))
        }
        $output = & (Join-Path $PSScriptRoot 'request_app_shutdown.ps1') -EndpointFile $latest -ExpectedExecutable $hostExecutable
        if ($LASTEXITCODE -ne 2 -or ($output|ConvertFrom-Json).phase -ne 'refused' -or $server.Requests -ne 2 -or -not $server.Authorized) {
            throw ('Control discovery did not reach the current authenticated per-process instance: '+$scenario)
        }
        Write-Output ($scenario+': current per-process descriptor reached; active-work refusal preserved.')
    } finally { $server.Dispose() }
}
Write-Output 'Shutdown client integration checks passed; no process was terminated.'

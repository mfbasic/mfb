param([string]$mode = 'now')
$ErrorActionPreference = 'Stop'
function Step($m) { [Console]::Error.WriteLine("[step] $m") }
$psi = New-Object Diagnostics.ProcessStartInfo 'C:\Users\test\b564p\b564p.exe'
$psi.UseShellExecute = $false; $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true
$srv = [Diagnostics.Process]::Start($psi)
Step 'start'
$port = [int]($srv.StandardOutput.ReadLine().Split('=')[1])
$c = New-Object Net.Sockets.TcpClient('127.0.0.1', $port)
$s = New-Object Net.Security.SslStream($c.GetStream(), $false, ([Net.Security.RemoteCertificateValidationCallback] { $true }))
Step "port $port"
$s.AuthenticateAsClient('127.0.0.1')
Step 'handshake'
$proto = $s.SslProtocol
$b = [Text.Encoding]::ASCII.GetBytes("hello`n"); $s.Write($b); $s.Flush()
Start-Sleep -Milliseconds 200
$s.ShutdownAsync().Wait()   # close_notify, write side only
Step 'shutdown sent'
if ($mode -eq 'exit') { $s.Dispose(); $c.Close(); Step 'tcp closed' }
$t = $srv.StandardOutput.ReadLineAsync(); if (-not $t.Wait(10000)) { Step 'server never reported the close'; $srv.Kill(); exit 3 }
$drained = $t.Result
Step "drained: $drained"
$srv.StandardInput.WriteLine('go'); $srv.StandardInput.Flush()
$total = 0; $peerErr = ''
if ($mode -ne 'exit') {
  $buf = New-Object byte[] 65536
  try { while (($n = $s.Read($buf, 0, $buf.Length)) -gt 0) { $total += $n } } catch { $peerErr = $_.Exception.GetType().Name }
}
$after = $srv.StandardOutput.ReadLine()
$srv.WaitForExit(30000) | Out-Null
"$mode proto=$proto $drained | $after | peer_received=$total $peerErr | srv exit $($srv.ExitCode)"

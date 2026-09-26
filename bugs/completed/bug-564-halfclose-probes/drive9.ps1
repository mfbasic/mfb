param([string]$mode = 'rst')
$psi = New-Object Diagnostics.ProcessStartInfo 'C:\Users\test\b564p\b564p9.exe'
$psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardInput = $true
$srv = [Diagnostics.Process]::Start($psi)
$port = [int]($srv.StandardOutput.ReadLine().Split('=')[1])
$c = New-Object Net.Sockets.TcpClient('127.0.0.1', $port)
$s = New-Object Net.Security.SslStream($c.GetStream(), $false, ([Net.Security.RemoteCertificateValidationCallback] { $true }))
$s.AuthenticateAsClient('127.0.0.1')
if ($mode -eq 'rst') { $c.Client.LingerState = New-Object Net.Sockets.LingerOption($true, 0); $c.Client.Close() }
else { $c.Client.Shutdown([Net.Sockets.SocketShutdown]::Both); $c.Client.Close() }   # FIN, no close_notify
Start-Sleep -Milliseconds 300
$srv.StandardInput.WriteLine('go')
"$mode " + $srv.StandardOutput.ReadLine()
$srv.WaitForExit(10000) | Out-Null

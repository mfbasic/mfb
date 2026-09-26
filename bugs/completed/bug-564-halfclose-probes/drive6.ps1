$ErrorActionPreference = 'Stop'
$psi = New-Object Diagnostics.ProcessStartInfo 'C:\Users\test\b564p\b564p6.exe'
$psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true
$srv = [Diagnostics.Process]::Start($psi)
$port = [int]($srv.StandardOutput.ReadLine().Split('=')[1])
$c = New-Object Net.Sockets.TcpClient('127.0.0.1', $port)
$s = New-Object Net.Security.SslStream($c.GetStream(), $false, ([Net.Security.RemoteCertificateValidationCallback] { $true }))
$s.AuthenticateAsClient('127.0.0.1')
$s.ShutdownAsync().Wait()   # close_notify; stay connected and keep reading
$buf = New-Object byte[] 65536
$reader = $s.ReadAsync($buf, 0, $buf.Length)
$out = $srv.StandardOutput.ReadToEndAsync()
if (-not $srv.WaitForExit(30000)) { $srv.Kill(); 'server hung' }
$out.Result.TrimEnd()
"[exit $($srv.ExitCode)] proto=$($s.SslProtocol)"

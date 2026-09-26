$psi = New-Object Diagnostics.ProcessStartInfo 'C:\Users\test\b564p\b564p8.exe'
$psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true
$srv = [Diagnostics.Process]::Start($psi)
$port = [int]($srv.StandardOutput.ReadLine().Split('=')[1])
$c = New-Object Net.Sockets.TcpClient('127.0.0.1', $port)
$s = New-Object Net.Security.SslStream($c.GetStream(), $false, ([Net.Security.RemoteCertificateValidationCallback] { $true }))
$s.AuthenticateAsClient('127.0.0.1')
$buf = New-Object byte[] 65536; $total = 0
$done = $srv.StandardOutput.ReadLineAsync()
$peak = 0; $r = $null
while (-not $done.IsCompleted) {
  if ($r -eq $null) { $r = $s.ReadAsync($buf, 0, $buf.Length) }
  if ($r.Wait(20)) { $total += $r.Result; $r = $null }
  try { $srv.Refresh(); if ($srv.PeakWorkingSet64 -gt $peak) { $peak = $srv.PeakWorkingSet64 } } catch {}
}
"$($done.Result) peakMB=$([math]::Round($peak/1MB)) peer_bytes_so_far=$total"
$srv.WaitForExit(10000) | Out-Null

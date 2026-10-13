# Run a program in the interactive desktop session and capture the screen at fixed
# offsets with DXGI Desktop Duplication (dd.cs). See README.md.
#
#   run-and-capture.ps1 -Exe C:\path\app.exe -Out C:\path\shots -At 4,12,20
#
# Environment variables set before launching this script (MFB_CANVAS_GPU;
# MFB_CANVAS_STATS, ... -- the latter only for a --debug build) reach the program. The program's stdout goes to
# <Out>\out.txt; each capture is <Out>\shot-<seconds>.png; capture failures are
# appended to <Out>\capture-errors.txt; <Out>\done.txt is written last.
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$Out,
  # Comma-separated seconds, as ONE string: `powershell -File` passes "4,12,20" as a
  # string, and binding that to [int[]] reads the commas as thousands separators
  # (one sleep of 41,220 s).
  [Parameter(Mandatory = $true)][string]$At,
  [int]$WaitSeconds = 60
)
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition (Get-Content -Raw (Join-Path $PSScriptRoot 'dd.cs'))
New-Item -ItemType Directory -Force -Path $Out | Out-Null
Remove-Item (Join-Path $Out '*') -ErrorAction SilentlyContinue
$p = Start-Process -FilePath $Exe -WorkingDirectory (Split-Path $Exe) `
  -RedirectStandardOutput (Join-Path $Out 'out.txt') -PassThru
$start = Get-Date
foreach ($t in ($At.Split(',') | ForEach-Object { [int]$_.Trim() } | Sort-Object)) {
  $wait = $t - ((Get-Date) - $start).TotalSeconds
  if ($wait -gt 0) { Start-Sleep -Milliseconds ([int]($wait * 1000)) }
  try { [DD]::Shot((Join-Path $Out "shot-$t.png")) }
  catch { "$t $_" | Out-File -Append -Encoding ascii (Join-Path $Out 'capture-errors.txt') }
}
$exited = $p.WaitForExit($WaitSeconds * 1000)
"exited=$exited" | Out-File -Encoding ascii (Join-Path $Out 'done.txt')

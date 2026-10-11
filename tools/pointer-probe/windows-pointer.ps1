# windows-pointer.ps1 -- start a canvas program in a real window and drive the pointer
# over it: in, a left press, a drag, a release, a right and a middle click, and out.
#
#   powershell -ExecutionPolicy Bypass -File windows-pointer.ps1 -Exe C:\path\app.exe
#
# Must run INSIDE a logged-in desktop session (from ssh: a scheduled task created with
# /it -- see README.md beside this file). Set MFB_CANVAS_STATS (and MFB_CANVAS_GPU=0)
# in the environment first; the program inherits it. Prints the client origin it found,
# then "done".
#
# Each step holds for 2.5 s: a --debug software frame of a full window takes long
# enough that a shorter hold can fall between two stats lines.
param([Parameter(Mandatory = $true)][string]$Exe)

Add-Type @"
using System; using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
}
"@
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep 5
$p.Refresh()
$h = $p.MainWindowHandle
$o = New-Object W+POINT
[W]::ClientToScreen($h, [ref]$o) | Out-Null
[W]::SetForegroundWindow($h) | Out-Null
"origin $($o.X) $($o.Y) handle $h"
# A point in CLIENT pixels, which are the canvas's surface pixels.
function at($x, $y) { [W]::SetCursorPos($o.X + $x, $o.Y + $y) | Out-Null; [W]::mouse_event(1, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 2500 }
# MOUSEEVENTF_*: 0x2/0x4 left down/up, 0x8/0x10 right, 0x20/0x40 middle.
function ev($f) { [W]::mouse_event($f, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 2500 }
at 300 200
at 310 210
ev 0x2
at 320 220
ev 0x4
ev 0x8
ev 0x10
ev 0x20
ev 0x40
at -60 200
at -70 210
Stop-Process -Id $p.Id -Force
"done"

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
$p = Start-Process -FilePath C:\mfbover\overid.exe -PassThru
Start-Sleep 6
$p.Refresh()
$h = $p.MainWindowHandle
$o = New-Object W+POINT
[W]::ClientToScreen($h, [ref]$o) | Out-Null
[W]::SetForegroundWindow($h) | Out-Null
"origin $($o.X) $($o.Y) handle $h"
function click($x, $y) {
  [W]::SetCursorPos($o.X + $x, $o.Y + $y) | Out-Null
  [W]::mouse_event(1, 0, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 600
  [W]::mouse_event(0x2, 0, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 300
  [W]::mouse_event(0x4, 0, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 600
}
click 70 50
click 190 50
click 310 60
click 200 200
Start-Sleep 1
Start-Sleep 12
"done"

# Drive the --app key probe on Windows (tools/key-probe).
#
#   powershell -ExecutionPolicy Bypass -File drive_windows_app.ps1 `
#       -Exe keyprobe.exe -Out out.txt [-Surface transcript|tui|canvas] [-Console]
#
# -Console drives the CONSOLE probe instead (`keyprobe.exe <out> [tui]`, with
# -Surface tui for term::on) in a new console window. Its keys are written into
# that console's input buffer as KEY_EVENT records (AttachConsole +
# WriteConsoleInputW), which conhost turns into what the program reads —
# including the VT sequences of ENABLE_VIRTUAL_TERMINAL_INPUT.
#
# Must run IN the interactive desktop session (from ssh: a scheduled task
# created with `/it`, see README.md). Launches the probe with KEYPROBE_OUT /
# KEYPROBE_SURFACE in its environment, brings its window to the front and types
# the probe's key sequence with SendInput — real input through the system's
# input queue, TranslateMessage and the window procedure — then prints the
# probe's output file.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Surface = "transcript",
    [switch]$Console
)
$ErrorActionPreference = "Stop"
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class KeyProbeInput {
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT {
        public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT {
        public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit)] public struct UNION {
        [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public UNION u; }
    [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint MapVirtualKeyW(uint code, uint map);
    public const uint EXTENDED = 1, KEYUP = 2, UNICODE = 4;
    [StructLayout(LayoutKind.Explicit, CharSet = CharSet.Unicode)] public struct KEY_EVENT_RECORD {
        [FieldOffset(0)] public int bKeyDown; [FieldOffset(4)] public ushort wRepeatCount;
        [FieldOffset(6)] public ushort wVirtualKeyCode; [FieldOffset(8)] public ushort wVirtualScanCode;
        [FieldOffset(10)] public char UnicodeChar; [FieldOffset(12)] public uint dwControlKeyState; }
    [StructLayout(LayoutKind.Explicit)] public struct INPUT_RECORD {
        [FieldOffset(0)] public ushort EventType; [FieldOffset(4)] public KEY_EVENT_RECORD KeyEvent; }
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool FreeConsole();
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool AttachConsole(uint pid);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool WriteConsoleInputW(IntPtr h, INPUT_RECORD[] recs, uint n, out uint written);
    static IntPtr conin = IntPtr.Zero;
    public static void Attach(uint pid) {
        FreeConsole();
        if (!AttachConsole(pid)) throw new Exception("AttachConsole failed: " + Marshal.GetLastWin32Error());
        conin = CreateFileW("CONIN$", 0xC0000000, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
    }
    public static void ConsoleKey(ushort vk, char ch, uint state, bool down) {
        INPUT_RECORD[] r = new INPUT_RECORD[1];
        r[0].EventType = 1;
        r[0].KeyEvent.bKeyDown = down ? 1 : 0;
        r[0].KeyEvent.wRepeatCount = 1;
        r[0].KeyEvent.wVirtualKeyCode = vk;
        r[0].KeyEvent.wVirtualScanCode = (ushort)MapVirtualKeyW(vk, 0);
        r[0].KeyEvent.UnicodeChar = ch;
        r[0].KeyEvent.dwControlKeyState = state;
        uint n;
        if (!WriteConsoleInputW(conin, r, 1, out n)) throw new Exception("WriteConsoleInputW failed: " + Marshal.GetLastWin32Error());
    }
    public static void Key(ushort vk, ushort scan, uint flags, bool up) {
        INPUT[] i = new INPUT[1];
        i[0].type = 1;
        i[0].u.ki.wVk = vk;
        i[0].u.ki.wScan = scan != 0 ? scan : (ushort)MapVirtualKeyW(vk, 0);
        i[0].u.ki.dwFlags = flags | (up ? KEYUP : 0);
        if (SendInput(1, i, Marshal.SizeOf(typeof(INPUT))) != 1)
            throw new Exception("SendInput failed: " + Marshal.GetLastWin32Error());
    }
}
"@

# (name, virtual key, flags, unicode scan)
$keys = @(
    @("a", 0x41, 0, 0), @("Left", 0x25, 1, 0), @("Right", 0x27, 1, 0),
    @("Up", 0x26, 1, 0), @("Down", 0x28, 1, 0), @("Backspace", 0x08, 0, 0),
    @("Delete", 0x2E, 1, 0), @("Escape", 0x1B, 0, 0), @("F5", 0x74, 0, 0),
    @("Shift+Left", 0x25, 1, 0), @("Home", 0x24, 1, 0), @("End", 0x23, 1, 0),
    @("e-acute", 0, 4, 0xE9), @("q", 0x51, 0, 0)
)

if (Test-Path $Out) { Remove-Item $Out }
$env:KEYPROBE_OUT = $Out
$env:KEYPROBE_SURFACE = $Surface
if ($Console) {
    $probeArgs = @($Out)
    if ($Surface -eq "tui") { $probeArgs += "tui" }
    $proc = Start-Process -FilePath $Exe -ArgumentList $probeArgs -PassThru
} else {
    $proc = Start-Process -FilePath $Exe -PassThru
}
$deadline = (Get-Date).AddSeconds(15)
while (-not (Test-Path $Out) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 100 }
while (-not $Console -and $proc.MainWindowHandle -eq 0 -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 100; $proc.Refresh()
}
Start-Sleep -Milliseconds 800
$shell = New-Object -ComObject WScript.Shell
[void]$shell.AppActivate($proc.Id)
if (-not $Console) {
    [void][KeyProbeInput]::SetForegroundWindow($proc.MainWindowHandle)
}
Start-Sleep -Milliseconds 500
if (-not $Console -and [KeyProbeInput]::GetForegroundWindow() -ne $proc.MainWindowHandle) {
    Write-Output "WARNING: the probe's window is not in the foreground"
}
if ($Console) {
    # (name, virtual key, character, control-key state: 0x10 SHIFT, 0x100 ENHANCED)
    $consoleKeys = @(
        @(0x41, [char]'a', 0), @(0x25, [char]0, 0x100), @(0x27, [char]0, 0x100),
        @(0x26, [char]0, 0x100), @(0x28, [char]0, 0x100), @(0x08, [char]8, 0),
        @(0x2E, [char]0, 0x100), @(0x1B, [char]27, 0), @(0x74, [char]0, 0),
        @(0x25, [char]0, 0x110), @(0x24, [char]0, 0x100), @(0x23, [char]0, 0x100),
        @(0, [char]0xE9, 0), @(0x51, [char]'q', 0)
    )
    [KeyProbeInput]::Attach([uint32]$proc.Id)
    foreach ($k in $consoleKeys) {
        [KeyProbeInput]::ConsoleKey([uint16]$k[0], [char]$k[1], [uint32]$k[2], $true)
        Start-Sleep -Milliseconds 50
        [KeyProbeInput]::ConsoleKey([uint16]$k[0], [char]$k[1], [uint32]$k[2], $false)
        Start-Sleep -Milliseconds 150
    }
    # Back to this script's own output before printing the result.
    [void][KeyProbeInput]::FreeConsole()
    $keys = @()
}
foreach ($k in $keys) {
    $vk = [uint16]$k[1]; $flags = [uint32]$k[2]; $scan = [uint16]$k[3]
    if ($k[0] -eq "Shift+Left") { [KeyProbeInput]::Key(0x10, 0, 0, $false) }
    [KeyProbeInput]::Key($vk, $scan, $flags, $false)
    Start-Sleep -Milliseconds 50
    [KeyProbeInput]::Key($vk, $scan, $flags, $true)
    if ($k[0] -eq "Shift+Left") { [KeyProbeInput]::Key(0x10, 0, 0, $true) }
    Start-Sleep -Milliseconds 150
}
if (-not $proc.WaitForExit(10000)) { $proc.Kill(); Write-Output "probe did not exit" }
Get-Content -Encoding UTF8 $Out

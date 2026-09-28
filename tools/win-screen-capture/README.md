# tools/win-screen-capture

Capture what a Windows display actually shows, for checking a windowed MFBASIC
program on a real GPU (bug-688 V6, box 206 in `.ai/remote_systems.md`).

**Why not a GDI screen grab.** `Graphics.CopyFromScreen` / `BitBlt` from the
screen DC reads GDI's copy of the desktop. Once a Vulkan (or DXGI flip-model)
swapchain presents to a window, DWM can hand its images straight to the display
(independent flip), and the GDI copy then shows something else: measured on box
206, four grabs a second apart during a fullscreen canvas phase returned one
frozen image — the child window's grey background, or an older frame — while
every frame was presenting. `PrintWindow(hwnd, PW_RENDERFULLCONTENT)` is no
better: it rendered only the child's background even in windowed mode.
DXGI Desktop Duplication returns the composed desktop the display is given,
swapchain content included, and matched the readback path's pixels exactly.

- **dd.cs** — `[DD]::Shot(path)`: one Desktop Duplication frame of output 0 of
  the default adapter, saved as a PNG. Plain C# over the COM vtables (no
  SharpDX), loaded with `Add-Type`. It keeps acquiring until a frame carries a
  desktop image — the first frames after `DuplicateOutput` can be empty (all
  black).
- **run-and-capture.ps1** — start a program, capture at the given second
  offsets, wait for it to exit.

## Use

Desktop Duplication needs the interactive session, so launch through a
scheduled task with `/it` (the `test` user must be logged on at the console):

```
scp tools/win-screen-capture/* app.exe run.bat test@192.168.1.206:cap/
ssh test@192.168.1.206 "schtasks /create /tn cap /tr C:\Users\test\cap\run.bat /sc once /st 23:59 /it /f && schtasks /run /tn cap"
```

where `run.bat` sets any environment variables and calls

```
powershell -ExecutionPolicy Bypass -File C:\Users\test\cap\run-and-capture.ps1 -Exe C:\Users\test\cap\app.exe -Out C:\Users\test\cap\shots -At 4,12,20
```

Poll for `shots\done.txt`, then copy `shots\` back. Compare captures by colour
histogram (e.g. PIL `Counter(img.getdata()).most_common()`) against a
readback-path run of the same scene rather than by eye.

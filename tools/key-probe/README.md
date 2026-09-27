# tools/key-probe

The bug-699 key probe: one program that reads keys with `io::readKey` and records
what it saw, plus a driver per platform that types the same key sequence into it
through that platform's real input path. The point is the comparison — every
surface on every platform must report the same keys:

```
PRESSES: a Left Right Up Down Backspace Delete Escape F5 Shift+Left Home EndKey é q
```

## Files

| File | What it is |
|---|---|
| `keys.mfb` | the reading loop (`probeKeys`), shared by both programs |
| `main_console.mfb` | the console program: `keyprobe <out-file> [tui]` (`tui` reads under `term::on`) |
| `main_app.mfb` | the `--app` program: `KEYPROBE_OUT=<out-file> KEYPROBE_SURFACE=transcript\|tui\|canvas` |
| `build.py` | assembles a project from the files above (byte copies) and runs `mfb build` |
| `drive_pty.py` | console driver over a pty: legacy xterm bytes, or `--kitty` to answer `term::on`'s query as a kitty terminal and send kitty-form keys with releases |
| `drive_macos_app.py` | macOS `--app` driver: `CGEventPostToPid` (needs Accessibility trust) |
| `drive_linux_app.sh` | Linux GTK `--app` driver, run on the box: extracted AppImage under a private Xvfb, `xdotool` (XTEST) |
| `drive_windows_app.ps1` | Windows driver, run in the interactive desktop session: `SendInput` for `--app`; `-Console` writes KEY_EVENT records into the console's input buffer (`WriteConsoleInputW`) — read the output file itself afterwards, since the script's own output does not reliably reach a redirect once it has attached to the probe's console |

The `--app` program takes its settings from the environment: `open` and the
drivers' launchers pass an environment more easily than arguments. (It was
written that way while `os::args` crashed in `--app` builds — bug-743, fixed.)

The output file's first line is `PRESSES:` followed by every press and repeat;
each later line is one event: `key event shift|ctrl|alt|meta grapheme-hex`.

## Usage

Build (the compiler is `$MFB`, else this checkout's `target/release/mfb`):

```sh
python3 tools/key-probe/build.py console /tmp/kpc            # + -target linux-aarch64, windows-x86_64, ...
python3 tools/key-probe/build.py app /tmp/kpa                # adds -app
```

Console, macOS or Linux:

```sh
python3 tools/key-probe/drive_pty.py /tmp/kpc/build/keyprobe.out /tmp/out.txt           # reader's own single-key mode
python3 tools/key-probe/drive_pty.py /tmp/kpc/build/keyprobe.out /tmp/out.txt --tui     # term::on, legacy terminal
python3 tools/key-probe/drive_pty.py /tmp/kpc/build/keyprobe.out /tmp/out.txt --kitty   # term::on, kitty terminal
```

`--app` on macOS:

```sh
python3 tools/key-probe/drive_macos_app.py /tmp/kpa/build/keyprobe.app /tmp/out.txt canvas
```

`--app` on a Linux GTK box (copy the AppImage and the script there first):

```sh
sh drive_linux_app.sh ./keyprobe-glibc.AppImage /tmp/out.txt tui
```

Windows (copy the `.exe`s and the script there; from ssh, run it in the logged-in
desktop session with a scheduled task created with `/it` — `.ai/remote_systems.md`
and `/tmp/fixbug-common.md` rule 13 describe box 206):

```bat
powershell -ExecutionPolicy Bypass -File drive_windows_app.ps1 -Exe keyprobe-app.exe -Out out.txt -Surface canvas
powershell -ExecutionPolicy Bypass -File drive_windows_app.ps1 -Exe keyprobe-con.exe -Out out.txt -Surface tui -Console
```

## Results (bug-699 Phase 7, 2026-09-26)

Every run below printed exactly the `PRESSES` line above.

| Platform | Surface | Events |
|---|---|---|
| macOS 15 aarch64 | console: reader mode, `term::on` legacy, `term::on` kitty (pty) | `NotSupported`, `NotSupported`, press + release |
| macOS 15 aarch64 | `--app` transcript, TUI, canvas (`CGEventPostToPid`) | press + release |
| Linux aarch64 glibc, GTK 4 (box 2226) | console: the same three pty runs | as macOS |
| Linux aarch64 glibc, GTK 4 (box 2226) | `--app` transcript, TUI, canvas (Xvfb, `xdotool`) | press + release |
| Windows 11 x86_64 (box 206, real session) | `--app` transcript, TUI, canvas (`SendInput`) | press + release |
| Windows 11 x86_64 (box 206) | console: reader mode and `term::on` (`WriteConsoleInputW`) | `NotSupported` |

Not measured: Linux x86_64 GTK (boxes 2227/2228 refused ssh all session), and a
real kitty-capable terminal emulator (the pty answers as one). Artefacts of the
synthetic input, not of the compiler: macOS reports `é`'s release under the
physical key the synthetic event names (`a`); Windows reports no release for `é`
(`SendInput` with `KEYEVENTF_UNICODE` has no virtual key); `xdotool key
shift+Left` releases Shift before Left, which is why the driver spells that chord
out as downs and ups.

# bug-1015: on GTK, `term::pollMouse` sees no mouse events unless the program also reads stdin

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (platform parity)

Status: Open
Regression Test: (proposed) a GTK app-mode row in `scripts/test-appimage.sh`: `term::enableMouse(TRUE)` + a `term::pollMouse` loop with NO stdin read, driven by xdotool click/move/scroll under Xvfb, asserting at least one event per action

**Owner decision (2026-10-10): GTK mouse events come from the window, like macOS and Windows.**
`term::pollMouse` must return the events the window received, whether or not the program ever
reads stdin.

On Linux GTK `--app` builds the mouse handlers write an SGR mouse report into the terminal input
stream, and the decoder that turns reports into `MouseEvent`s runs only on the stdin read path
(`io::readKey` and friends). A program that polls the mouse without reading stdin never decodes
anything, so `term::pollMouse` returns `MouseKind.None` forever.

**Correct behavior:** the GTK window's press/release/motion/scroll events reach the mouse event
queue directly (or `pollMouse` drains and decodes pending input itself), so the same program
behaves the same on GTK, macOS and Windows. Check how the macOS and Windows backends feed the
queue and make GTK match.

## Failing Reproduction

Measured by the coordinator on 2026-10-10 on box 2226 (Debian 12 aarch64 glibc, GTK4, Xvfb +
xdotool), with bs18-agent-i's compiler (bug-982 fixed, so the app no longer crashes). The program
is bug-982's repro: `term::on`, `term::enableMouse(TRUE)`, then 40 × (`term::pollMouse`, log an
`event` line when the kind is not `None`, `os::sleep(100)`), then `done`. It never reads stdin.

| action | `event` lines | `done` |
|---|---|---|
| `xdotool click 1` | 0 | 1 |
| `xdotool mousemove …` | 0 | 1 |
| `xdotool click 4` (scroll) | 0 | 1 |

bs18-agent-i measured the same program with one `io::readKey(0)` added per loop: 2, 2 and 3 events.
(The coordinator has not re-run that variant.)

Expected: at least one `event` line per action.

## Related

- bug-982 (the crash these handlers used to cause; fixed). bug-985 (a `setMode`-built window
  attaches no mouse controllers at all): fix this one first or together, since both end in
  "pollMouse sees nothing".
- macOS: bs18-agent-i could not measure synthetic mouse delivery on the host at all (no IMP
  reached `write`, `screencapture` returned black), so macOS delivery is unverified, not known good.

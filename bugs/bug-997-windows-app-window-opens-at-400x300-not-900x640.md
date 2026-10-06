# bug-997: Windows `--app` windows open at 400×300 (a 384×261 client) while macOS and Linux open at 900×640

Last updated: 2026-10-06
Effort: small (<1h)
Severity: LOW
Class: Other (Platform parity)

Status: Open
Regression Test: planned `src/target/win_x86_64/app/mod.rs` unit test on the emitted `CreateWindowExW` size (outer size from `AdjustWindowRectEx` of a 900×640 client); box 206 windowed run reading `damage=` from `MFB_CANVAS_STATS`

An `--app` program on Windows opens a 400×300 *outer* window, whose client area is 384×261
on box 206 (Win11, default theme). macOS opens a 900×640 *content* rect and Linux GTK a
900×640 default size. The canvas's own default size is also 900×640. So on Windows only, a
canvas program's first frames have about a sixth of the area, `canvas::getSize()` reports
384×261, and a `term::` grid seeds at 48×16 instead of the ~112×40 the other platforms get.
A typical canvas UI laid out for the documented size overlaps: the canvas-demo's ten-card
menu does. The demo works around it by going fullscreen when the surface is smaller than
720×480 (`examples/canvas-demo/src/main.mfb`, "Windows opens new windows at 400 by 300").
That is a per-program workaround for a runtime default.

**The single correct behavior a fix produces:** a Windows app window opens with a
**900×640 client area**, the same content size macOS and Linux give. The outer size comes
from `AdjustWindowRectEx(WS_OVERLAPPEDWINDOW)` (DPI-aware if the process is) and is clamped
to the monitor's work area when the screen is smaller. The position stays `CW_USEDEFAULT`.

References:

- `mfb spec app macos-runtime` (`src/docs/spec/app/01_macos-runtime.md`): `contentRect = NSMakeRect(100, 100, 900, 640)`
- `mfb spec app linux-runtime` (`src/docs/spec/app/02_linux-runtime.md`): "default size 900×640"
- `mfb spec app term-backend` (`src/docs/spec/app/04_term-backend.md`): "The initial size comes from the 900×640 frame" (macOS); the Windows section says the `WM_SIZE` `CreateWindowExW` sends "seeds a windowed run". It does not state the size.
- No spec topic or man page states the Windows window size (`grep -rn '400' src/docs/spec/app` → none; `mfb man app` → none). The two platforms that are specified agree on 900×640.
- bug-688 (records the 384×261 client while fixing canvas size publication), bug-540 (records `before=48x16` as "the default 400x300 window's client")
- Found while running `examples/canvas-demo` on box 206

## Failing Reproduction

Any canvas program, e.g. bug-996's `coldprobe` (`/tmp/coldprobe`), built
`mfb build --app --debug --target windows-x86_64`, run **windowed** on box 206 through a
`schtasks /it` task with `MFB_CANVAS_SYNC=1` and `MFB_CANVAS_STATS` set:

```
gpuFrames=1  directFrames=1  damage=0,0,384,261
gpuFrames=2  directFrames=2  damage=0,0,384,261
```

- Observed: canvas surface 384×261 (the client of a 400×300 outer window). A desktop capture
  of the canvas-demo in the same state (`/tmp/cdwin/cold/probe.png`, scratch) shows the small window.
- Expected: `damage=0,0,900,640` (`canvas::getSize()` = 900×640), as on macOS and Linux.

Contrast: macOS (`src/target/macos_aarch64/app/bootstrap.rs`, `NSMakeRect(100, 100, 900, 640)`
content rect) and Linux GTK (`src/target/linux_gtk/bootstrap.rs`,
`gtk_window_set_default_size(window, WINDOW_WIDTH, WINDOW_HEIGHT)` with
`src/target/linux_gtk/mod.rs:WINDOW_WIDTH`/`WINDOW_HEIGHT` = 900/640). Headless runs on every
platform use the 900×640 canvas default, which is why no headless test sees this.

## Root Cause

`src/target/win_x86_64/app/mod.rs` (the GUI bootstrap that registers the class and creates
the window, comment `CreateWindowExW(0, &class, &title, style, CW, CW, 400, 300, 0, 0, hInst, 0)`)
hard-codes `nWidth = 400`, `nHeight = 300`. These are outer sizes, so the client is smaller
still. The value dates from `b6642fe62` (2026-07-26, "plan-66-J-2: Win32 app-mode bootstrap
floor"), a minimal bootstrap, and was never revisited. The transcript `EDIT` child is also
created at 400×300 (comment `CreateWindowExW(0, L"EDIT", NULL, EDIT_STYLE, 0, 0, 400, 300, …)`),
but `WM_SIZE` refits it, so only the top-level size matters. Every size-dependent initial
state on Windows follows from the client: the canvas size published on entering canvas mode
(bug-688 V6, the `GetClientRect` publish), the `term::` grid from `emit_term_resize`, and the
swapchain extent.

## Goal

- Windowed run on box 206: first stats line `damage=0,0,900,640`; `canvas::getSize()` 900×640 before any user resize.
- `term::` initial grid on Windows equals `floor(900/8) × floor(640/16)` = 112×40 (the backend's cell size), with the documented clamps.
- On a work area smaller than the window, the window fits the work area (no off-screen title bar).

### Non-goals (must NOT change)

- Window position (`CW_USEDEFAULT`), style, class, title, fullscreen behaviour.
- The headless 900×640 canvas default and the headless scripted-resize path (`TERM_RESIZE_W_ENV_SYM`).
- macOS and Linux sizes.
- Do NOT make it 900×640 *outer*. The other platforms specify the content size.

## Blast Radius

`grep -n '"400"\|"300"' src/target/win_x86_64/app/mod.rs`:

- top-level `CreateWindowExW` width/height: fixed by this bug.
- transcript `EDIT` child `CreateWindowExW` 400/300: harmless (`WM_SIZE` refits it). Set it to the client size for consistency, or leave it with a comment.
- `CreateFontW(… FW_NORMAL=400 …)`: unrelated (font weight).
- Goldens: the 5 `tests/syntax/app/*/golden/*.windows-x86_64.app.ncodesum` encode the emitted code. They move by exactly this immediate (and an `AdjustWindowRectEx` import/call if added). Re-baseline only those, after the full suite.
- Spec: add the Windows size to `04_term-backend.md`'s Windows section (or a Windows runtime topic) so all three platforms state it.
- `examples/canvas-demo/src/main.mfb`: its fullscreen-if-small workaround comment becomes stale. Keep the guard (small screens exist) but fix the comment.

## Fix Design

Compute the outer rect from a 900×640 client with `AdjustWindowRectEx(&rect,
WS_OVERLAPPEDWINDOW, FALSE, 0)` (user32, already linked), then clamp to
`SystemParametersInfoW(SPI_GETWORKAREA)` or the monitor work area. Pass the resulting
width/height to `CreateWindowExW`. Share 900/640 as named constants with the Linux
`WINDOW_WIDTH`/`WINDOW_HEIGHT` (and macOS's literal) if a shared seam exists. Otherwise
name them locally and cite the other two.

Rejected: hard-coding a fixed outer size such as 916×679. Chrome size varies with theme
and DPI.

## Phases

### Phase 1 — failing test + audit

- [ ] Unit test in `src/target/win_x86_64/app/mod.rs` tests: the bootstrap's `CreateWindowExW` size comes from a 900×640 client (`AdjustWindowRectEx` is called with it). Fails at HEAD (400/300 immediates).
- [ ] Record the box 206 windowed `damage=0,0,384,261` row here (done above), and the term grid via `term::getSize` in a windowed run.

Acceptance: the unit test fails for the documented reason.
Commit: —

### Phase 2 — the fix

- [ ] Replace the 400/300 immediates with the adjusted, clamped outer size; tidy the `EDIT` child size.
- [ ] Spec: state the Windows initial client size beside macOS/Linux.

Acceptance: unit test passes; box 206 windowed run reports `damage=0,0,900,640`.
Commit: —

### Phase 3 — regenerate + validate

- [ ] Full suite, then re-baseline only the 5 Windows `.ncodesum` goldens; diff shows only this change.
- [ ] Box 206: canvas-demo windowed, the menu fits without overlap (`tools/win-screen-capture`); `term::` grid at 112×40.

Acceptance: full suite green; golden delta is exactly the size change.
Commit: —

## Validation Plan

- Regression test(s): `win_x86_64/app` unit test on the emitted size.
- Runtime proof: box 206 windowed stats line `damage=0,0,900,640` and a Desktop Duplication capture.
- Doc sync: `mfb spec app term-backend` (Windows section) or a Windows runtime topic.
- Full suite: `cargo test`.

## Open Decisions

- DPI: at 150% scaling, should 900×640 be logical (scaled) or physical pixels? Recommended: match what the process's DPI awareness already does for the canvas size (physical client pixels today), and record the choice in the spec.

## Summary

A one-constant fix with an `AdjustWindowRectEx` call. The only real risk is the golden
re-baseline, which must show nothing but the size.

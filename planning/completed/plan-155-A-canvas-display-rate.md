# plan-155-A: the display refresh rate, published by the main thread

Last updated: 2026-09-24
Overall Effort: x-large (1d–3d)
Effort: large (3h–1d)
Depends on: nothing

Plan-155 adds two public canvas members:

- `canvas::getFrameStats() AS canvas::FrameStats` — how often the program
  presents, how often the backend draws, and how fast the display refreshes.
- `canvas::waitFrame()` — paces an animation loop to the display without a
  hand-tuned `os::sleep`.

`canvas::present` stays non-blocking. That was decided in discussion (§3,
rejected alternatives).

Both members need one fact the runtime does not have today: **the refresh
rate of the monitor the window is on.** This sub-plan gets that number. Each
platform's main thread publishes it into the process-global graphics state,
the same way it already publishes the surface size, and republishes it when
the window moves to another monitor. The number is only visible internally, on
the `--debug` stats line and through an internal builtin. B and C are what make
it public.

The three sub-plans run in letter order:

| Sub-plan | Effort | Scope |
|---|---|---|
| A (this) | large | display rate on macOS, GTK and Windows; the four stale "vsync" doc claims |
| B | large | `canvas::FrameStats`, `canvas::Renderer`, `canvas::getFrameStats()` |
| C | medium | `canvas::waitFrame()`, wind adopts `present` + `waitFrame` |

A goes first because it has the most **design uncertainty**. It uses three
platform APIs this codebase has never called, each needs a screen-change hook,
and the headless test boxes may not report a real rate at all. B and C only do
arithmetic on the number and wait on it.

References:

- `.ai/canvas-threading.md`:
  - §1, thread ownership. The main thread owns the window.
  - §5, the resize handshake. The display rate is published the same way.
  - §11, test affordances.
- `mfb spec app canvas` (`src/docs/spec/app/06_canvas.md`), "Retained, not immediate".
- `.ai/arch-abi.md` — Win64 and macOS AArch64 calling traps for the new external calls.
- `.ai/remote_systems.md` — box 2226 (Debian 12 GTK), box 2230 (Win11 x86_64).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Nothing in `src` reads a display refresh rate yet | `grep -rniE "maximumFramesPerSecond\|gdk_monitor_get_refresh_rate\|EnumDisplaySettings\|dmDisplayFrequency" src` → no matches | MET (2026-09-24, re-run at start of /follow-plan: no output) |
| The graphics state ends at 808 bytes | `grep -n "GRAPHICS_STATE_SIZE: usize" src/codegen/runtime/canvas/mod.rs` → `= 808` | MET (2026-09-24, re-run: `321:pub(crate) const GRAPHICS_STATE_SIZE: usize = 808;`) |

Everything below assumes both hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* rows.

## 1. Goal

- On macOS, GTK and Windows, the process-global graphics state holds the
  refresh rate of the window's current monitor, in millihertz. It is `0` when
  the OS gives no answer (headless, or an API that isn't available).
- The value is republished when the window moves to a monitor with a different
  rate.
- `MFB_CANVAS_DISPLAY_HZ=<hz>` overrides the value on every platform. This is
  a test affordance, like `MFB_CANVAS_RESIZE_W`.
- The `--debug` stats line (`MFB_CANVAS_STATS`) reports `displayHz=`.
- The four doc sites that say the canvas renders "on vsync" are corrected.

### Non-goals (explicit constraints)

- **No public API in this sub-plan.** Nothing is added to `mfb man canvas`.
  The internal builtin is `internal_only: true`.
- **No new redraw trigger.** A monitor change publishes a number. It does not
  signal a redraw. `.ai/canvas-threading.md` §4's five triggers stay five.
- **No vsync.** No backend changes how or when it presents.
- **No existing field moves.** New `GRAPHICS_OFFSET_*` words are appended
  past `GRAPHICS_OFFSET_MTL_LAYER_SHOW` (800), and `GRAPHICS_STATE_SIZE` grows.
  Neither of the two unnamed gaps (136–199 and 240–295) is reused, because
  nothing rules out a bare-number offset that already uses them.

## 2. Current State

**The surface size is the precedent.**
`src/codegen/runtime/canvas/mod.rs:emit_publish_surface_size` writes
`GRAPHICS_OFFSET_WIDTH` and `GRAPHICS_OFFSET_HEIGHT` with plain `store_u64`s
and no lock. It bumps `GRAPHICS_OFFSET_RESIZES` only when the size changed.
Its callers:

- **macOS:** `src/target/macos_aarch64/app/bootstrap.rs:emit_canvas_set_frame_size_helper`,
  the `MFBCanvasView setFrameSize:` IMP.
- **GTK:** `src/target/linux_gtk/bootstrap.rs:emit_canvas_resize_helper`,
  on the GtkDrawingArea `"resize"` signal.
- **Windows:** `src/target/win_x86_64/app/mod.rs:emit_wndproc`, the
  `WM_SIZE` arm, plus a headless caller in `emit_main`.

**No screen-change hook exists on any platform.** The closest macOS
precedents:

- `src/target/macos_aarch64/app/metal.rs:emit_metal_layer_apply` re-reads
  `[[window screen] colorSpace]` on every apply.
- `src/target/macos_aarch64/app/window.rs:emit_window_observe` shows how to
  add an `NSNotificationCenter` observer (`SEL_ADD_OBSERVER`,
  `NS_WINDOW_DID_ENTER_FULL_SCREEN`).

**How to declare a new import:**

- **macOS:** selectors are `(symbol, name)` tuples, e.g. `metal.rs:SEL_SCREEN`.
  They are listed in `metal.rs:metal_data_objects` or
  `app/mod.rs:app_mode_reconcile_data_objects`, and resolved by
  `Asm::load_selector`.
- **GTK:** `(GTK, "symbol")` pairs in
  `src/target/linux_gtk/mod.rs:app_mode_imports`. `GTK` is `libgtk-4.so.1`.
- **Windows:** `import("…", USER32, "_main")` in
  `src/target/win_x86_64/plan.rs:app_mode_imports`, called with
  `call_external`.

**The internal-builtin precedent.**
`src/codegen/builtins/canvas/func_graphics.rs` registers `surfaceWidth` and
`surfaceHeight` as internal Integer builtins over `emit_surface_dimension`.
`displayMilliHz` copies that shape.

**The stats line.**
`src/codegen/builtins/canvas/helper_surface.rs` builds the `--debug`
`MFB_CANVAS_STATS` line. `displayHz=` is appended there.

**The stale "vsync" claims.** The canvas redraws only on its five triggers,
and time is not one of them (`.ai/canvas-threading.md` §4). The platform
survey found no code that waits for the display: GTK and Windows software
blits post to the UI thread, and Vulkan renders offscreen and reads back. The
Metal path calls `nextDrawable`, but no code sets `displaySyncEnabled`. So
these sentences are wrong:

- `src/docs/spec/app/06_canvas.md:12` ("on vsync, on resize, on damage")
- `src/codegen/builtins/canvas/mod.rs:6` (module doc comment)
- `src/codegen/builtins/canvas/mod.rs:139` (`MODULE_DESC`, the man page)
- `src/codegen/builtins/canvas/func_present.rs:12` (`present`'s man page)

`helper_damage.rs:13` also says "on every vsync", but there it describes a
*program's* loop, so it is accurate and stays.

### Measured populations

| What | Count | Command |
|---|---|---|
| callers of `emit_publish_surface_size` | 4 | `grep -rn "emit_publish_surface_size(" src \| grep -v "fn emit" \| wc -l` |
| "vsync" occurrences in canvas code and docs | 5 (4 wrong, 1 accurate) | `grep -rn "vsync" src/codegen/builtins/canvas src/docs` |
| graphics-state size | 808 | `grep -n "GRAPHICS_STATE_SIZE: usize" src/codegen/runtime/canvas/mod.rs` |

### Verified properties

- **The graphics state is a zero-filled data object**, emitted at
  `runtime/canvas/mod.rs:graphics_state_data_object` (`"00".repeat(GRAPHICS_STATE_SIZE)`). A new word therefore starts
  at 0, which already means "unknown". This was read, not assumed.
- **The main thread's size stores are plain, with no lock.** Read in
  `emit_publish_surface_size`. A single aligned 64-bit store needs no lock
  for a reader that only wants some recent value, and the display rate is
  that kind of value.
- UNVERIFIED: **the minimum macOS version.** `NSScreen.maximumFramesPerSecond`
  needs macOS 12. The query must be guarded with `respondsToSelector:` and
  answer 0 when it is missing. Phase 1 finds the deployment target
  (`grep -rn "minos\|LSMinimumSystemVersion" src/target/macos_aarch64`).
- UNVERIFIED: **whether any test box reports a real rate.** Headless runs
  probably report 0. Phase 1 settles, per platform, how a real value can be
  observed.
- UNVERIFIED: **whether Windows `WM_SIZE` should signal a redraw.** The other
  two platforms signal one after publishing the size; Windows does not. It is
  out of this plan's path because the display rate triggers no redraw, but
  Phase 1 reads the wndproc and settles it. If it is a missed repaint, it is
  filed with `/write-bug` and fixed, not left.

## 3. Design Overview

**Storage.** Append two words to the graphics state:

- `GRAPHICS_OFFSET_DISPLAY_MHZ` (808) — written by the main thread only.
- `GRAPHICS_OFFSET_DISPLAY_OVERRIDE` (816) — written once by the worker from
  `MFB_CANVAS_DISPLAY_HZ`.

`GRAPHICS_STATE_SIZE` becomes 824. Readers use the override when it is
non-zero.

- **Millihertz, not hertz:** GTK answers in mHz, and 59.94 Hz displays exist.
- **The override is a separate word:** the platform write can then never
  clobber it.

**Publishing.** Add `emit_publish_display_rate(mhz_reg)` beside
`emit_publish_surface_size`. It does a plain `store_u64`. Each platform calls
it at the two moments below:

| Platform | Query | When |
|---|---|---|
| macOS | `[[window screen] maximumFramesPerSecond]` × 1000, only if the screen `respondsToSelector:` it | surface ready, and on `NSWindowDidChangeScreenNotification` (observer as in `emit_window_observe`) |
| GTK4 | `gdk_monitor_get_refresh_rate(gdk_display_get_monitor_at_surface(display, surface))` (already mHz; 0 means unknown) | widget realize, and the `GdkSurface` `"enter-monitor"` signal |
| Windows | `MonitorFromWindow` → `GetMonitorInfoW` (device name) → `EnumDisplaySettingsW(name, ENUM_CURRENT_SETTINGS)`. The value is `dmDisplayFrequency` × 1000; a frequency of 0 or 1 means "hardware default" and publishes 0 | `WM_CREATE`/first show, `WM_DISPLAYCHANGE`, and `WM_MOVE` when `MonitorFromWindow` returns a different handle from the last one |

**Reading.** An internal `canvas::displayMilliHz() AS Integer` returns the
override if it is set, otherwise the published word. B and C use it.

**The override.** `__canvas_ensureGraphics` (`helper_render.rs`) already reads
`MFB_CANVAS_SYNC` and `MFB_CANVAS_GPU` there. It reads
`MFB_CANVAS_DISPLAY_HZ` the same way and stores `hz × 1000` through an
internal `canvas::setDisplayOverride(mhz)`.

**Where correctness risk concentrates:**

- **The Windows `WM_MOVE` check.** It runs on every move, so the
  `MonitorFromWindow` comparison must stay cheap. The last monitor handle is
  kept in a platform static, not in the graphics state.
- **Win64 shadow space and alignment** on four new calls (`.ai/arch-abi.md`).

**Where design uncertainty concentrates:** whether each API returns a real
value on the machines we can reach. That is Phase 1.

**Gate class:** new behavior on a new code path. There is no byte-identity
gate:

- Every app build gains the new publish code, so the canvas app `.ncode` is
  expected to diff on all three platforms.
- `tests/syntax/app/app-mouse-surface` is the canvas change sentinel
  (`.ai/testing-gates.md`). Its diff is expected. Re-baseline only the lines
  that come from the new publish calls, after inspecting them.

**Rejected alternatives:**

- **The graphics thread queries the rate per frame.** That breaks §1: only the
  main thread touches the window. It would also cost an OS call per frame.
- **CVDisplayLink / CADisplayLink.** It gives a real vsync callback, but that
  is a pacing mechanism, which this plan rejects (C, §3). It is also more API
  than one number needs.
- **Hertz as a Float word.** The rest of the graphics state is integers, and a
  plain `store_u64` of mHz keeps the store a single aligned write.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; strike moot tasks with evidence; fill
> `Commit:` the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — probes, docs fix, storage and override (uncertainty first)

- [x] Probe each API outside the compiler, with a scratch program in `/tmp`,
      not `scripts/`:
      - macOS: Swift/ObjC, `NSScreen.main!.maximumFramesPerSecond`.
      - box 2226: C, GTK4, `gdk_monitor_get_refresh_rate`.
      - box 2230: C, `EnumDisplaySettingsW`.
      
      Record each value, and whether headless or ssh sessions answer 0, in
      Corrections. This decides how the Phase 2/3 runtime proofs are observed.
      → macOS 120, 2226 74993 mHz over plain ssh, 2230 60 Hz over ssh (Corrections 1).
- [x] Find the macOS deployment target, and record whether the
      `respondsToSelector:` guard can ever be false. → minos 11.0
      (`src/os/macos/link/commands.rs:build_version`, `11 << 16`), so yes: the guard
      stays (Corrections 2).
- [x] Read the Windows `WM_SIZE` arm and settle whether its missing
      `signalRedraw` is a bug (§2). If it is, `/write-bug` it and fix it
      before Phase 3. → a bug, and a second one beside it: bug-698, fixed in
      `13f3ea204` (Corrections 3).
- [x] Fix the four "vsync" sentences listed in §2. Say what actually
      happens: the scene is redrawn when it changes, on resize, and on OS
      damage; the canvas does not wait for the display. The man text follows
      `.ai/man-content.md` (no internals); the spec sentence may name the
      triggers. → `mfb man canvas | grep -c vsync` → 0; `mfb spec app canvas | grep -c vsync` → 0.
- [x] `src/codegen/runtime/canvas/mod.rs`: add `GRAPHICS_OFFSET_DISPLAY_MHZ`
      (808) and `GRAPHICS_OFFSET_DISPLAY_OVERRIDE` (816), each with a doc
      comment in the style of `GRAPHICS_OFFSET_SYNC`; set
      `GRAPHICS_STATE_SIZE` to 824; add `emit_publish_display_rate`.
- [x] `func_graphics.rs`: add internal `displayMilliHz() AS Integer`
      (override first) and `setDisplayOverride(mhz AS Integer)`.
      `helper_render.rs:__canvas_ensureGraphics` reads
      `MFB_CANVAS_DISPLAY_HZ`.
- [x] (added) List `canvas.displayMilliHz` / `canvas.setDisplayOverride` as supported
      runtime calls in `src/target/{macos_aarch64,linux_common,win_x86_64}/mod.rs` —
      without it the first build failed "native backend does not support runtime call
      'canvas.displayMilliHz'" (Corrections 4).
- [x] `helper_surface.rs`: append `displayHz=<mHz/1000 as Float>` to the stats
      line.
- [x] Test, in `tests/canvas/rt_canvas_graphics_thread.rs`:
      `display_hz_override_reaches_the_stats_line` runs a headless present
      with `MFB_CANVAS_DISPLAY_HZ=144` and checks the stats line has
      `displayHz=144`. → `cargo test --test rt_canvas_graphics_thread display_hz` →
      1 passed (the field renders `144.00`; the test parses it as a number).
- [x] (added) Re-baseline the canvas sentinel for Phase 1's own change: `.ir` (line
      shifts in the synthesized `builtins/canvas.mfb`), `.nir`/`.nplan` (the
      `setDisplayOverride` call and its two string literals), and the four
      `.app.ncodesum` (the same, plus `_mfb_str_N` renumbering). No other golden embeds
      the canvas builtins (`grep -rl "builtins/canvas.mfb" tests`). `cargo test --bin mfb
      canvas` → 92 passed.
- [x] (added) `.ai/canvas-threading.md` §11: `MFB_CANVAS_DISPLAY_HZ`.

Acceptance: the override test passes on this Mac; the four sentences are
fixed and render.
  Check: `cargo test --test rt_canvas_graphics_thread display_hz` → 1 passed (est. 3 min);
  `mfb man canvas | grep -c vsync` → 0 and `mfb spec app canvas | grep -c vsync` → 0 (est. 1 min).
  Verified: 1 passed; 0; 0.
Commit: `16a90c4e4` (bug-698: `13f3ea204`)

### Phase 2 — macOS publish

- [x] `metal.rs` / `bootstrap.rs`: add the `maximumFramesPerSecond` and
      `respondsToSelector:` selectors. Publish when the surface is ready. → a new
      `app/display.rs` (selectors, `_mfb_macapp_display_rate`, the IMP), called from
      `emit_reconcile_canvas_helper` on every canvas entry; imports on the canvas arm
      of `plan.rs` (Corrections 6).
- [x] `window.rs`: observe `NSWindowDidChangeScreenNotification` and
      republish from the observer. → observed from the canvas view (method
      `mfbScreenChanged:` on `MFBCanvasView`), not `window.rs`, whose observers exist
      only for a window-member program (Corrections 6). Pinned by
      `the_canvas_reconcile_publishes_and_observes_only_for_a_canvas_program`,
      `the_rate_send_is_guarded_and_published`,
      `the_screen_change_imp_republishes_for_the_notifying_window`
      (`cargo test --bin mfb macos_aarch64::app` → 36 passed).
- [x] Runtime proof: build a non-headless `--debug` canvas app, run it with
      `MFB_CANVAS_STATS=/tmp/s.txt`, and read `displayHz=` off the stats line.
      It must match the Phase 1 probe (120 on a ProMotion panel, 60 on an
      external monitor if one is attached). Record the observed line.
      → `/tmp/p155/win` (present, sleep 1.5 s, present), run without any HEADLESS
      variable: two lines, `directFrames=1 … displayHz=120.00` and `directFrames=2 …
      displayHz=120.00` (`directFrames` > 0 proves a real window). No external monitor
      is attached, so the screen-change path was not driven at runtime.
- [x] (added) The `canvas_view_overrides_the_two_methods_that_deliver_keys` count
      `class_addMethod == 3` → 4, and `mfbScreenChanged:` added to its selector list
      (Corrections 7). Goldens: macOS `.app.ncodesum` of the sentinel and of
      `app-window-surface` (the seven selector strings, nothing else — localized by
      `.ncode` diff), the sentinel's macOS `.nplan` (the three imports per canvas-arm
      call). `rt_canvas_graphics_thread` → 12 passed, 2 ignored (every headless canvas
      run now executes the publish in the reconcile).
- [x] (added) `.ai/canvas-threading.md` §5: the display-rate paragraph.

Acceptance: a real, windowed macOS run reports the probed rate.
  Check: the runtime proof above → `displayHz=` equals the probe (est. 5 min;
  it needs a real window, which is why a headless test can't stand in).
  Verified: `displayHz=120.00` = the probe's 120.
Commit: `1e6d26581`

### Phase 3 — GTK and Windows publish

- [x] `linux_gtk/mod.rs` imports plus `bootstrap.rs`: publish on realize and
      on `"enter-monitor"`. → published where the canvas reconcile reads the window's
      `GdkSurface` (the first point one exists, every canvas entry), with
      `enter-monitor` connected once per surface via a new `ST_DISPLAY_HOOKED` slot
      (Corrections 8). Pinned by
      `the_canvas_reconcile_publishes_the_display_rate_after_reading_the_surface`,
      `the_display_rate_follows_the_surface_across_monitors`.
- [x] `win_x86_64/plan.rs` imports plus `app/mod.rs:emit_wndproc`: publish on
      create, `WM_DISPLAYCHANGE`, and a `WM_MOVE` whose monitor changed. → "create" is
      the canvas reconcile arm, after the window is shown and its HWND published;
      `_mfb_winapp_display_rate(hwnd, force)` keeps the last `HMONITOR` in
      `_mfb_winapp_display_monitor`. Pinned by
      `the_display_rate_is_published_on_canvas_entry_and_followed_by_the_wndproc`,
      `the_display_rate_helper_asks_the_monitor_then_its_mode`; the existing
      `canvas_hwnd_is_published_by_one_arm_and_cleared_by_the_others` count 12 → 14
      (Corrections 9). `cargo test --bin mfb target::` → 219 passed.
- [x] Runtime proof on 2226 and 2230, observed the way Phase 1 found possible.
      If a box can only run headless, the proof is `displayHz=0` there plus
      the Phase 1 probe showing the API answers 0 in that session. Record
      which it was. → both windowed, both real values (the probe program, `/tmp/p155/win`:
      present, sleep 12 s, present):
      - 2226 (GNOME Wayland, the glibc AppImage extracted, no HEADLESS variable):
        `frames=1 displayHz=0.00`, `frames=2 displayHz=74.99`, `frames=3 displayHz=74.99`
        — probe 74993 mHz. The first frame is drawn before the compositor maps the
        window, so GDK knows no monitor yet; the value arrives through `enter-monitor`.
      - 2230 (Win11, over ssh, windowed): `frames=1 displayHz=60.00`,
        `frames=2 displayHz=60.00` — probe 60 Hz. rc=0 — the first windowed run of the
        bug-698 WndProc.
- [x] Re-baseline `tests/syntax/app/app-mouse-surface`, only the lines the
      publish calls add (inspect one diff first, per `.ai/testing-gates.md`). → and the
      other app fixtures, whose Linux/Windows sums move for the unconditional imports:
      localized on `macos-app-mode-term` (Windows: three imports + the monitor static;
      linux-aarch64: three imports, the `enter-monitor` string, the state block one
      word larger). 11 sums rewritten; `scripts/test-accept.sh … 'app-*'
      'macos-app-mode-*' 'app_*'` → passed (9 tests).
- [x] (added) `emit_publish_display_rate` takes `impl Into<Operand>`: the Windows
      helper passes Win64 role tokens, and `to_string()` on one produced
      `unknown x86-64 register '%argMFB2'` at the first Windows build (Corrections 10).

Acceptance: both platforms build and run a canvas app and report the value
their session allows. Verified: 2226 `displayHz=74.99`, 2230 `displayHz=60.00`.
  Check: `scripts/linux-runtime-proof.sh` on box 2226 running the same `--debug`
  canvas app → `displayHz=` line present (est. 8 min: box 2223 has no GTK, so
  the GTK box is the smallest one that exercises the realize hook);
  on box 2230, the Windows build of the same app → `displayHz=` line present (est. 10 min).
Commit: `535b6bdd5`

## Validation Plan

- Tests: `display_hz_override_reaches_the_stats_line`; the sentinel's
  re-baselined lines.
- Coverage check: `grep -rn "displayMilliHz\|DISPLAY_MHZ" src tests` → the
  emitter, the three platforms, the builtin, the stats line, and the test.
- Runtime proof: the Phase 2 and Phase 3 stats lines.
- Doc sync: `.ai/canvas-threading.md` §5 gets a paragraph ("The display rate
  is published the same way, and triggers no redraw"); §11 gets
  `MFB_CANVAS_DISPLAY_HZ`. The four "vsync" fixes.
- Final gate: once, at the end of plan-155-C.

## Open Decisions

- **The windowed rate on a ProMotion Mac is the maximum (120), not the
  current adaptive rate.** Recommended: publish the maximum. The OS lowers
  the rate for static content, and C's floor wants the fastest rate the
  display can take, not a moment's throttled one.
- **Several monitors on GTK and Windows:** use the monitor with the largest
  overlap (what both APIs return). Recommended as-is; nothing better is
  observable.

## Corrections

1. **Probe results (Phase 1).** All three answer a real rate in the session a proof
   can reach, so no platform needs the "headless → 0" fallback proof:
   - macOS (this Mac, 15.7.7): `NSScreen.maximumFramesPerSecond` → `Built-in Retina
     Display 120` (`swiftc /tmp/p155/hz.swift`).
   - box 2226 (Debian 12 GTK4, aarch64): `gdk_monitor_get_refresh_rate` → **74993** mHz,
     via Python `ctypes` against `libgtk-4.so.1` (the box has no C compiler and no
     `pkg-config`). It answers the same over **plain ssh** with no `WAYLAND_DISPLAY` set —
     GTK finds the live GNOME session's `wayland-0` through `XDG_RUNTIME_DIR`.
   - box 2230 (Win11): `EnumDisplaySettingsW(NULL, ENUM_CURRENT_SETTINGS)` →
     `dmDisplayFrequency` **60**, 1024x768, over ssh (PowerShell `Add-Type` P/Invoke;
     `dmDisplayFrequency` at byte offset **184** of `DEVMODEW`, whose full size is 220).
2. **macOS deployment target is 11.0** (`build_version` writes `minos = 11 << 16`), and
   `maximumFramesPerSecond` is macOS 12+, so the `respondsToSelector:` guard is live
   code, not a formality.
3. **Windows `WM_SIZE` — two bugs, not one (bug-698, `13f3ea204`).** The missing
   `signalRedraw` was a missed repaint: a resized static scene kept its old-size frame.
   Reading the arm also showed WndProc wrote `rbx/rsi/rdi/rbp/r12/r13` (the SCRATCH pool's
   x86 realizations, all Win64-nonvolatile) with `"calleeSaved": []` — every arm, not
   just WM_SIZE. That had to be fixed before Phase 3 adds more code to WndProc. Fixed
   by saving the whole bank in the frame; the five Windows `.app.ncodesum` moved and the
   diff was confined to WndProc.
4. **A new canvas runtime call must be listed per target.** The plan named the
   registry only; `src/target/{macos_aarch64,linux_common,win_x86_64}/mod.rs` each carry
   the list of `canvas.*` calls the native backend accepts. Added as a Phase 1 task.
   plan-155-B's `notePresent`/`frameCounters` and C's `awaitPickup` need the same, and
   C's timed wait also needs its libSystem imports on macOS' `plan.rs` arm.
5. **Final gate narrowed by the user (2026-09-24).** "Do not run the full suite as a
   final gate, only run the canvas tests. If main changes and there are not code overlap
   do not run the full suite, only run the canvas tests." plan-155-C's final gate is
   therefore the canvas test set (`rt_canvas_*`, the canvas CLI man-example test, the
   sentinel), not `cargo test`.
6. **macOS placement (Phase 2).** The plan named `metal.rs`/`bootstrap.rs` and
   `window.rs`. The code went into a new `app/display.rs`: the observer cannot live in
   `window.rs`, whose functions and selectors are emitted only for a window-member
   program, and the method must be on a class a canvas program always has —
   `MFBCanvasView`, built by the canvas reconcile. "Surface ready" is the end of
   `emit_reconcile_canvas_helper` (every canvas entry). A window not yet on a screen
   answers nil for `screen`, so `[NSScreen mainScreen]` stands in; headless runs
   therefore publish the main screen's rate too, which is why the tests that care set
   `MFB_CANVAS_DISPLAY_HZ`. Imports ride the canvas arm of `plan.rs`, like the Metal
   classes.
7. **A test count the design disproved.** `canvas_view_overrides_the_two_methods_that_deliver_keys`
   (plan-98-A; count last set in `f329ccc87`) pins `class_addMethod == 3` on the canvas
   view to catch stray overrides. §3 puts the screen-change method on that view, so the
   line became 4 and the selector joined the by-name list; nothing else in the test
   changed.
8. **GTK: the hook is the surface read, not `realize`.** The canvas reconcile already
   reads `gtk_native_get_surface` right after `gtk_window_present`, the first point a
   `GdkSurface` exists — the same moment `realize` would give, on a path that runs every
   canvas entry. Connecting there each time would stack handlers, and
   `g_signal_handlers_disconnect_matched` takes seven arguments (a stack argument on
   x86-64 SysV), so the helper records the surface it hooked in `ST_DISPLAY_HOOKED` and
   connects only for a new one. The first probe run looked like a failure
   (`displayHz=0.00` on both frames): WAYLAND_DEBUG showed no compositor event at all
   after the toplevel was created, and the same program from the HEAD compiler showed
   the same — the main thread was busy (`/proc/<pid>/task/*/wchan`: `running`) in GTK's
   first software-GL frame on this VM, and the 2 s program ended before the window
   mapped. A 12 s program showed the handler publishing 74.99. The plan's check named
   `scripts/linux-runtime-proof.sh`, which runs acceptance `.run` fixtures and cannot run
   a canvas app with `MFB_CANVAS_STATS`; the proof ran the AppImage on the box the way
   `scripts/test-canvas-gpu-rows.sh` does, without `MFB_GTKAPP_HEADLESS`.
9. **The Windows HWND-read count.** `canvas_hwnd_is_published_by_one_arm_and_cleared_by_the_others`
   (plan-98-A, extended by plan-98-F) enumerates every WndProc read of `CANVAS_HWND_SYM`;
   the display arms' gate is a seventh, so 12 relocations became 14 and the new reader
   is named in the message.
10. **`emit_publish_display_rate` signature.** Written with `&str` registers after
   `emit_publish_surface_size`; the Windows helper uses `abi::mfb_arg(n)` tokens (the
   volatile Win64 registers), which must reach the selector as tokens. Changed to
   `impl Into<Operand>` — the unit tests passed with the string form because they
   inspect pre-select instructions; the Windows `--app` build is what failed.
11. **bug-698, not bug-695.** The WndProc fix was committed (`13f3ea204`) as "bug-695",
   the next free number in this worktree; main meanwhile renumbered two of its own bugs
   to bug-695/696 (`a96313a76`) and added bug-697. The code comments and this record were
   renamed to bug-698 at the merge; the commit message keeps the old number.

## Summary

The risk is three unfamiliar platform APIs and whether our boxes can observe
them, so Phase 1 probes them before any codegen. Nothing about rendering,
redraw triggers, `present`, or the public API changes in this sub-plan. The
visible changes are the stats-line field and the four corrected sentences.

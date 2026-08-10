# plan-94-C: Mouse events — macOS app backend

Last updated: 2026-08-09
Effort: large (3h–1d)
Depends on: plan-94-B (the shared decoder + injection contract)

Wire native mouse input in `--app` mode on macOS by adding mouse-event IMPs to the
`TermView : NSView`, converting each to cell coordinates, formatting the SGR bytes,
and **injecting them into the same worker input pipe the backend already uses for
keystrokes** — so the plan-94-B decoder/ring decodes them with zero macOS-specific
event-queue code.

Behavioral outcome: a macОS app that `enableMouse(TRUE)` and polls receives
`Down`/`Up`/`Drag`/`Move`/`ScrollUp`/`ScrollDown` with correct cell coords and
modifiers from real trackpad/mouse input. (App mode is not headless-testable; the
gate is compile + assembly inspection + the `.app.ncode` golden diff, exactly like
the `didResize` macOS work.)

References:

- plan-94-A §3–§4 and plan-94-B §3 (the pump + injection contract — read first).
- `src/target/macos_aarch64/app/term_view.rs` (the TermView class synthesis,
  `setFrameSize:` IMP, and the `TVSTATE` struct with `TV_CELL_W`/`TV_CELL_H` cell
  metrics), `src/target/macos_aarch64/app/mod.rs` (`TV_*` offsets, selector table,
  IMP registration at `emit_term_set_frame_size_helper()` call site `:703`,
  selector/type-encoding tables `:857`/`:869`), `src/target/macos_aarch64/app/app_io.rs`
  (how the backend feeds input to the worker).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-94-B complete | CLI mouse rt test passes; the decoder consumes injected bytes from the input pipe | NOT MET |
| macOS app keystroke→worker input path located | grep the app input plumbing (Phase 1 task) | UNVERIFIED |

> If plan-94-B is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- The `TermView` overrides `mouseDown:`/`mouseUp:`/`mouseDragged:`/`mouseMoved:`/
  `rightMouseDown:`/`rightMouseUp:`/`otherMouse*:`/`scrollWheel:` (the set needed
  for the six `MouseKind`s + three buttons).
- Each IMP converts the event location to a 0-based cell `(row, col)` using the
  cached `TV_CELL_W`/`TV_CELL_H` (the same metrics `setFrameSize:` uses), formats
  the SGR report, and writes those bytes into the worker input pipe.
- Motion is delivered only while mouse mode is on (avoid flooding the pipe when the
  program hasn't enabled it): gate emission on the per-arena mouse flag, or on an
  NSTrackingArea installed only when enabled.

### Non-goals

- No change to CLI, GTK, or Windows.
- No new event queue: events flow as bytes through the existing pipe into the
  plan-94-B decoder/ring. (Keeps the ring worker-local — plan-94-A §4.3.)
- No change to `setFrameSize:`/`drawRect:`/the grid.

## 2. Current State

The `TermView` is synthesized for every macOS `--app` build (the `setFrameSize:`
IMP is emitted unconditionally — proven by the `didResize` work, whose 2-line
setFrameSize change shifted `macos-app-mode-io`/`plumbing` `.app.ncode` goldens).
`TVSTATE` caches `TV_CELL_W`@40/`TV_CELL_H`@48 (`app/mod.rs:425`), so px→cell is
`floor(px / cell)` — the exact computation `setFrameSize:` already does for
rows/cols. IMPs are registered via `class_addMethod` against a selector +
type-encoding table (`app/mod.rs:857`/`:869`); adding a mouse IMP mirrors the
`setFrameSize:` registration. The keystroke path already turns GUI key events into
bytes the worker reads as stdin — Phase 1 locates it and mouse mirrors it.

### Verified properties

- **`TV_CELL_W`/`TV_CELL_H` are the live cell metrics** (read by `setFrameSize:`
  to compute rows/cols) — so a mouse IMP can reuse them for px→cell. Verified from
  `app/mod.rs:425` and `term_view.rs` `setFrameSize:` body.
- **Keystroke→worker byte path exists** — UNVERIFIED exact symbol; Phase 1 first
  task greps `app/app_io.rs`/`bootstrap.rs` for the input-pipe write the key
  handler uses, and mouse reuses it.

## 3. Design

Per IMP: read `locationInWindow` (NSPoint, two doubles in the ObjC calling
convention), convert to view coords (`convertPoint:fromView:nil`), flip Y if
needed (NSView is bottom-left origin; grid is top-left), divide by
`TV_CELL_W`/`TV_CELL_H` → `(col,row)`, clamp to the grid. Encode the SGR report
(button/motion/modifier bits per plan-94-B §3, coords +1 back to 1-based), write
the bytes into the worker input pipe. `scrollWheel:` maps `deltaY` sign to
`ScrollUp`/`Down`. Modifiers come from `[NSEvent modifierFlags]`.

Emission gate: only write bytes when mouse mode is enabled — read the per-arena
mouse flag is awkward from a main-thread IMP (no x19, same constraint as
`setFrameSize:` in the didResize work), so gate instead on a `TVSTATE` mouse-enabled
byte set when `enableMouse` runs (mirror how `didResize` recorded resize on
`TVSTATE`), or install/remove an `NSTrackingArea` on enable/disable so motion IMPs
simply don't fire when off. Recommend the `TVSTATE` flag + always-registered IMPs;
simpler and matches the didResize precedent.

Risk: this is hand-written AArch64 in ObjC-callback context (no x19 arena base;
NSPoint float args; `convertPoint:` msgSend). Bounded by assembly inspection and
the `.app.ncode` golden diff.

## Phases

### Phase 1 — Locate the input pipe + add one IMP (mouseDown → Down)

- [ ] Grep `app/app_io.rs`/`bootstrap.rs` for the keystroke→input-pipe write;
      record the symbol/fd in Corrections.
- [ ] Add a `mouseDown:` IMP: px→cell via `TV_CELL_W/H`, format `\x1b[<0;x;yM`,
      write to the input pipe; register it beside `setFrameSize:` (`app/mod.rs`
      selector/type tables + `class_addMethod`).
- [ ] Add a `TVSTATE` mouse-enabled flag byte; `enableMouse` sets/clears it
      (macОS `emit_app_term_helper` arm, or via the shared path writing TVSTATE).

Acceptance: `cargo build` clean; the emitted `mouseDown:` IMP disassembles to the
expected px→cell + pipe-write; `.app.ncode` for a macos app fixture shows exactly
the new IMP (regenerate + confirm diff is only the addition).
Commit: —

### Phase 2 — Full event set

- [ ] Add `mouseUp:` (Up), `mouseDragged:`/`rightMouseDragged:` (Drag),
      `mouseMoved:` (Move, needs a tracking area or accepts-first-responder),
      `rightMouseDown:`/`Up:` and `otherMouse*:` (Right/Middle),
      `scrollWheel:` (ScrollUp/Down from `deltaY`).
- [ ] Modifiers from `modifierFlags` into the SGR bits.
- [ ] Y-flip + clamp verified against `drawRect:` coordinate origin.

Acceptance: `cargo build` clean; each IMP disassembles to the correct SGR encode;
regenerated `.app.ncode`/`.ncodesum` goldens diff only by the new IMPs; a manual
smoke build launches and (developer-verified interactively) reports clicks. Note in
the plan that interactive verification is manual — CI cannot drive a window.
Commit: —

## Validation Plan

- Tests: none runtime-headless; add a `syntax/app` fixture exercising
  `enableMouse`+`pollMouse` so the macOS app `.app.ncode` golden covers the IMPs.
- Runtime proof: manual — build the app, click, confirm the program prints the
  event (documented as a manual step; not in CI).
- Doc sync: term-backend spec macOS section — the mouse IMPs + px→cell + pipe
  injection.
- Acceptance: `cargo build`; `scripts/test-accept.sh <exe> /tmp/out '*app*'`;
  regenerate affected `.app.ncode`/`.ncodesum` and confirm diffs are only the IMPs.

## Open Decisions

- **Motion gating** — `TVSTATE` flag with always-registered IMPs (recommended) vs.
  install/remove `NSTrackingArea` on enable/disable. (§3)

## Corrections

<Filled in during execution — esp. the input-pipe symbol and the Y-flip origin.>

## Summary

C is per-backend hand-written assembly with the same headless-test limitation as
the `didResize` macOS work; the injection design keeps it small (format bytes,
write pipe) with no new queue. Gate is assembly + golden diff + manual smoke.

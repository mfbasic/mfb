# plan-35-D: App (macOS) — present-driven redraw + resize

Last updated: 2026-07-11
Effort: medium (1h–2h)
Depends on: plan-35-A

Converge the macOS `TermView` backend on the shared present model. macOS already
routes `io::write` into a view-resident `TermCell[]` grid while TUI is active; this
sub-plan (1) coalesces the **eager per-write redraw** into a present triggered by
`term::sync()` / `term::off`, and (2) adds **grid resize on window resize**
(currently the grid is fixed at init). The rendered content is unchanged; only
*when* the surface repaints changes.

Shared design of record: `planning/plan-35-shadow-grid-unify.md` (§2.3, §4.5).
Encodes D1 (mandatory `sync`; `off` implies a final present). Independent of the
console sub-plans B/C at the source level (different files), so D can land in
parallel with them after A.

## 1. Goal

- `term::sync()` (and `term::off`'s final present) is what calls
  `setNeedsDisplay:` on the `TermView`; `_mfb_macapp_term_writeString` no longer
  requests a redraw per write.
- On window resize, the grid recomputes rows/cols, reallocs the `TermCell[]`
  buffer, and forces a full redraw (so a resized window reflows instead of
  clipping at the init dimensions).
- `term::off` presents the final frame before the content-view swap back to the
  transcript.
- `mfb build -app` `examples/life` (macOS) renders identically to today, repaints
  once per present, and reflows on resize.

### Non-goals

- No change to the console backend or GTK (Phases B/C/E).
- No change to what is drawn — cell content, colors, fonts, cursor caret stay as
  today; only redraw *timing* and *resize* change.

## 2. Current State

`src/target/macos_aarch64/app/term_view.rs`: `TermCell[16B]` grid + 96-byte
`TVSTATE` (cursor row/col, cell metrics, current attrs). `mfbWriteString:`
(`_mfb_macapp_term_writeString`, `term_view.rs:825`) mutates the grid on the main
thread and ends with `setNeedsDisplay:` (the `w_redraw` label, ~`term_view.rs:982`).
`term::clear` also `performSelectorOnMainThread:@selector(setNeedsDisplay:)`
(`app/app_io.rs`). The grid is **not** resized on live window resize — the
autoresizing mask scales the view but rows/cols are fixed at
`_mfb_macapp_term_init` (`term_view.rs`, spec `04_term-backend.md` §term_init).
`io::flush` in app mode is a no-op returning OK (`app/app_io.rs:211`) — the
natural present hook. `term::off` (`emit_app_term_off_helper`, `app/app_io.rs`)
swaps the content view back to the transcript without a final present.

## 3. Design

- **Present.** Add a `term::sync` app arm (`emit_app_term_helper`, macOS
  `app/app_io.rs:712` table) that `performSelectorOnMainThread:@selector(setNeedsDisplay:)`
  on the `TermView`. Also make `io::flush` in app mode drive the same present
  (replacing its no-op body). Remove the per-write `setNeedsDisplay:` from
  `mfbWriteString:` (and the per-op redraws in `clear`/cursor ops) so redraw is
  present-driven only.
- **Resize.** Install a resize hook: on the view's frame change (override
  `setFrameSize:` or observe the window's resize), recompute
  `cols = floor(w/cellW)`, `rows = floor(h/cellH)`, realloc `TermCell[]`
  (preserving the top-left overlap, like the console realloc), update `TVSTATE`
  rows/cols, and `setNeedsDisplay:`. `term::terminalSize` already reads these
  fields, so a program re-querying size (as `life` does) sees the new extent.
- **`off` final present.** In `emit_app_term_off_helper`, `setNeedsDisplay:` +
  ensure the pending draw runs before/around the content-view swap so the last
  frame is shown.

Risk: main-thread ordering — the present must be marshaled
(`performSelectorOnMainThread:waitUntilDone:`) consistently with the existing
grid writes so a `sync` can't race ahead of the writes it should show. Resize
realloc must not tear a concurrent `drawRect:`.

## Phases

### Phase 1 — present-driven redraw

- [ ] `term::sync` app arm → `setNeedsDisplay:` on the `TermView`
      (`app/app_io.rs`); make `emit_app_io_flush_helper` (`app/app_io.rs:211`)
      drive the same present.
- [ ] Remove per-write `setNeedsDisplay:` from `_mfb_macapp_term_writeString`
      and the per-op redraws in the clear/cursor helpers (`term_view.rs`,
      `app/app_io.rs`).
- [ ] `emit_app_term_off_helper`: present the final frame before the content-view
      swap.

Acceptance: a `-app` program that draws then `term::sync()`s shows the frame; a
program that draws WITHOUT `sync()` shows nothing new (mandatory-present holds in
app mode too). Commit: —

### Phase 2 — grid resize on window resize

- [ ] Resize hook (`setFrameSize:`/window observer) recomputing rows/cols,
      reallocating `TermCell[]`, forcing a full redraw (`term_view.rs`).
- [ ] Tests: manual `-app` run of `examples/life` (macOS) — resize the window and
      confirm reflow; automated smoke that `term::terminalSize` reflects a
      programmatic frame change.

Acceptance: resizing the macOS app window reflows `life`; `term::terminalSize`
returns the new extent. Commit: —

## Validation Plan

- Runtime proof: `mfb build -app` `examples/life` on macOS — renders identically,
  one repaint per present, reflows on resize.
- Tests: `func_term_*` under `-app` (macOS) still green; the resize smoke.
- Doc sync: update `04_term-backend.md` §macOS (present-driven redraw + resize
  now implemented; remove the "not resized on live window resize" note).

## Summary

macOS moves from eager per-write redraw to present-driven, and gains resize
reflow. Content is unchanged; risk is main-thread present ordering and the resize
realloc vs `drawRect:` race. Console and GTK untouched.

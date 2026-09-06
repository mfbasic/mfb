# bug-540: Windows `--app` `term::` is a reduced implementation — styles ignored, fixed 80x25, no resize, no clustering

Last updated: 2026-09-05
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Open — **WIN-01 and WIN-05 are fixed; WIN-02, WIN-03 and WIN-04 remain**
Regression Test: `tests/cli_win_app_term_fidelity.rs` (three cases, covering
WIN-01 and WIN-05) and the `term::` style/reset case in `scripts/test-winapp.sh`
(box 2230). Nothing yet covers WIN-02/03/04.

The Windows `mfb build --app` `term::` backend draws, but it is not the same
surface the console and macOS backends present. Four documented contracts are
broken there, each silently — every call returns success and no diagnostic is
emitted:

- **WIN-01** `term::LineStyle` and `term::FillStyle` are ignored. Every line and
  box is `Light` whatever the program asked for, and `term::fillRect` paints the
  background colour instead of the requested block or shade glyph, so `Filled`,
  `Light`, `Medium`, `Dark`, `Checker` and `CheckerAlt` are indistinguishable.
- **WIN-02** the surface is a hard-coded 80 columns by 25 rows that does not
  follow the window, so `term::terminalSize` reports 80x25 regardless of how
  large the user makes the window, and everything outside that box is dead space.
- **WIN-03** `term::didResize` always reports `FALSE` — the Windows backend has
  no dispatcher arm for it and nothing on Windows ever sets the flag it falls
  back to reading — so a program that reflows on resize never reflows.
- **WIN-04** `term::drawText` walks UTF-16 units, not extended grapheme clusters,
  so a combining mark or a ZWJ emoji occupies several cells instead of one, and a
  double-width unit starting in the last column is drawn rather than dropped —
  the opposite of the "never split a wide glyph at the edge" rule the console and
  macOS backends implement.
- **WIN-05** (added 2026-09-05, found while fixing bug-541) `term::on` resets only
  `active`, `fg` and `bg` in the shared term-state. The console (`emit_on`), macOS
  (`emit_app_term_on_helper`) and GTK (`emit_app_term_on`) bodies all also reset
  `bold`, `underline`, `cursorVisible` and the pending-resize flag, and
  `mfb man term` promises `term::on` "resets all `term::` state to its defaults".
  So `term::setBold(TRUE)`, `term::off()`, `term::on()`, `term::getBold()`
  answered `TRUE` on Windows and `FALSE` on every other backend. This one IS
  observable from a program and headlessly, which is why it is the sub-issue with
  a runtime assertion.

**The single correct behavior a fix produces:** a program built with
`mfb build --app` on Windows paints the same cells, with the same glyphs, the
same surface size, the same resize reporting and the same cluster/width handling
as the console backend and the macOS app backend.

References:

- `mfb spec app term-backend` → the per-backend coverage table and the
  "Windows: GDI memDC (immediate mode)" section, which currently records all four
  of these as named gaps.
- `mfb man term` gap 2, and the per-member disclosures on `drawHLine`,
  `drawVLine`, `drawBox`, `fillRect`, `drawText`, `terminalSize` and
  `didResize`. Closing this bug means deleting those disclosures.
- Found during the `term::` row/column coordinate migration, commit `fc1860141`,
  which fixed the *coordinate* half of this backend (clamping, endpoint
  normalisation, span clipping, control-glyph skip) and left the fidelity half.
- Sibling gaps filed at the same time: bug-539 (GTK draws nothing), bug-541
  (inactive gate not enforced in app mode).

**Update after bug-539 landed.** No shared root cause: bug-539 was the GTK
dispatcher returning `None` for the six positioned members (so they fell through
to a console emitter that no-ops in an app build), whereas Windows *does*
dispatch all six and the defect is in the bodies themselves. Nothing in the
bug-539 fix touched `src/target/win_x86_64/`, and the Windows `.ncodesum` goldens
are byte-identical across it.

The GTK fix does, however, supply a worked precedent for **WIN-01** and
**WIN-04**, both of which are now solved twice in-tree rather than once:

- WIN-01: `src/target/linux_gtk/term_draw.rs:emit_select_packed_glyph` resolves
  the `LineStyle`/`FillStyle` ordinal against the same
  `crate::codegen::error::constants::TERM_*_CODEPOINTS` tables the console and
  macOS backends read, converting each entry to the backend's own cell encoding
  **at emit time** (`pack_codepoint`). That is the shape WIN-01 wants: read the
  shared table, never hard-code a code point.
- WIN-04: `term::drawText` there is not a second walk. It is the *write* helper's
  own cluster walk emitted a second time under `TermWriteMode::DrawText`, so the
  clustering, the width lookup and the "drop a wide cluster that would not fit"
  rule are literally the same instructions `io::write` uses. If the Windows
  backend has an immediate-mode text writer with correct clustering, WIN-04 is the
  same move; if it does not, WIN-04 and its `io::write` twin should be fixed
  together rather than separately.

## What landed on 2026-09-05, and what did not

**WIN-01 and WIN-05 are fixed.** They are the two sub-issues that are contained,
that do not depend on the surface becoming dynamic, and whose correctness can be
established against an in-tree oracle rather than a photograph.

* WIN-01: `emit_win_select_codepoint` is a compare/branch chain over a
  `TERM_*_CODEPOINTS` table, called once per glyph — one for a rule, six for a box
  (two edges selected once each, four corners), one for a fill. The dash/dot
  corner fallback comes for free, because it lives in the table.
* WIN-05: `emit_term_on` resets all seven fields. Measured on 2230, headless, same
  program before and after: `reset bold=TRUE underline=TRUE` became
  `reset bold=FALSE underline=FALSE`, which is what every other backend answers.

**WIN-02, WIN-03 and WIN-04 are NOT fixed**, and the reason is recorded here
rather than being left as an unstated omission.

* **WIN-02/WIN-03 have no instrument on the one box that can run them.** A
  Windows `term::` resize is a window event, and `scripts/test-winapp.sh` runs
  `MFB_WINAPP_HEADLESS=1`, which builds no window and runs no message loop — so
  `WM_SIZE` is unreachable there. The canvas backend solved the identical problem
  with a *scripted* resize (`MFB_CANVAS_RESIZE_W`/`_H` drive the same publisher on
  the headless path — see the "resize handshake on Windows" section of
  `scripts/test-winapp.sh`). **WIN-02 should not be attempted before the `term::`
  twin of that hook exists**, because the failure mode the fix risks is a clamp
  that still says 79 while the surface is 120 wide, which silently truncates and
  which no compile-time gate can see. That hook is new product surface and belongs
  in this bug's Phase 2, named as a prerequisite rather than discovered mid-fix.
* **WIN-04's correct fix is a refactor of `emit_app_io_write`, not a new walk.**
  Confirmed by reading it: that function's grid path already decodes surrogate
  pairs, folds trailing combining marks and ZWJ sequences, computes a display
  width and reserves the trailing column — roughly 430 lines, addressed through
  its own private slot constants and with label names (`term_loop`, `term_extend`,
  `term_nl`, …) that are not tagged per call site. Sharing it means tagging every
  label and parameterising the slot set, exactly as `linux_gtk`'s `TermWriteMode`
  does for the GTK twin. That is the right change and it is not a small one;
  doing it as a second copy is what the Summary below explicitly forbids.
* **Neither WIN-01's nor WIN-04's pixel-level claim is runtime-provable today.**
  The GDI grid has no readback path (`MFB_CANVAS_DUMP` has no `term::` twin) and
  the box is headless, so "which glyph landed in which cell" cannot be observed
  from 2230. WIN-01 is therefore pinned by *codegen inspection against the macOS
  oracle* — the two backends must emit the same glyph set for the same member —
  plus a runtime assertion that every style's select chain executes without
  faulting. Said plainly rather than implied: the artifact gate and the runtime
  run together prove the right code points are selected and that the code runs;
  they do not prove the pixels.

**Open Decision closed with evidence.** "Confirm no shipped example depends on
80x25": none does. All four `term::terminalSize` callers in `examples/`
(`life`, `snake`, `ai_chat`, `browser/app`) wrap it in a `TRAP` and reflow from
the value — `examples/life/src/main.mfb:283` is the shape they all share
(`grep -rn "terminalSize" examples/`). WIN-02 is free to change the answer.

**One correction to this report.** Its Root Cause on `emit_term_fill_rect` is
accurate, but the Blast Radius line "`emit_app_io_write` — its cluster walk and
trailing-column reservation are the model for WIN-04" understates it: the walk is
not a *model*, it is the implementation WIN-04 must call. That is what makes
WIN-04 a refactor rather than an addition.

## Failing Reproduction

```
cat > /tmp/winterm/src/main.mfb <<'MFB'
IMPORT term
IMPORT color
IMPORT io
FUNC main() AS Integer
  term::on()
  LET size AS term::TermSize = term::terminalSize()
  io::print(toString(size.columns) & "x" & toString(size.rows))
  term::drawHLine(term::LineStyle.Double, 1, 1, 30)      ' WIN-01: expect ═, get ─
  term::drawBox(term::LineStyle.HeavyDash, 3, 1, 8, 30)  ' WIN-01: expect ┅┇, get ─│
  term::fillRect(term::FillStyle.Dark, 10, 1, 12, 30)    ' WIN-01: expect ▓, get blank
  term::drawText(14, 1, "cafe" & "́" & "|日本|")           ' WIN-04: expect 8 cells, get 9
  io::print(toString(term::didResize()))                 ' WIN-03: FALSE after a resize
  term::sync()
  term::off()
  RETURN 0
END FUNC
MFB
mfb build --app -target windows-x86_64 /tmp/winterm
# ship to 2230 (Win11 x86_64) per scripts/test-winapp.sh and run; resize the
# window before the didResize() read.
```

- Observed: `80x25` whatever the window size (WIN-02); every rule and box drawn
  in `─`/`│` with `┌┐└┘` corners (WIN-01); the `fillRect` region shows only the
  background colour (WIN-01); the combining acute occupies its own cell and the
  trailing `|` lands one column right of where the console puts it (WIN-04);
  `FALSE` even though the window was resized (WIN-03).
- Expected: the live window size in cells; `═`, `┅`/`┇` with heavy corners, `▓`;
  `café|日本|` in 8 cells; `TRUE` on the read after the resize.

Contrast cases that work today and bound the bug:

| Environment | Build | WIN-01 | WIN-02 | WIN-03 | WIN-04 |
| --- | --- | --- | --- | --- | --- |
| Windows console (2230) | `mfb build` | ✓ | ✓ | ✓ | ✓ |
| macOS app | `mfb build --app` | ✓ | ✓ | ✓ | ✓ |
| Windows app (2230) | `mfb build --app` | ✗ | ✗ | ✗ | ✗ |

The Windows *console* build on the same box is correct on all four, which
localises every one of these to `src/target/win_x86_64/app/`.

## Root Cause

All four are in `src/target/win_x86_64/app/mod.rs`.

**WIN-01.** `emit_term_draw_line` writes the glyph as a literal — `9472` (`─`)
for the horizontal form and `9474` (`│`) for the vertical — chosen by the
emit-time `horizontal` flag, never from `ARG[0]`. `emit_term_draw_box` does the
same for its two edges and hard-codes `9484`/`9488`/`9492`/`9496` for the four
corners. `emit_term_fill_rect` stores `32` (space) as its glyph and relies on the
background colour to make the region visible. In all three the `LineStyle` /
`FillStyle` ordinal arrives in `ARG[0]` and is simply never read. The console and
macOS backends instead index
`crate::codegen::error::constants::TERM_HLINE_CODEPOINTS`,
`TERM_VLINE_CODEPOINTS`, `TERM_CORNER_TL/TR/BL/BR_CODEPOINTS` and
`TERM_FILL_CODEPOINTS` by that ordinal — macOS through the reusable
`emit_app_select_unichar` helper, which has no Windows counterpart.

**WIN-02.** The backend is immediate-mode with no cell grid; the surface is the
compile-time pair `const TUI_COLS: usize = 80` / `const TUI_ROWS: usize = 25`.
`emit_term_size` allocates the `TermSize` record and stores those two constants
into it. It never asks the window for its client rect, and the cell metrics are
themselves fixed (Consolas at 8x16 px), so nothing in the backend is
size-derived.

**WIN-03.** `emit_app_term_helper` has no `"term.didResize"` arm, so the call
falls through to the console `src/codegen/term/core/term.rs:emit_did_resize`,
which reads-and-clears `term_state_offset + TERM_STATE_DID_RESIZE_OFFSET` (slot
56). On the console that slot is latched by the present path when the terminal
size changes; on macOS and GTK the backends own the flag on their own surface
state (`TV_DID_RESIZE`, `ST_TERM_DID_RESIZE`). On Windows nothing writes it —
`grep -n 'TERM_STATE_DID_RESIZE_OFFSET' src/target/win_x86_64/` returns nothing —
so the reader always sees the zero it was initialised with. WIN-03 is downstream
of WIN-02: there is no resize to latch while the surface is a constant.

**WIN-04.** `emit_term_draw_text_at` converts UTF-8 to UTF-16 once and then
iterates UTF-16 *units*, decoding a surrogate pair as one glyph and advancing the
unit index by `UCOUNT`. It has no combining-mark/ZWJ extension loop, so a cluster
is never folded into one cell position. It also tests only `CURCOL >= TUI_COLS`
before stamping, with no "reserve the trailing column for a width-2 cluster"
preflight, where the console `emit_draw_text` drops the cluster and stops the run
and macOS drops the whole cluster. The same file's *write* path
(`emit_app_io_write`) DOES extend clusters and DOES reserve the trailing column —
so the two paths in one backend disagree about the same string, which is the
sharpest statement of the defect.

## Goal

- **WIN-01** `emit_term_draw_line`, `emit_term_draw_box` and
  `emit_term_fill_rect` select every glyph from the shared `TERM_*_CODEPOINTS`
  tables by the incoming ordinal, including the dash/dot corner fallback to the
  matching Light/Heavy corner that the console and macOS backends implement.
- **WIN-02** the surface follows the window: `emit_term_size` reports the client
  rect divided by the cell metrics, and every emitter's bound comes from that
  rather than from `TUI_COLS`/`TUI_ROWS` constants.
- **WIN-03** a genuine client-size change latches a flag that `term::didResize`
  reads and clears exactly once, via a Windows dispatcher arm reading Windows
  surface state (the macOS `TV_DID_RESIZE` shape).
- **WIN-04** `emit_term_draw_text_at` folds extended grapheme clusters, computes
  a display width per cluster, and drops a width-2 cluster that would straddle
  the right edge instead of drawing it.

### Non-goals (must NOT change)

- The immediate-mode design. This backend deliberately has no retained cell
  grid; the fix must not introduce one to make WIN-04 easier.
- The `WM_PAINT` / `BitBlt` present path, the Consolas `CreateFontW` setup, or
  the `DEFAULT_CHARSET` font-linking that supplies CJK.
- The coordinate work already landed in `fc1860141` —
  `win_normalize_pair`, `win_clip_span`, `win_clamp_slot`, `win_guard_on_grid`,
  `win_clamp_register` and the row-before-column argument order. WIN-02 changes
  what the bounds *are*, not the rules applied to them.
- The console and macOS backends, which are the behavioural oracle.
- **Tempting wrong fixes, explicitly forbidden:** (a) "documenting" WIN-02 as
  intended by declaring 80x25 the Windows surface contract — the man pages
  already disclose it as a gap, and a fixed surface makes `terminalSize` useless
  for layout on the one platform where windows are resizable by default;
  (b) making `didResize` return `TRUE` once at startup to satisfy a reflow loop;
  (c) approximating WIN-04 by counting UTF-16 units and calling it a width.

## Blast Radius

Found with `grep -n 'TUI_COLS\|TUI_ROWS\|9472\|9474\|948[0-9]\|949[0-9]\|TERM_STATE_DID_RESIZE_OFFSET' src/target/win_x86_64/app/mod.rs`
and by diffing the Windows dispatcher arms against
`src/codegen/builtins/term/mod.rs`'s 24 members.

- `src/target/win_x86_64/app/mod.rs:emit_term_draw_line` — WIN-01; fixed here.
- `src/target/win_x86_64/app/mod.rs:emit_term_draw_box` — WIN-01; fixed here.
- `src/target/win_x86_64/app/mod.rs:emit_term_fill_rect` — WIN-01; fixed here.
- `src/target/win_x86_64/app/mod.rs:emit_term_size` — WIN-02; fixed here.
- `src/target/win_x86_64/app/mod.rs:emit_app_term_helper` — WIN-03 (missing arm);
  fixed here.
- `src/target/win_x86_64/app/mod.rs:emit_term_draw_text_at` — WIN-04; fixed here.
- Every other `TUI_COLS`/`TUI_ROWS` use — `emit_term_draw_line`,
  `emit_term_draw_box`, `emit_term_fill_rect`, `emit_term_draw_glyph_at`,
  `emit_app_io_write`, and the `win_clip_span`/`win_guard_on_grid` call sites
  added in `fc1860141` — all in scope for WIN-02: each is a bound that must
  become dynamic together, or the surface and its clamps disagree.
- `src/target/win_x86_64/app/mod.rs:emit_app_io_write` — its cluster walk and
  trailing-column reservation are the model for WIN-04 and should be shared, not
  duplicated.
- `src/target/macos_aarch64/app/app_io.rs:emit_app_select_unichar` — unaffected,
  and is the shape WIN-01 should follow.
- `src/target/linux_gtk/app_io.rs` — unaffected by this bug; its own gap is
  bug-539. Note WIN-01's table-driven selection is exactly what bug-539 Phase 2
  must also do, so land whichever comes second against the first's helper.
- `tests/syntax/app/macos-app-mode-term` — carries the
  `windows-x86_64.app.ncodesum` golden and, since `fc1860141`, exercises all six
  positioned helpers. It will shift; that is the intended sentinel.

## Fix Design

Four independent sub-issues; WIN-01 and WIN-04 are self-contained, WIN-03 depends
on WIN-02.

**WIN-01** is the cheapest and highest-value: add a Windows counterpart to
macOS's `emit_app_select_unichar` (a compare/branch chain over a 7-entry table
writing the chosen unichar to a slot) and call it once per glyph — one for a
line, six for a box, one for a fill.

**WIN-02** is where the risk concentrates. Today `TUI_COLS`/`TUI_ROWS` are
compile-time constants folded into immediates all over the file; making the
surface dynamic means introducing two writable globals beside `TUI_ROW_SYM` /
`TUI_COL_SYM`, seeding them from the client rect at `term::on` and on
`WM_SIZE`, and replacing **every** immediate. A partial conversion is worse than
none: a clamp that still says 79 while the surface is 120 wide silently truncates.

**WIN-03** then follows the macOS shape — a `didResize` flag on Windows surface
state, set by the `WM_SIZE` handler that WIN-02 already needs, plus a
`"term.didResize"` dispatcher arm that reads and clears it.

**WIN-04** should reuse `emit_app_io_write`'s existing cluster walk rather than
grow a second one, exactly as the Open Decision in bug-539 proposes for GTK: the
walk needs a mode that does not commit the shadow cursor.

Expected generated-output shift: the `windows-x86_64.app.ncodesum` golden on
`tests/syntax/app/macos-app-mode-term`, and any other Windows app `.ncodesum`.
Confirm the delta is confined to the Windows app bodies.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] `tests/syntax/app/macos-app-mode-term` already exercises a dash style, a
      dot style, `Double` and a `FillStyle` through all six positioned members
      (it gained them in bug-539), so its `windows-x86_64.app.ncodesum` is the
      artifact-gate sentinel for WIN-01 and no fixture change was needed. It does
      NOT cover a combining cluster or a wide cluster at the right edge; that is
      WIN-04's to add, with WIN-04.
- [x] Added `tests/cli_win_app_term_fidelity.rs`. It asserts WIN-01 by comparing
      the Windows body's glyph immediates against the **macOS body's**, member for
      member, rather than restating the tables here — a restated table can drift
      from the emitters and keep passing. RED confirmed at `8d87e06b7`:
      `drawHLine` emitted `{2500}` against macOS's
      `{2500,2501,2504,2505,2508,2509,2550}`.
- [x] Added the `term::` style/reset case to `scripts/test-winapp.sh` and ran it
      on 2230. RED confirmed for WIN-05 (`reset bold=TRUE underline=TRUE`) and for
      WIN-02 (`size=80x25`). WIN-01 and WIN-04 are **not** observable from that
      box — see "What landed" above.
- [x] Re-ran the blast-radius search. **33 `TUI_COLS`/`TUI_ROWS` sites**
      (`grep -c 'TUI_COLS\|TUI_ROWS' src/target/win_x86_64/app/mod.rs`), of which
      2 are the definitions and 5 are prose; the rest are the immediates WIN-02
      must convert, spread across `emit_term_on`, `emit_term_clear`,
      `emit_term_move_to`, `emit_term_draw_line`, `emit_term_draw_box`,
      `emit_term_fill_rect`, `emit_term_draw_glyph_at`, `emit_term_size`,
      `emit_app_io_write` and the `WndProc` `WM_PAINT` BitBlt.

Acceptance: met for WIN-01 and WIN-05. WIN-02/03/04 keep an OPEN Phase 1 item —
the scripted-resize hook (see "What landed") — which is why they did not land.
Commit: —

### Phase 2 — the fix

- [x] WIN-01: `emit_win_select_codepoint` added; line, box (both edges + all four
      corners, with the dash/dot corner fallback that lives in the tables) and
      fill all driven from `TERM_*_CODEPOINTS` by the incoming ordinal. Each body
      parks that ordinal as its FIRST store, because the clipping helpers below it
      use ARG[0..2] as scratch.
- [x] WIN-05: `emit_term_on` resets all seven shared term-state fields.
- [ ] **WIN-02 prerequisite (new):** a scripted `term::` resize hook on the
      headless Windows path, the twin of `MFB_CANVAS_RESIZE_W`/`_H`. Without it
      WIN-02 is unfalsifiable on the only box that can execute it.
- [ ] WIN-02: make the surface dynamic — writable cols/rows globals seeded at
      `term::on` and updated on `WM_SIZE`; convert every `TUI_COLS`/`TUI_ROWS`
      immediate, including the `win_clip_span`/`win_guard_on_grid` bounds. Note
      the backing bitmap does not have to be recreated on every resize: allocating
      it once at a generous maximum and treating cols/rows as the *logical*
      surface avoids a UI-thread/worker race over the memDC's bitmap entirely.
- [ ] WIN-03: latch a resize flag in the `WM_SIZE` handler; add the
      `"term.didResize"` arm that reads and clears it.
- [ ] WIN-04: share `emit_app_io_write`'s cluster walk with
      `emit_term_draw_text_at` under a "do not commit the cursor" mode, including
      the trailing-column reservation. Its labels must gain a per-call-site tag
      first; today they are bare (`term_loop`, `term_extend`, `term_nl`, …) and two
      copies in one object would collide.

Acceptance: met for WIN-01 and WIN-05.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [x] `scripts/regen-outside-ncode.sh`. **Exactly one golden moved** —
      `macos-app-mode-term`'s `windows-x86_64.app.ncodesum`. The other five app
      sums are byte-identical, including `macos-app-mode-io` and
      `macos-app-mode-plumbing` on the SAME target, which is what says the change
      is confined to the `term::` bodies rather than to shared app plumbing.
- [x] Deleted the *style* disclosures only: the "ignores `line`/`fill`"
      paragraph on `drawHLine`, `drawVLine`, `drawBox` and `fillRect`, and the
      style clause of the `mfb man term` overview gap. The `terminalSize`,
      `didResize` and `drawText` disclosures **stay**, because WIN-02/03/04 stay.
      The spec's coverage table now reads "yes, per `LineStyle`/`FillStyle`" in the
      Windows column for those three rows, and its Windows section records both
      the table-driven selection and the `term::on` reset.
- [x] `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
      `scripts/artifact-gate.sh all`, `scripts/man-census.sh --fill term`.
- [x] Re-ran on 2230. A live window resize is still not runnable there — see the
      scripted-hook prerequisite above.

Acceptance: met for WIN-01 and WIN-05.
Commit: —

## Validation Plan

- Regression test(s): the extended Windows app `.ncodesum` fixture (artifact
  gate) plus the `scripts/test-winapp.sh` end-to-end case.
- Runtime proof: the reproduction on 2230 with a live resize — lowering is not
  runtime proof for a per-backend change, and WIN-02/WIN-03 cannot be observed
  from a dump at all.
- Doc sync: `mfb man term` gap 2 and seven member pages;
  `src/docs/spec/app/04_term-backend.md` coverage table and Windows section.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`.

## Open Decisions

- ~~Land WIN-01 alone first?~~ **Done.** WIN-01 landed on its own (with WIN-05,
  which is the same shape of defect in the same file and is the one sub-issue with
  a real runtime assertion), before the dynamic-size conversion starts.
- ~~Confirm no shipped example depends on 80x25.~~ **Closed: none does.** All four
  `term::terminalSize` callers in `examples/` trap and reflow from the value.
- **Still open, and it is the gating question for WIN-02/WIN-03:** does the
  scripted `term::` resize hook (`MFB_WINAPP_TERM_RESIZE_W`/`_H`, the twin of the
  canvas one) belong in the product, or should WIN-02 land on lowering-level
  evidence plus structural assertions the way a hand-written aarch64 GTK body has
  to? The canvas precedent says the hook is acceptable; recording it as a decision
  because it adds test-only surface to a shipped binary, which is a judgement call
  and not a technical one.

## Summary

WIN-01 and WIN-05 are done. The engineering risk is almost entirely WIN-02: `TUI_COLS`/`TUI_ROWS` are folded
into immediates throughout the file and a partial conversion silently truncates
drawing. WIN-01 is contained and should land first. WIN-04's risk is duplicating
rather than sharing the cluster walk, which would leave `drawText` and
`io::write` disagreeing about the same string in the same backend — the exact
defect this bug records.

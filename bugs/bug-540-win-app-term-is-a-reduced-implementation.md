# bug-540: Windows `--app` `term::` is a reduced implementation — styles ignored, fixed 80x25, no resize, no clustering

Last updated: 2026-09-04
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: — (none exists; Phase 1 adds one)

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

- [ ] Extend the Windows app fixture (or add one) so the artifact gate covers a
      dash style, a dot style, `Double`, every `FillStyle`, a combining cluster
      and a wide cluster at the right edge.
- [ ] Add a `scripts/test-winapp.sh` end-to-end case running the reproduction on
      2230, capturing the window and the two `io::print` lines; confirm all four
      sub-issues fail today.
- [ ] Re-run the blast-radius searches and record a verdict per site, especially
      the complete list of `TUI_COLS`/`TUI_ROWS` immediates WIN-02 must convert.

Acceptance: the end-to-end case fails on all four sub-issues for the documented
reasons; the `TUI_COLS`/`TUI_ROWS` site list is complete.
Commit: —

### Phase 2 — the fix

- [ ] WIN-01: add the Windows unichar-select helper; drive line, box (edges +
      corners, with the dash/dot corner fallback) and fill from the shared tables.
- [ ] WIN-02: make the surface dynamic — writable cols/rows globals seeded at
      `term::on` and updated on `WM_SIZE`; convert every `TUI_COLS`/`TUI_ROWS`
      immediate, including the `win_clip_span`/`win_guard_on_grid` bounds.
- [ ] WIN-03: latch a resize flag in the `WM_SIZE` handler; add the
      `"term.didResize"` arm that reads and clears it.
- [ ] WIN-04: share `emit_app_io_write`'s cluster walk with
      `emit_term_draw_text_at` under a "do not commit the cursor" mode, including
      the trailing-column reservation.

Acceptance: the Phase 1 end-to-end case passes on all four; nothing in Non-goals
changed; the coordinate helpers from `fc1860141` still behave as their tests say.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/regen-ncodesum.sh`; confirm the delta is confined to the Windows
      app bodies.
- [ ] Delete gap 2 from the `mfb man term` overview and the per-member
      disclosures on `drawHLine`, `drawVLine`, `drawBox`, `fillRect`, `drawText`,
      `terminalSize` and `didResize`; update the spec coverage table's "Windows
      app" column and the three bullets in the Windows section.
- [ ] `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
      `scripts/artifact-gate.sh all`, `scripts/man-census.sh --fill term`.
- [ ] Re-run the reproduction on 2230, including a live window resize.

Acceptance: full suite green; the deltas are only the Windows app bodies; the
docs no longer disclose gaps that no longer exist.
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

- Land WIN-01 alone first? It is a contained, high-value change (styles are the
  visible half of the gap) and does not depend on WIN-02. Recommended: yes —
  split this bug's Phase 2 so WIN-01 lands and is proven before the dynamic-size
  conversion starts.
- WIN-02 changes `term::terminalSize`'s answer on Windows app mode from a
  constant to a live value. Confirm no shipped example depends on 80x25.

## Summary

The engineering risk is almost entirely WIN-02: `TUI_COLS`/`TUI_ROWS` are folded
into immediates throughout the file and a partial conversion silently truncates
drawing. WIN-01 is contained and should land first. WIN-04's risk is duplicating
rather than sharing the cluster walk, which would leave `drawText` and
`io::write` disagreeing about the same string in the same backend — the exact
defect this bug records.

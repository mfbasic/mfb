# bug-539: Linux `--app` mode silently draws nothing for all six positioned `term::` calls

Last updated: 2026-09-04
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: — (none exists; Phase 1 adds one)

A program that draws a TUI with `term::drawText`, `term::drawGlyph`,
`term::drawHLine`, `term::drawVLine`, `term::drawBox` or `term::fillRect` renders
correctly in a Linux **terminal** and in a macOS or Windows **`mfb build --app`**
window, but renders **nothing at all** from those six calls in a **Linux
`--app`** window. The cursor, colours, attributes, `clear`, `sync`,
`terminalSize` and `didResize` all work, so the window appears alive and the
program reports success — it just has no text, no rules and no boxes on it.

The failure is **silent**: no error is raised, no diagnostic is emitted, and
`term::isOn` reports `TRUE`. A developer sees a blank window and has no signal
pointing at the backend. `examples/wide-demo` and `examples/ai_chat` — both of
which compose their entire UI out of these six calls — are blank windows on
Linux app mode today.

**The single correct behavior a fix produces:** the GTK4 backend implements all
six positioned drawing helpers against its own cell grid, so a program built with
`mfb build --app` on Linux paints the same cells, with the same `LineStyle` /
`FillStyle` glyphs and the same clamping/clipping rules, as the console backend
and the macOS app backend already do.

References:

- `mfb spec app term-backend` → the per-backend coverage table
  (`src/docs/spec/app/04_term-backend.md`) and the Linux GTK4 section.
- `mfb man term` gap 1 — the man pages currently DISCLOSE this gap rather than
  promising parity. Closing this bug means deleting that disclosure.
- Found during the `term::` row/column coordinate migration, commit `fc1860141`.
- Sibling gaps filed at the same time: bug-540 (Windows app-mode reduced
  implementation), bug-541 (inactive gate not enforced in app mode).

## Failing Reproduction

```
cat > /tmp/gtkdraw/src/main.mfb <<'MFB'
IMPORT term
IMPORT color
FUNC main() AS Integer
  term::on()
  term::setForeground(color::rgb(0, 255, 0))
  term::drawBox(term::LineStyle.Double, 1, 2, 8, 40)
  term::drawText(3, 4, "if you can read this, the bug is fixed")
  term::drawHLine(term::LineStyle.Heavy, 5, 3, 39)
  term::fillRect(term::FillStyle.Medium, 6, 4, 7, 38)
  term::drawGlyph(2, 4, 9731)
  term::sync()
  os::sleep(5000)
  term::off()
  RETURN 0
END FUNC
MFB

# Console build on the same box: draws everything.
mfb build /tmp/gtkdraw && /tmp/gtkdraw/build/gtkdraw.out

# App build on the same box (2228 Ubuntu x86_64 GTK, or 2226 Debian aarch64 GTK):
mfb build --app -target linux-x86_64 /tmp/gtkdraw
# ship + run on the box, per scripts/test-appimage.sh
```

- Observed: the app window opens, is cleared to the term background, and stays
  empty. Exit code 0, no diagnostic.
- Expected: the same box, text, rule, fill and snowman the console build paints.

Contrast cases that work today and bound the bug:

| Environment | Build | Result |
| --- | --- | --- |
| Linux terminal (any box) | `mfb build` | works ✓ |
| macOS app | `mfb build --app` | works ✓ |
| Windows app (2230) | `mfb build --app` | draws ✓ (with bug-540's style/cluster gaps) |
| Linux app (2228 / 2226) | `mfb build --app` | draws nothing ✗ |

`term::moveTo` + `io::write` DOES paint in Linux app mode, which is the sharpest
contrast: the GTK grid, the cell writer and the present path all work. Only the
six positioned entry points are missing.

## Root Cause

`src/target/linux_gtk/app_io.rs:emit_app_term_helper` is the GTK app-mode
dispatcher. Its `match call` has arms for `term.on`, `off`, `isOn`, `didResize`,
`clear`, `sync`, `moveTo`, `setForeground`, `setBackground`, `setBold`,
`setUnderline`, `terminalSize`, `showCursor` and `hideCursor`, and then
`_ => return None`. All six positioned drawing calls hit that fall-through.

`None` means "this backend does not implement the call — use the console
emitter", and `src/codegen/builtins/term/gen_shared.rs:lower_term_helper` duly
falls through to `console_lower_term_helper`. That is normally the right
behaviour (it is how the GTK backend reuses the pure readers), but for a *writer*
it is a trap: every console drawing emitter opens with `emit_gate_inactive` and
then `emit_load_grid`, which loads the console shadow-grid header pointer from
`term_state_offset + TERM_STATE_GRID_OFFSET` and branches to the inactive label
when it is null.

That pointer is written **only** by the console `term::on`
(`src/codegen/term/core/term.rs`, the sole writer — `grep -n
'TERM_STATE_GRID_OFFSET' src/target/**` returns nothing). In a GTK app build
`term::on` is `src/target/linux_gtk/app_io.rs:emit_app_term_on`, which sets up
the GTK surface and never allocates a console grid, so slot 48 stays 0 for the
life of the program. Every fallen-through drawing call therefore takes the
inactive branch on its first instruction and returns `RESULT_OK_TAG`.

So the silence is structural, not incidental: the fall-through path is
*guaranteed* to no-op in an app build, and it reports success while doing it.

## Goal

- All six of `term::drawHLine`, `drawVLine`, `drawBox`, `fillRect`, `drawText`
  and `drawGlyph` have GTK arms in
  `src/target/linux_gtk/app_io.rs:emit_app_term_helper` and paint the GTK cell
  arrays.
- The glyph selected matches the `LineStyle` / `FillStyle` ordinal, using the
  same `TERM_HLINE_CODEPOINTS` / `TERM_VLINE_CODEPOINTS` / `TERM_CORNER_*` /
  `TERM_FILL_CODEPOINTS` tables the console and macOS backends read.
- The coordinate rules match the console backend exactly: row-before-column
  arguments (`mfb spec app term-backend` → "Coordinate convention"), endpoints
  and corners accepted in either order, spans clamped, a fixed coordinate or box
  corner off the grid skipped rather than slid onto the rim, `drawText` clipped
  at both edges and `drawGlyph` bounds-checked.
- `drawText` honours the GTK wide/EGC-pool cell model already used by the GTK
  write path (`GTK_WIDE_TRAIL`, `ST_TERM_POOL`).

### Non-goals (must NOT change)

- The GTK cell storage layout (parallel char/fg/bg static arrays + the 32 B/cell
  EGC pool). This bug adds writers, not a new representation.
- The shared console term-state global, and the `None` fall-through contract for
  the pure readers (`isOn`, the attribute getters) — those are correct.
- The console, macOS and Windows backends. The console emitters are the
  behavioural oracle here and must not be reshaped to make the GTK port easier.
- **Tempting wrong fix, explicitly forbidden:** making the fall-through raise
  `ErrUnsupported` instead of no-opping. That converts a silent blank window into
  a program that dies on Linux app mode and does not compile-time differ from
  every other target — the calls are legal and the docs promise they draw.
  Equally forbidden: deleting the man-page/spec gap disclosure without
  implementing the helpers.

## Blast Radius

Found with `grep -n '_ => return None' src/target/linux_gtk/app_io.rs` and by
diffing the GTK dispatcher's arms against `src/codegen/builtins/term/mod.rs`'s
24 registered members.

- `src/target/linux_gtk/app_io.rs:emit_app_term_helper` — the bug; fixed here.
- `src/target/macos_aarch64/app/app_io.rs:emit_app_term_helper` — unaffected:
  implements all six (`emit_app_draw_line`, `emit_app_draw_box`,
  `emit_app_fill_rect`, `emit_app_draw_text`, `emit_app_draw_glyph`).
- `src/target/win_x86_64/app/mod.rs:emit_app_term_helper` — unaffected by *this*
  bug: implements all six. Its own reduced fidelity is bug-540.
- `src/codegen/builtins/term/gen_shared.rs:lower_term_helper` — unaffected: the
  `None` fall-through is the right contract; the defect is that GTK returns
  `None` for a writer.
- `src/target/linux_gtk/app_io.rs:emit_app_term_clear` / `emit_app_term_sync` /
  the GTK write path — unaffected and are the model the six new emitters should
  follow (they already own the cell arrays, the wide sentinel and the pool).
- `examples/wide-demo`, `examples/ai_chat`, `examples/snake`,
  `examples/browser/app` — consumers that are blank in Linux app mode today;
  they become the end-to-end proof, not code to change.
- `tests/syntax/app/macos-app-mode-term` — covers macOS-app and Windows-app
  `.ncodesum` only. A GTK app fixture does not exist and Phase 1 must add one,
  or the six new emitters land with no artifact sentinel at all.

## Fix Design

Port the macOS app bodies, not the console ones. `emit_app_draw_line`,
`emit_app_draw_box`, `emit_app_fill_rect`, `emit_app_draw_glyph` and
`emit_app_draw_text` in `src/target/macos_aarch64/app/app_io.rs` already have the
right *shape* for an app backend — resolve the style ordinal to a unichar from
the shared tables, then stamp into the view's own cells — and the GTK backend
already has the matching cell-array accessors. The console emitters are the
behavioural oracle for clamping/ordering but their grid-header indexing does not
transfer.

The correctness risk concentrates in three places:

1. **Style-table selection.** Six tables, seven variants each; a wrong ordinal
   mapping is invisible in a smoke test that only uses `Light`. The fixture must
   exercise a dash, a dot and `Double`.
2. **The wide/EGC path in `drawText`.** GTK packs the display width into the fg
   word's free bits 27–28 and folds clusters into `ST_TERM_POOL`. Reusing the
   GTK *write* path's cluster walk rather than writing a second one is what keeps
   `drawText` and `io::write` from disagreeing on the same string.
3. **The clamp/clip rules.** These are now specified per member in
   `mfb spec app term-backend` → "Coordinate convention"; implement against that
   table, not against a reading of the console assembly.

Rejected: making the GTK backend allocate a console shadow grid and keep the
fall-through. That doubles the cell storage, needs the console present path
(which writes ANSI to a terminal that is not there), and leaves two grids to keep
in sync.

Expected generated-output shift: none for existing goldens — no current fixture
builds `--app` for a GTK target. Phase 1's new fixture is additive.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a GTK app-mode fixture (a `linux-x86_64`/`linux-aarch64` `--app`
      `.ncodesum` golden alongside the macOS/Windows ones, modelled on
      `tests/syntax/app/macos-app-mode-term`) that calls all six positioned
      helpers with a dash style, a dot style and `Double`. Confirm the dispatcher
      emits the console fall-through today.
- [ ] Add an rt-behavior or `scripts/test-appimage.sh` end-to-end case that runs
      the reproduction on 2228 and captures the window content, and confirm it is
      blank today.
- [ ] Write each blast-radius verdict above into this file after re-running the
      searches.

Acceptance: the new fixture builds and its golden shows the fall-through; the
end-to-end case fails for the documented reason (blank surface, exit 0).
Commit: —

### Phase 2 — the fix

- [ ] Implement `emit_app_term_draw_line` (both orientations),
      `_draw_box`, `_fill_rect`, `_draw_glyph`, `_draw_text` in
      `src/target/linux_gtk/app_io.rs`, and add their six arms to
      `emit_app_term_helper`.
- [ ] Resolve every glyph from the shared
      `crate::codegen::error::constants::TERM_*_CODEPOINTS` tables — no
      hard-coded code points (that is the defect bug-540 records on Windows).
- [ ] Honour the row-before-column argument order and the per-member
      clamp/clip/skip rules from `mfb spec app term-backend`.

Acceptance: the Phase 1 fixture's golden shows GTK bodies; the end-to-end case
paints the same cells as the console build; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/regen-ncodesum.sh` for the new GTK app goldens; confirm the delta
      is only the new fixture and the six new bodies.
- [ ] Delete gap 1 from the `mfb man term` overview and from
      `func_draw_{h,v}line` / `func_draw_box` / `func_fill_rect` /
      `func_draw_text` / `func_draw_glyph`, and update the spec coverage table's
      "Linux GTK app" column.
- [ ] `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
      `scripts/artifact-gate.sh all`, `scripts/man-census.sh --fill term`.
- [ ] Re-run the reproduction on 2228 and 2226.

Acceptance: full suite green; the coverage table and the man pages no longer
disclose a gap that no longer exists; the reproduction paints on both GTK boxes.
Commit: —

## Validation Plan

- Regression test(s): the GTK `--app` `.ncodesum` fixture (artifact gate) plus
  the end-to-end paint check on a real GTK box.
- Runtime proof: the reproduction above on 2228 (x86_64 glibc) and 2226
  (aarch64 glibc) — lowering is not runtime proof for a per-backend change.
- Doc sync: `mfb man term` overview gap 1 and the six member pages;
  `src/docs/spec/app/04_term-backend.md` coverage table and Linux section.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`.

## Open Decisions

- Whether `drawText` reuses the GTK write path's cluster walk directly
  (recommended — one implementation, no drift) or gets its own copy that does not
  advance the shadow cursor. The write path advances the cursor and `drawText`
  must not, so the shared walk needs a "do not commit the cursor" mode.

## Summary

The engineering risk is in the port itself — six new emitters against the GTK
cell arrays, with the style tables and the wide/EGC cluster path as the two
places a plausible-looking implementation silently diverges from the console
oracle. The dispatcher change is one line per member. Nothing outside
`src/target/linux_gtk/` needs to move, and the console/macOS/Windows backends
must be left exactly as they are so they remain the oracle.

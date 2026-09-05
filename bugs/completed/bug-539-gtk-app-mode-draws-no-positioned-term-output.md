# bug-539: Linux `--app` mode silently draws nothing for all six positioned `term::` calls

Last updated: 2026-09-05
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: FIXED
Regression Test: `tests/cli_linux_app_mode.rs` —
`linux_app_mode_positioned_term_members_reach_the_gtk_backend` (RED before),
`linux_app_mode_draw_text_specializes_the_write_path_without_taking_its_cursor`
(RED before), `linux_app_mode_without_term_emits_no_positioned_helpers` (the
positive pin), `linux_app_mode_gtk_term_helpers_are_structurally_sound_on_aarch64`,
`linux_app_mode_no_symbol_address_is_overwritten_by_the_offset_it_needs`
(RED before). Artifact gate:
`tests/syntax/app/macos-app-mode-term/golden/macos_app_mode_term.linux-{x86_64,aarch64}.app.ncodesum`.

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

- [x] Added the two GTK app `.ncodesum` goldens
      (`macos_app_mode_term.linux-x86_64.app.ncodesum` and `.linux-aarch64.app`)
      to the existing `tests/syntax/app/macos-app-mode-term` fixture, which
      already calls all six positioned helpers with `Light`, `HeavyDash` and
      `Double`. This is what puts GTK app codegen under the artifact gate at all
      — it previously had **no** byte-identity coverage on any target. Style
      coverage of a dash, a dot AND `Double` is carried by the new
      `TERM_DRAW_SOURCE` in `tests/cli_linux_app_mode.rs`
      (`LightDash`/`HeavyDot`/`Double`/`Medium`).
- [x] Confirmed the fall-through on the pre-fix compiler, from the codegen dump
      rather than from reading: see "Root cause, confirmed" below.
- [x] Confirmed it end to end on 2228 (Ubuntu/Debian x86_64 GTK): the window
      opens, is cleared, and stays empty; exit 0; no diagnostic.
- [x] Re-ran the blast-radius searches; verdicts recorded below.

Acceptance: met. Commit: (this branch)

### Phase 2 — the fix

- [x] `src/target/linux_gtk/term_draw.rs` gained the worker-side bodies
      `_mfb_gtkapp_term_stamp` / `_run` / `_hline` / `_vline` / `_box` / `_fill` /
      `_glyph` / `_draw_text`, emitted from `emit_term_positioned_helpers()` and
      gated on `AppEntrySpec::uses_term`, and
      `src/target/linux_gtk/app_io.rs:emit_app_term_helper` gained the six arms
      (one shared `emit_app_term_draw` body: the §4.2.1 active gate, a call, the
      OK tag).
- [x] Every glyph is resolved from
      `crate::codegen::error::constants::TERM_{HLINE,VLINE,CORNER_*,FILL}_CODEPOINTS`
      by `emit_select_packed_glyph`, converted to the GTK cell's packed-UTF-8
      form at EMIT time by `pack_codepoint`. No code point is written by hand —
      that is the defect bug-540 records on Windows.
- [x] Row-before-column argument order and the per-member clamp/clip/skip rules
      are taken from `mfb spec app term-backend`; the arms pass the incoming ABI
      argument registers through untouched, so there is no second copy of the
      order to drift.

Acceptance: met.

### Phase 3 — regenerate expected outputs + full validation

- [x] `bash scripts/regen-ncodesum.sh target/release/mfb`: 143 goldens
      refreshed, and `git status` showed **only the two new files** — no existing
      golden's sum moved, which is the containment proof that nothing outside the
      GTK app path changed.
- [x] Deleted the gap-1 disclosure from the `mfb man term` overview (now "Two
      app-mode gaps", renumbered) and from `func_draw_{h,v}line` /
      `func_draw_box` / `func_fill_rect` / `func_draw_text` / `func_draw_glyph`;
      updated the `mfb spec app term-backend` coverage table (the Linux column is
      now `yes` on every row) and its Linux/Unicode sections, plus
      `mfb spec app linux-runtime`'s dispatcher paragraph.
- [x] `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
      `scripts/artifact-gate.sh all` (1910 goldens, 0 diffs),
      `scripts/man-census.sh --fill term` (24/24 complete).
- [x] Re-ran the reproduction on 2228. **Not** re-run on 2226/2225/2224 — see
      "Runtime proof" below.

Acceptance: met.

## Root cause, confirmed

Not read off the source: taken from the pre-fix compiler's own `-ncode` dump of
`tests/syntax/app/macos-app-mode-term` built `-target linux-x86_64 --app`.

- `_mfb_rt_term_term_drawHLine` is 291 instructions — the CONSOLE emitter. Its
  fifth instruction is `ldr_u64 r10, [r15, 3792]` (the `active` gate, which
  passes, because GTK `term::on` does set that slot) and its thirteenth is
  `ldr_u64 r10, [r15, 3840]` — `term_state_offset + TERM_STATE_GRID_OFFSET`,
  the console shadow-grid header — followed by `cmp 0` / `b.eq …_inactive`.
- Sweeping every function in that plan for a **store** to `[r15, 3840]` returns
  exactly one: `_mfb_macapp_program`'s `str_u64 xzr` zero-init. Nothing else in
  the program ever writes it.

So the slot is 0 for the life of a GTK app process and all six positioned members
branch to `_inactive` on their first gate and return `RESULT_OK_TAG`. The failure
is structural, not incidental: the fall-through is *guaranteed* to no-op, and it
reports success while doing it. `drawBox` (1570 instructions), `drawText` (1960),
`drawGlyph` (769), `fillRect` (360) and `drawVLine` (285) are the same shape.

This settles the three candidate explanations the brief named: it is **not** the
backend failing to draw what the term layer produced, and **not** a coordinate
mismatch putting output off-screen. The term layer produced a *console* body for
an *app* build, and that body is dead on arrival.

After the fix the same dump shows each member reduced to 11 instructions — gate,
`bl _mfb_gtkapp_term_hline`, OK tag — and eight new `_mfb_gtkapp_term_*` bodies.
`_mfb_gtkapp_term_write` is **byte-identical** across the change (diffed
function-by-function on the pre- and post-fix plans: 6 functions changed, 8
added, 0 removed, and `_mfb_gtkapp_term_write` in neither list).

## Runtime proof (2228, Ubuntu/Debian x86_64 glibc GTK4, `DISPLAY=:99`)

Screenshots of the real window, captured with `import -window`, compared against
the **console** build of the same program on the same box (its ANSI stream
replayed into a grid).

- The reproduction from this document paints the double box, the text, the heavy
  rule, the medium fill, the snowman and a light-dot vertical rule, at the same
  cells the console build prints — including the detail that the text overruns
  the box's right edge, exactly as the console oracle does.
- The sharper test is `io::write` versus `drawText` **in one frame**: the same
  string written through `term::moveTo` + `io::write` on one row and through
  `term::drawText` on another produced *identical* pixel histograms for both
  bands (`(0,80,8)`×324, `(0,63,6)`×48, `(0,77,8)`×30). Same string, same cells.
- `term::drawText(row, -5, "LEFTCLIPPED")` renders `LIPPED` starting at column 0
  — the left-clip rule — and the shadow cursor stays where `io::write` left it.
- Not executed: **linux-aarch64** GTK app mode. 2226 (Debian aarch64 GTK), 2225
  and 2224 all refused ssh for the duration of this work. The aarch64 plan is
  therefore verified at the lowering level only. As the compensating control,
  `linux_app_mode_gtk_term_helpers_are_structurally_sound_on_aarch64` asserts on
  the **AArch64** plan — the arch where these tokens realize to physical
  `x19`–`x28` with hand-tracked liveness instead of being coloured by the
  allocator — that every branch targets a label the function defines, every frame
  is released on every return path, every frame is a multiple of 16, and the
  pinned arena register `x19` is never borrowed without being saved and restored.

## Bug found and fixed alongside (GTK draw callback, pre-existing)

`_mfb_gtkapp_term_draw` computed the snapshot EGC-pool slot with
`asm.state_array(abi::SCRATCH[0], ST_TERM_SNAP_POOL)`. For an offset past the add
immediate that helper stages the offset in `SCRATCH[0]` *itself*, so the
destination register was overwritten with the offset and the following add
doubled it: the emitted sequence was `adrp x9,state; add_pageoff x9,x9,state;
mov x9,#431304; add x9,x9,x9`, i.e. `2 * ST_TERM_SNAP_POOL + idx*32` as an
absolute address. The GTK **main thread** SIGSEGV'd the first time the renderer
met a cell holding a multi-scalar grapheme cluster.

Reproduced under gdb on 2228 on a binary built from pre-fix `main`
(`Thread 1 … SIGSEGV` at `movzbq 0x0(%r12),%rdx`, deterministic 2/2); clean 3/3
after. Fixed at the call site (`SCRATCH[2]`, dead there) and made
unrepresentable in `Asm::state_array`, which now records a plan-level error for
that aliasing. `linux_app_mode_no_symbol_address_is_overwritten_by_the_offset_it_needs`
pins the whole class and is RED on pre-fix `main`.

## Blast radius (re-run 2026-09-05)

- `src/target/linux_gtk/app_io.rs:emit_app_term_helper` — the bug; six new arms
  plus the shared `emit_app_term_draw` body.
- `src/target/linux_gtk/term_draw.rs` — the eight new worker bodies, the
  `TermWriteMode` specialization of `emit_term_write_helper`, and the
  snapshot-pool address fix.
- `src/target/linux_gtk/mod.rs` — the eight new symbol constants, the
  `uses_term`-gated emission in both `emit_app_program_entry` and its x86 twin,
  and the `Asm::state_array` guard.
- `src/codegen/term/core/term.rs` — `emit_encode_utf8` is now `pub(crate)` so the
  GTK `drawGlyph` body reuses the console's runtime UTF-8 encoder rather than
  copying it. Visibility only; no emitted byte moves (the artifact gate's 1910
  goldens confirm).
- `src/target/macos_aarch64/app/app_io.rs` — untouched, and its `.ncodesum`
  goldens are byte-identical.
- `src/target/win_x86_64/app/mod.rs` — untouched, goldens byte-identical. Its own
  reduced fidelity is bug-540, which now carries a note pointing at
  `emit_select_packed_glyph` and `TermWriteMode` as the worked precedent for
  WIN-01 and WIN-04.
- `src/codegen/builtins/term/gen_shared.rs:lower_term_helper` — unchanged. The
  `None` fall-through contract is still right for the pure readers; the defect
  was GTK returning `None` for a *writer*.
- `examples/wide-demo`, `examples/ai_chat`, `examples/snake`,
  `examples/browser/app` — unchanged; they are consumers that now paint.

## Validation Plan

- Regression test(s): the GTK `--app` `.ncodesum` fixture (artifact gate) plus
  the end-to-end paint check on a real GTK box.
- Runtime proof: the reproduction above on 2228 (x86_64 glibc) and 2226
  (aarch64 glibc) — lowering is not runtime proof for a per-backend change.
- Doc sync: `mfb man term` overview gap 1 and the six member pages;
  `src/docs/spec/app/04_term-backend.md` coverage table and Linux section.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`.

## Open Decisions — resolved

- **`drawText` reuses the write path's cluster walk.** Not by a runtime "do not
  commit the cursor" flag, which would have put five branches through a
  651-instruction body and changed the bytes `io::write` emits, but by
  specializing the *emitter*: `emit_term_write_helper(uses_term, mode)` produces
  `_mfb_gtkapp_term_write` for `TermWriteMode::Write` and
  `_mfb_gtkapp_term_draw_text` for `TermWriteMode::DrawText`. The two differ only
  where a run meets an edge (start cell, left/right clip vs wrap, wide-at-edge
  drop vs wrap, control byte, scroll, cursor commit — tabulated on the enum); the
  decode, the `charwidth` lookup, the combining-mark fold into the EGC pool, the
  `GTK_WIDE_TRAIL` sentinel and the attribute packing are the same emitted
  instructions. The `Write` body came out byte-identical, which was verified
  rather than assumed.

## Summary (as filed)

The engineering risk is in the port itself — six new emitters against the GTK
cell arrays, with the style tables and the wide/EGC cluster path as the two
places a plausible-looking implementation silently diverges from the console
oracle. The dispatcher change is one line per member. Nothing outside
`src/target/linux_gtk/` needs to move, and the console/macOS/Windows backends
must be left exactly as they are so they remain the oracle.

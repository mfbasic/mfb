# term:: Shadow-Grid Unification Plan

Last updated: 2026-07-11
Effort: huge (>3d)

> **This is the shared design of record.** Per the write-plan effort rule this
> **huge** feature is split into six small/medium sub-plans — build from those,
> not this file's `## Phases` (kept below as the overview):
>
> - `plan-35-A-sync-surface-and-model.md` — `term::sync` surface + shadow model
>   scaffold (no rendering change). *Depends on nothing.*
> - `plan-35-B-console-grid-write.md` — console `on` allocates the grid; drawing
>   + `io::write` mutate it; temp full-repaint present. *Depends on A.*
> - `plan-35-C-console-diff-present.md` — console front/back diff in `sync`; `off`
>   final present (the crux). *Depends on B.*
> - `plan-35-D-app-macos-present.md` — macOS present-driven redraw + resize.
>   *Depends on A.*
> - `plan-35-E-app-gtk-present.md` — GTK present-driven redraw + close tearing +
>   resize. *Depends on A.*
> - `plan-35-F-migration-validation.md` — migrate `life`, test matrix, docs.
>   *Depends on B, C, D, E.*
>
> Build order: A → then B/D/E in parallel → C (after B) → F last. The three
> gating decisions (D1–D3) are **resolved** (see §Open Decisions).

Make `term::` a double-buffered TUI surface with a single programming model
across the console (CLI) and windowed app backends. While TUI mode is on, every
glyph — from `io::print`/`io::write` and from `term::` drawing calls — mutates an
in-memory **cell grid** rather than the terminal; a new **mandatory**
`term::sync()` **presents** the grid by emitting the minimal set of updates: a
front/back-buffer **diff** to ANSI on the console, a coalesced redraw of the
view-resident grid in app mode. Nothing appears until the program calls
`term::sync()` (and `term::off` implies a final present); a program that draws
without a following `sync()` shows nothing — this is the retained-mode contract,
and existing TUI programs are updated to call it (backward compatibility is a
non-concern here, D1).
The single behavioral outcome a correct implementation produces: **a full-screen
program that repaints every frame (e.g. `examples/life`) shows no flicker and,
on the console, emits only the cells that actually changed between frames** —
while every existing non-TUI program's output is byte-for-byte unchanged.

References (read first):

- `mfb spec app term-backend` — `src/docs/spec/app/04_term-backend.md` (the cell
  model, term-state global, app-mode grid storage, content-view swap).
- `mfb spec app console-io` — `src/docs/spec/app/03_console-io.md`.
- `mfb man term` — `src/docs/man/builtins/term/package.md` and per-function pages.
- Memory: `term:: module progress`, `plan-13 app:: GUI design`
  (poll-FALSE-before-first-sync + timed-poll flush — the precedent for
  auto-present), `keyboard input needs Enter` (bug-149/150 — the input path this
  interacts with), `bug-148 loop-trap` (life ships an inline-TRAP workaround).

## 1. Goal

- With TUI mode on, `io::print`/`io::write` and `term::moveTo`/`setForeground`/
  `setBackground`/`setBold`/`setUnderline`/`clear`/`showCursor`/`hideCursor`
  update an in-memory cell grid (glyph + fg + bg + bold + underline per cell) and
  a shadow cursor + current-attribute set, touching the real terminal on **no**
  call except a present.
- A new builtin **`term::sync()`** presents the grid: on the console it diffs the
  back buffer against the last-presented front buffer and writes only the changed
  cells (cursor moves + SGR runs + glyphs); in app mode it triggers a single
  coalesced redraw of the view grid. It is a no-op while TUI mode is off.
- `term::sync()` is the **only** thing that presents a frame (D1: mandatory).
  `term::off` implies a final present before restoring the screen. No implicit
  auto-present at input boundaries.
- `examples/life` is updated to call `term::sync()` once per frame and shows no
  flicker; on the console a steady-state frame (few live cells changing) emits
  O(changed cells) bytes, not a full-screen repaint.
- Byte-for-byte identical output for every program that never calls `term::on`.

### Non-goals (explicit constraints)

- **No change to the `term::` language surface** other than *adding* `term::sync`
  (and, if D1 picks it, keeping the existing calls' signatures). Names, arities,
  argument/return types, and the `TermColor`/`TermSize` record layouts of every
  existing call stay exactly as declared in `src/builtins/term.rs`.
- **No change to non-TUI output.** Programs that never call `term::on` must
  produce identical bytes on stdout/stderr; the console `io::write` fast path
  (`ARENA_OUT_ENABLED` buffering + direct `emit_write`) is untouched while TUI is
  off. This is a byte-gate obligation (`scripts/artifact-gate.sh`).
- **No change to the term-state global's existing 8 slots' meaning** (active/fg/
  bg/bold/underline/cursorVisible at offsets 0..40). New shadow state uses the
  two reserved slots (48, 56) and/or a newly-allocated grid block; the offsets
  and defaults other backends already read stay put
  (`src/docs/spec/app/04_term-backend.md` §Shared term-state global).
- **No new leaks.** The grid/back+front buffers allocated on `term::on` must be
  freed on `term::off` (and at shutdown if `off` is skipped). Correctness over
  performance — never trade a leak for a faster present.
- **No `-app` behavioral regression.** The macOS `TermView` and GTK
  `GtkDrawingArea` surfaces must render the same content they do today; this plan
  changes *when* they repaint (coalesced to present), not *what* they show.
- **Determinism preserved.** Output stays byte-deterministic across the four
  targets and re-runs (the `bug-87` guarantee).

## 2. Current State

The two backends are **asymmetric**: app mode is already a retained cell grid;
the console is pure immediate ANSI with no grid and no cursor tracking. This plan
gives the console a grid and unifies the *model*, not a single physical buffer
(app keeps its view-resident grid — see §3).

### 2.1 Shared term-state global (both backends)

Eight `u64` slots in the program-entry frame, reached off the pinned arena-state
register (`x19`, `ARENA_STATE_REGISTER`) at `term_state_offset + field`:
active(0), fg(8), bg(16), bold(24), underline(32), cursorVisible(40),
reserved(48), reserved(56). Defaults: fg = white (16777215), bg = 0, cursor
visible. `src/docs/spec/app/04_term-backend.md` §Shared term-state global;
offsets `TERM_STATE_*_OFFSET` referenced throughout
`src/target/shared/code/term.rs`.

### 2.2 Console backend — immediate ANSI, no grid

`src/target/shared/code/term.rs` (`lower_term_helper`): each helper is a
self-contained routine emitting fixed ANSI byte strings to fd 1 via
`platform.emit_write`. `term::on` writes `ESC[?1049h…2J…H` (alt-screen + reset +
clear + home); `moveTo` emits `ESC[<row>;<col>H` **straight through** (no cursor
is remembered); `setForeground`/`setBackground` emit SGR truecolor and store the
packed value in the global; `clear` emits `ESC[2J ESC[H`. **There is no cell
grid and no shadow cursor** — the terminal is the only state.

Console text output is term-**unaware**: `lower_io_write_helper`
(`src/target/shared/code/io_helpers.rs:262`) stages fd/ptr/len and calls
`platform.emit_write` (→ libc `write(2)`, `src/target/macos_aarch64/code.rs:232`),
with an optional per-arena stdout buffer (`ARENA_OUT_ENABLED`, plan-14-A). It
never consults `TERM_STATE_ACTIVE`. So while TUI is on, glyphs land on the
terminal directly, positioned only by whatever `moveTo` ANSI preceded them.

### 2.3 App backend — already a retained grid (the precedent to mirror)

- **macOS** (`src/target/macos_aarch64/app/term_view.rs`,
  `.../app/app_io.rs`): a `calloc`'d `TermCell[16B]` grid (glyph u32, fg u32,
  bg u32, bold u8, underline u8) plus a 96-byte `TVSTATE` associated object
  holding cursor row/col, cell metrics, and current fg/bg/bold/underline.
  `mfbWriteString:` (`_mfb_macapp_term_writeString`) writes glyphs into the grid
  at the cursor, wraps/scrolls, then `setNeedsDisplay:`. `drawRect:` paints the
  whole grid. Grid is **not** resized on live window resize.
- **GTK** (`src/target/linux_gtk/term_draw.rs`, `.../app_io.rs`): three parallel
  static arrays (chars u8, fg u32, bg u32) at stride 160×48; flags packed into
  the fg/bg words (COLOR_SET bit 24, bold 25, underline 26). `_mfb_gtkapp_term_write`
  mutates the arrays and `g_idle_add`s a redraw. Documented **tearing** caveat:
  the worker mutates the grid and only *requests* a main-thread redraw, so a
  concurrent draw can show a torn frame (`term_draw.rs:513-528`).
- **App io routing already gates on `active`:** `emit_app_io_write_helper`
  (macOS `app/app_io.rs:13`, GTK `app_io.rs:369`) loads `TERM_STATE_ACTIVE` and,
  when set, routes the string into the grid writer; otherwise to the transcript
  view; else to fd. **This is exactly the "route glyphs through the grid while
  on" behavior the console lacks** — the console change in Phase B is the mirror
  of code app mode already ships.
- **No explicit present:** redraw is eager per write (`setNeedsDisplay:` /
  `g_idle_add(queue_draw)`). `io::flush` in app mode is a **no-op** returning OK
  (macOS `app_io.rs:211`, GTK `app_io.rs:485` — a labeled SCAFFOLD). This is the
  natural seam a present/sync hooks into.

### 2.4 Dispatch (console vs app)

Single fork in `lower_runtime_helper` (`src/target/shared/code/mod.rs`):
`app_mode = build_mode.is_app()` (`:1110`). Each io/term helper does
`if app_mode { platform.emit_app_*(…) } else { lower_*_console(…) }`
(io at `:1246-1290`, term at `:1118`). App bodies are per-target
`CodegenPlatform` trait methods that return `None` when unsupported; `isOn` and
the attribute getters return `None` so both backends share the console reader
off the global.

### 2.5 The consumer

`examples/life/src/main.mfb` is the real full-screen program: each frame it
`term::moveTo(y,0)` + `io::write(row)` for every row (`drawGrid`, :208), draws a
status bar, and `term::clear()`s only when `dirty`. It **has no `sync()` call**
today and relies on immediate rendering — Phase F edits it to call `term::sync()`
once per frame (D1 makes `sync` mandatory; backward compat is a non-concern). It
queries `term::terminalSize()` each loop and reads keys via `io::readChar()`.

### 2.6 Registration surface for a new `term::` builtin

Adding `term::sync` touches, at minimum: `src/builtins/term.rs` (const +
`is_term_call` + `param_types`/`arity`/`call_return_type_name` +
`call_param_names`), `src/target/shared/runtime/term_specs.rs` (a
`TERM_SYNC_SPEC`), `src/target/shared/runtime/catalog.rs` (register it),
`src/target/shared/code/term.rs` (console emit), the two `emit_app_term_helper`
match tables (macOS `app/app_io.rs:712`, GTK `app_io.rs:9`), and the consumers
that enumerate term calls (`src/syntaxcheck/builtins.rs`,
`src/syntaxcheck/helpers.rs`, `src/resolver/mod.rs`, `src/monomorph/lower.rs`,
`src/ir/lower.rs`, `src/ir/verify/mod.rs`). Docs: a man page
`src/docs/man/builtins/term/sync.txt` + `package.md` synopsis
(`scripts/update_man.sh`), and `src/docs/spec/app/04_term-backend.md`.

## 3. Design Overview

**What "unify" means here.** The app grid lives inside the native view
(`TermView`/`GtkDrawingArea`), unreachable from MFBASIC memory; the console has
no grid at all. So we do **not** create one physical buffer shared by both. We
unify the **model and semantics**:

- a **cell** = glyph + fg + bg + bold + underline (the union of what both
  backends already store);
- a **shadow cursor** (row, col) and **current-attribute set** (fg/bg/bold/
  underline) that drawing calls mutate;
- **coordinate/scroll/clamp/wrap** rules identical on both backends;
- a **present** operation (`term::sync`, mandatory — D1) that is the *only* thing
  that touches the terminal / requests a redraw; `term::off` implies a final
  present.

Three layers, landed lowest-risk first:

1. **Surface + policy (Phase A).** Add `term::sync` end-to-end as a present hook
   (initially a no-op) and define the cell/grid model and where the console grid
   lives (D2: an arena-allocated header block). No rendering behavior changes
   yet — pure scaffolding + docs. Safe to land alone.
2. **Console retained model (Phases B, C — the crux).**
   - *B:* make console `io::write`/`term::moveTo`/`setColor`/`setAttr`/`clear`/
     cursor-visibility mutate the shadow grid + cursor + current attrs instead of
     emitting ANSI. `term::on` allocates back+front buffers and clears; `term::off`
     final-presents, restores, and frees.
   - *C:* implement the **diff presenter** in `term::sync`: back-vs-front buffer
     diff → minimal ANSI (cursor CUP + SGR run-coalescing + glyphs), then copy
     back→front; `term::off` runs a final present. This is where correctness risk
     concentrates.
3. **App convergence (Phases D, E).** Coalesce the eager per-write redraw into
   the present: `term::sync` triggers the single redraw; per-write
   `setNeedsDisplay:`/`g_idle_add` is removed in favor of present-driven redraw;
   `term::off`/input-boundary auto-present flush pending frames. macOS also gains
   grid resize-on-window-resize (currently fixed at init). GTK's tearing caveat
   is closed by presenting a marshaled snapshot. D = macOS, E = GTK.
4. **Migration + validation (Phase F).** Confirm/adjust `examples/life`, add
   `func_term_*` tests (incl. a diff-minimality assertion), run the byte-gate and
   cross-target acceptance, and finish doc/spec sync.

**Where the risk is:** the console diff presenter (Phase C) — SGR minimization,
unicode display-width in the grid writer (wide/zero-width cells), the
last-cell-of-last-row scroll hazard `life` already dances around
(`drawStatus` writes `cols-1`), and cursor-visibility during a present. Second:
the app on/off/resize flush ordering (Phases D/E) and the GTK marshaled-snapshot
present. The surface work (A) and the write→grid redirect (B) are mechanical
mirrors of code app mode already ships.

**Rejected alternatives:**

- *Keep console immediate; only buffer + skip full clears.* Kills some flicker
  but gives no diffing and does not unify the backends — a stopgap, not the
  goal (§1). Rejected.
- *One physical grid shared by console and app.* The app grid is view-resident
  native state; hoisting it into MFBASIC-visible memory would rewrite both
  native renderers for no user-visible gain. Rejected in favor of a shared
  *model* with two presenters.
- *Auto-present at input/`off` boundaries so programs need no `sync()`.*
  Rejected (D1): backward compatibility is a non-concern here, and a mandatory
  explicit `sync()` gives a simpler, more predictable renderer with one present
  path. Existing TUI programs are edited to call `sync()`.

## 4. Detailed Design

### 4.1 Cell + grid model (shared semantics)

Cell fields: `glyph` (u32 unichar; 0/space = blank), `fg`/`bg` (packed
`r|g<<8|b<<16`, the existing console convention), `bold`, `underline`. Console
back+front buffers are `rows*cols` cells each. The macOS `TermCell` (16B) is
already this shape; GTK packs flags into fg/bg — the shared *semantics* are what
must match, not the byte layout (§Non-goals allow per-backend storage).

Shadow cursor `(row, col)` zero-based from top-left; current attributes fg/bg/
bold/underline. `moveTo` sets the cursor (clamped ≥0; app also clamps to the last
cell); writing a glyph advances the cursor, wraps at the right edge, and scrolls
at the bottom (mirroring `mfbWriteString:` today). Writing never emits — it only
mutates cells at the cursor using the current attributes.

### 4.2 Where the console grid lives

**D2 (resolved): one arena-allocated header block, one reserved slot.** The
console needs six persistent values it lacks today — `back*`, `front*`, `rows`,
`cols`, `cursorRow`, `cursorCol` (fg/bg/bold/underline/cursorVisible already have
global slots and become the current-attribute set). Two 64-bit pointers alone
fill both reserved slots (48, 56), so the pack-into-slots alternative doesn't fit
— a header block is required regardless. `term::on` allocates one arena block
laid out `[rows, cols, cursorRow, cursorCol | back cells… | front cells…]` sized
to `terminalSize()` and stores its base pointer in reserved slot 48 (slot 56 left
free for future state — scroll region, damage rect). Allocation uses the arena
allocator (`ARENA_ALLOC_SYMBOL`, already called from `term.rs` for
`TermColor`/`TermSize`). `term::off` frees the block and zeroes the slot; shutdown
teardown (`_mfb_shutdown`) frees if `off` was skipped. On terminal resize between
frames, the present reallocs the block and forces a full repaint — `life`
re-queries size each loop.

### 4.3 Console present (diff → ANSI) — Phase C, the crux

`term::sync` walks the back buffer against the front buffer (`term::off` runs the
same present once, then restores):

- Skip unchanged cells. For each run of changed cells: emit one CUP
  (`ESC[r;cH`) to the run start (or a bare cursor move if adjacent), switch SGR
  only when fg/bg/bold/underline differ from the last-emitted attributes
  (coalesce SGR across a run), then emit the glyph bytes.
- After the diff, restore the shadow cursor position and visibility, flush the
  stdout buffer, and `memcpy` back→front.
- Full-repaint fallback on first present after `on`/resize (front buffer all
  "dirty"). The alt-screen clear stays in `on`.

**D3 (resolved): emit the grid writer + diff presenter as neutral `abi::`
codegen** in `term.rs`/`io_helpers.rs`, matching today's backend style — one
implementation across aarch64/x86/riscv, no new package-boot machinery. (The
MFBASIC-source-package alternative is not taken.)

### 4.4 Console `io::write` becomes term-aware — Phase B

`lower_io_write_helper` (`io_helpers.rs:262`) gains a leading branch: if
`TERM_STATE_ACTIVE != 0`, route the string into the grid writer (glyph-by-glyph
with unicode display-width via the existing `unicode_backend`) at the shadow
cursor instead of `emit_write`; else the current fd path, unchanged (byte-gate).
This is the direct mirror of `emit_app_io_write_helper`'s `active` branch.

### 4.5 App present convergence — Phases D/E

Replace eager per-write redraw with present-driven redraw: the grid writer still
mutates cells but stops calling `setNeedsDisplay:` / `g_idle_add(queue_draw)`;
`term::sync`, the input-boundary auto-present, and `term::off` request the single
redraw. `io::flush` (today a no-op) becomes the present in app mode. macOS adds
grid resize on window resize (recompute rows/cols, realloc `TermCell[]`, force
full redraw). GTK closes the tearing caveat by presenting a marshaled snapshot on
the main loop rather than letting the worker race the draw.

## Compatibility / Format Impact

- **Language surface:** adds `term::sync() AS Nothing`. All existing `term::`
  signatures and the `TermColor`/`TermSize` records are unchanged. `.mfp` ABI
  index gains one call entry (a normal additive change; bump handled by the
  existing package-ABI machinery).
- **term-state global:** the two reserved slots (48, 56) gain defined meaning
  (grid header/pointers); the eight existing slots' offsets and defaults are
  unchanged, so cross-backend readers (`isOn`, getters) are unaffected.
- **Observable console output while TUI on:** changes from "full rows every
  frame" to "changed cells only." This is the intended behavior change and is
  **not** byte-gated (the byte-gate covers TUI-**off** programs). A
  golden/behavioral test asserts the new minimal-diff output (Phase F).
- **Unchanged:** all non-TUI stdout/stderr bytes; app-mode rendered content;
  determinism across targets.

## Phases

> Each phase below is a `plan-35-<letter>` sub-plan. Fill `Commit:` as each lands.

### Phase A — `term::sync` surface + shadow model + auto-present policy

Low-risk scaffold: the new builtin exists and is wired end-to-end as a present
hook (no-op until B/C), the cell/grid model and grid-storage location are
specified, and the auto-present policy is locked. No rendering change.

- [ ] Declare `term::sync` in `src/builtins/term.rs` (const, `is_term_call`,
      `call_param_names`/`param_types`/`arity`/`call_return_type_name` → 0-arg,
      `Nothing`) and update the module's unit tests (`ALL`, `NO_ARG`).
- [ ] Add `TERM_SYNC_SPEC` in `src/target/shared/runtime/term_specs.rs` and
      register it in `src/target/shared/runtime/catalog.rs`.
- [ ] Add a no-op console emit arm for `term.sync` in
      `src/target/shared/code/term.rs` and no-op app arms in both
      `emit_app_term_helper` tables (macOS `app/app_io.rs`, GTK `app_io.rs`).
- [ ] Thread `term.sync` through the term-call enumerators in
      `src/syntaxcheck/builtins.rs`, `src/syntaxcheck/helpers.rs`,
      `src/resolver/mod.rs`, `src/monomorph/lower.rs`, `src/ir/lower.rs`,
      `src/ir/verify/mod.rs`.
- [ ] Define the cell model, shadow cursor/current-attrs, and console
      grid-storage layout (reserved slots vs header block, D2) in
      `src/docs/spec/app/04_term-backend.md`; write `sync.txt` man page +
      `package.md` synopsis (`scripts/update_man.sh`).
- [ ] Tests: `tests/syntax/term/*` accepts `term::sync()`; a `func_term_sync_valid`
      that calls it while on/off and returns cleanly (no-op OK).

Acceptance: `term::sync()` compiles, type-checks, and runs as a no-op on console
and `-app` (both targets); `mfb man term sync` renders; byte-gate green (no
codegen change for existing calls). Commit: —

### Phase B — Console: route drawing + text into the shadow grid

Console `term::on` allocates back+front buffers; `io::write`, `moveTo`,
`setColor`, `setAttr`, `clear`, `showCursor`/`hideCursor` mutate the grid /
cursor / current attrs instead of emitting ANSI. Rendering still happens via a
*temporary* full-repaint present so behavior is observable before C. Depends on A
and Open Decisions D2/D3.

- [ ] `term::on`/`off` in `src/target/shared/code/term.rs`: allocate/clear
      back+front buffers sized to `terminalSize()`; free on `off`; register
      shutdown free.
- [ ] Console grid writer + term-aware `io::write` in
      `src/target/shared/code/io_helpers.rs` (branch on `TERM_STATE_ACTIVE`),
      with unicode display-width via `unicode_backend`; wrap/scroll matching
      `mfbWriteString:`.
- [ ] Convert `moveTo`/`setForeground`/`setBackground`/`setBold`/`setUnderline`/
      `clear`/`showCursor`/`hideCursor` in `term.rs` to grid/cursor/attr mutation
      (no direct ANSI).
- [ ] Tests: `func_term_write_grid_*` (cursor advance, wrap, scroll, clamp);
      unchanged non-TUI byte-gate.

Acceptance: a console program that draws via `io::write`+`moveTo` and calls
`term::sync()` shows correct content (via the temp full-repaint present); a
program that never calls `term::on` is byte-identical (`artifact-gate.sh`).
Commit: —

### Phase C — Console: diff presenter in `term::sync` + `term::off` final present (highest-risk)

Replace the temp full-repaint with the real front/back diff; `term::off` runs a
final present before restoring the screen. Depends on B.

- [ ] Diff renderer in `term.rs`, invoked by `term::sync`: run-detection, CUP
      minimization, SGR coalescing, glyph emit, cursor restore, stdout flush,
      back→front copy; first-present/resize full-repaint fallback.
- [ ] `term::off` calls the same present once (final frame), then emits the
      alt-screen restore.
- [ ] Tests: `func_term_diff_minimal_*` asserting a steady-state frame emits only
      changed-cell bytes (capture stdout under a PTY); resize forces full repaint;
      `term::off` flushes the last frame.

Acceptance: `examples/life` on the console shows no flicker and a steady frame
emits O(changed cells) bytes (asserted by capturing the escape stream);
`term::off` leaves a correct final frame then restores the user screen.
Commit: —

### Phase D — App (macOS): present-driven redraw + resize

Coalesce `TermView` redraw into the present; add grid resize on window resize.
Depends on A (independent of B/C at the source level).

- [ ] `term::sync` (and `term::off`'s final present) drives `setNeedsDisplay:`;
      remove per-write redraw in `_mfb_macapp_term_writeString` (`term_view.rs`).
- [ ] Resize handler: recompute rows/cols, realloc `TermCell[]`, force full
      redraw (`term_view.rs` / `app/app_io.rs`).
- [ ] `term::off` presents the final frame before the content-view swap
      (`app/app_io.rs`).
- [ ] Tests: `-app` run of `examples/life` (macOS) renders correctly and resizes.

Acceptance: `mfb build -app` life on macOS renders identically to today, repaints
once per present (not per write), and reflows on window resize. Commit: —

### Phase E — App (GTK): present-driven redraw + close tearing

Mirror D for GTK; present a marshaled snapshot to end the worker/draw race.
Depends on A.

- [ ] Coalesce `_mfb_gtkapp_term_write`'s per-write `g_idle_add(redraw)` into a
      present-triggered redraw (`term_draw.rs` / `app_io.rs`).
- [ ] Present a main-loop snapshot to close the tearing caveat
      (`term_draw.rs:513-528`); grid resize on window resize (also clears the
      Phase-6 SCAFFOLD gap noted in the spec).
- [ ] Tests: `-app` run of `examples/life` (linux) renders without tearing.

Acceptance: `mfb build -app` life on Linux renders without a torn frame, repaints
once per present, and reflows on resize. Commit: —

### Phase F — Migration, validation, docs

- [ ] Edit `examples/life/src/main.mfb` to call `term::sync()` once per frame
      (after `drawGrid`/`drawStatus`, before the input read) — required now that
      `sync` is mandatory (D1). Sweep any other `term::` example for the same.
- [ ] Full `func_term_*` pass incl. diff-minimality and on/off flush; run
      `scripts/artifact-gate.sh`, `scripts/test-accept.sh`, and cross-target
      (x86 + riscv via ssh 2229) + both `-app` backends.
- [ ] Final `mfb spec app term-backend` + `console-io` + man sync.

Acceptance: all acceptance suites green on all four targets and both app
backends; docs describe the shadow-grid/present model. Commit: —

## Validation Plan

- **Tests:** `tests/func_term_*` (grid write/wrap/scroll/clamp, sync no-op,
  diff-minimality under PTY capture, on/off flush, resize repaint) and
  `tests/syntax/term/*` (accepts `term::sync`). Negative: `term::sync` while off
  is a clean no-op; unsupported-while-off semantics unchanged for
  `terminalSize`.
- **Runtime proof:** `examples/life` on the console (capture the escape stream:
  steady-state frame = only changed cells) and under `mfb build -app` on macOS
  and Linux (no flicker/tearing, resize reflows).
- **Byte-gate:** `scripts/artifact-gate.sh` proves every TUI-**off** program is
  byte-identical; determinism across the four targets (bug-87 guarantee).
- **Doc sync:** `src/docs/spec/app/04_term-backend.md`, `03_console-io.md`, and
  `src/docs/man/builtins/term/{sync.txt,package.md}` — obligatory per AGENTS.md.
- **Acceptance:** `scripts/test-accept.sh` + cross-target ssh (x86, riscv 2229) +
  both `-app` builds.

## Open Decisions

All three gating decisions are **resolved** (2026-07-11); no open forks remain.

- **D1 — Auto-present policy → RESOLVED: `sync()` mandatory.** `term::sync()` is
  the only explicit present; `term::off` implies a final present. No implicit
  auto-present. Backward compatibility is a non-concern, so existing TUI programs
  (`examples/life`) are edited to call `sync()`. Simpler, single-path renderer.
  (§1, §4.3, Phase C/F)
- **D2 — Console grid storage → RESOLVED: one arena header block, one reserved
  slot.** Layout `[rows, cols, cursorRow, cursorCol | back cells… | front cells…]`,
  base pointer in reserved slot 48. Chosen because the console needs six
  persistent values and two 64-bit pointers alone fill both reserved slots — the
  pack-into-slots alternative does not fit, so a header block is required
  regardless. (§4.2)
- **D3 — Console renderer implementation → RESOLVED: neutral `abi::` codegen.**
  Emit the grid writer + diff presenter in `term.rs`/`io_helpers.rs`, matching
  today's backend — one implementation across aarch64/x86/riscv, no new
  package-boot machinery. The MFBASIC-source-package alternative is not taken.
  (§4.3)

## Summary

The engineering risk is concentrated in **Phase C** (the console front/back diff
renderer: SGR minimization, unicode display-width, the last-cell scroll hazard,
cursor handling) and secondarily in the **app present/flush/resize ordering**
(Phases D/E) and GTK's marshaled-snapshot present. Everything else is mechanical:
the surface work (A) is registration, and routing console `io::write` through the
grid (B) is a direct mirror of the `active`-gated code app mode already ships.
Untouched: all non-TUI output (byte-gated), the existing `term::` signatures and
record layouts, the eight established term-state slots, and cross-target
determinism. The whole feature is **huge** and must be split into
`plan-35-A … F` before implementation. **D1–D3 are resolved** (sync mandatory;
one arena header block; `abi::` codegen), so Phase A can begin once the plan is
split.

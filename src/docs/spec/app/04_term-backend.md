# App-Mode Terminal Backend

The GUI rendering of the `term::` TUI: a fixed character grid painted on a native
drawing surface that is swapped in as the window content view while TUI mode is
active. This documents the cell model, the grid-state memory layout, the
content-view swap, and how the GUI `term::` helpers keep the **same** term-state
global that the console backend uses — so `term::isOn`, the no-op gates, and
auto-restore stay backend-uniform. Per-function semantics (`term::on`,
`term::moveTo`, …) are owned by `mfb man`; this topic is the rendering/cell-model
contract a reimplementer rebuilds against.

Two independent backends exist: AppKit `TermView : NSView` (macOS) and a GTK4
`GtkDrawingArea` + Cairo surface (Linux). They share the *console* term-state
global and the packed-RGB colour convention, but their per-view grid storage
differs (heap `TermCell[]` vs. parallel static arrays). Both are described below;
divergences are flagged.

## Shared term-state global (console-uniform)

The GUI setters write the same per-program TUI slots the console backend reads.
These live in the program-entry frame just past the program globals/`LINK` slots,
reached off the pinned arena-state register `x19` at `term_state_offset + field`.
Eight `u64` slots, zero-initialized (the inert TUI-off default). [[src/target/shared/code/error_constants.rs:TERM_STATE_ACTIVE_OFFSET]]

| Field | Offset | Meaning |
|-------|--------|---------|
| active | 0 | 1 while `term::on`; gates no-ops and auto-restore |
| fg | 8 | packed `r \| g<<8 \| b<<16` foreground (default 16777215 = white) |
| bg | 16 | packed background (default 0 = black) |
| bold | 24 | bold flag |
| underline | 32 | underline flag |
| cursorVisible | 40 | cursor-visible flag |
| (reserved) | 48, 56 | two reserved slots for the app backend |

The GUI `term::on`/`off`/`setForeground`/`setBackground`/`setBold`/
`setUnderline`/`showCursor`/`hideCursor`/`moveTo`/`clear` helpers update these
exactly as the console backend would, then additionally drive the surface (below).
Pure readers — `term::isOn` and the attribute getters — keep the shared console
implementation; the app dispatcher returns `None` for them so they read the global
the setters maintain. [[src/target/macos_aarch64/app/app_io.rs:emit_app_term_helper]]

`store_term_state` is the one-line writer: `mov x9, #value; str x9, [x19,
term_state_offset+field]`. [[src/target/macos_aarch64/app/app_io.rs:store_term_state]]

## macOS: `TermView : NSView`

### Class synthesis

The `_main` bootstrap synthesizes `TermView` at runtime via
`objc_allocateClassPair(NSView, "TermView", 0)` (zero extra instance bytes — the
grid state is a separate `calloc`'d buffer, attached as an associated object,
because `object_getIndexedIvars` storage is not reliably backed for
runtime-synthesized classes). Five methods are added, then
`objc_registerClassPair`. [[src/target/macos_aarch64/app/bootstrap.rs:emit_main_bootstrap]]

| Selector | Type encoding | IMP symbol |
|----------|---------------|------------|
| `drawRect:` | `v@:{CGRect=dddd}` | `_mfb_macapp_term_drawRect` |
| `isFlipped` | `c@:` | `_mfb_macapp_term_isFlipped` |
| `mfbWriteString:` | `v@:@` | `_mfb_macapp_term_writeString` |
| `acceptsFirstResponder` | `c@:` | `_mfb_macapp_term_acceptsFR` |
| `keyDown:` | `v@:@` | `_mfb_macapp_term_keyDown` |

`isFlipped` returns YES so row 0 is at the top and cell `(row, col)` maps to
`(col*cellW, row*cellH)` in the flipped space. `acceptsFirstResponder` returns
YES so the surface can take keyboard focus while TUI mode is active. Both are
constant `mov x0, #1; ret` stubs. [[src/target/macos_aarch64/app/term_view.rs:emit_term_view_is_flipped]] [[src/target/macos_aarch64/app/term_view.rs:emit_term_accepts_first_responder]]

After registration the bootstrap allocates one instance
(`initWithFrame:NSMakeRect(0,0,900,640)`), sets
`autoresizingMask = NSViewWidthSizable|NSViewHeightSizable`, runs
`_mfb_macapp_term_init` to size + allocate the grid, then stashes the instance on
NSApp under `_mfb_macapp_termview_key` (OBJC_ASSOCIATION_ASSIGN; the alloc +1
keeps it alive). The window, the transcript scroll view, and the transcript text
view are likewise stashed under `_mfb_macapp_window_key`,
`_mfb_macapp_scrollview_key`, and `_mfb_macapp_textview_key`. [[src/target/macos_aarch64/app/bootstrap.rs:emit_main_bootstrap]]

### Grid-state struct (`TVSTATE` associated object)

A `calloc`'d buffer attached to the view via `objc_setAssociatedObject(view,
&TVSTATE_KEY, state, OBJC_ASSOCIATION_ASSIGN)` — a plain C buffer the runtime
never messages. Twelve 8-byte fields = 96 bytes (`TV_STATE_SIZE`). [[src/target/macos_aarch64/app/term_view.rs:emit_term_init_helper]]

| Field | Offset | Type | Meaning |
|-------|--------|------|---------|
| `TV_CELLS` | 0 | `TermCell*` | heap grid, `rows*cols` cells |
| `TV_ROWS` | 8 | i64 | row count |
| `TV_COLS` | 16 | i64 | column count |
| `TV_CURSOR_ROW` | 24 | i64 | cursor row (0-based) |
| `TV_CURSOR_COL` | 32 | i64 | cursor column (0-based) |
| `TV_CELL_W` | 40 | f64 | cell width in points |
| `TV_CELL_H` | 48 | f64 | cell height in points |
| `TV_CURSOR_VISIBLE` | 56 | i64 | cursor-visible flag |
| `TV_CUR_FG` | 64 | u32 | current packed fg applied to written cells (default white) |
| `TV_CUR_BG` | 72 | u32 | current packed bg (default 0 = black) |
| `TV_CUR_BOLD` | 80 | i64 | current bold flag |
| `TV_CUR_UNDERLINE` | 88 | i64 | current underline flag |

The `TV_CUR_*` attribute fields (64..96) are the app-mode mirror of the
term-state global: the GUI setters write **both** the global (for readers) and
this view-local copy (readable from the main-thread write/draw path without
touching the worker's `x19`). f64 fields store the raw IEEE bits via
`str`/`ldr`.

### `TermCell` layout (16 bytes, `CELL_SIZE`)

```text
offset  size  field
  0      4    glyph     u32 unichar (0 or 32/space = blank)
  4      4    fg        u32 packed r|g<<8|b<<16
  8      4    bg        u32 packed r|g<<8|b<<16
 12      1    bold      u8
 13      1    underline u8
 14      2    (padding)
```

Cell address: `cells + (row*cols + col) * 16` (`lsl #4`). [[src/target/macos_aarch64/app/term_view.rs:emit_term_view_draw_rect]]

### `term_init` — grid sizing

`void _mfb_macapp_term_init(id termView)`, called once from the bootstrap:

1. `state = calloc(1, 96)`.
2. `font = [NSFont userFixedPitchFontOfSize:N]` (the transcript font size).
3. `cellW = [font maximumAdvancement].width`.
4. `cellH = [[[NSLayoutManager alloc] init] defaultLineHeightForFont:font]`.
5. `cols = floor(900 / cellW)`, `rows = floor(640 / cellH)` (900×640 =
   `TERM_VIEW_WIDTH`×`TERM_VIEW_HEIGHT`, the initial frame). cellW/cellH are
   persisted as f64.
6. `cells = calloc(rows*cols, 16)` — zero = a cleared grid (blank glyphs, black
   bg).
7. cursor `(0,0)` (already zero), `TV_CURSOR_VISIBLE = 1`, `TV_CUR_FG = white`;
   bg/bold/underline default to 0.
8. attach `state` via `TVSTATE_KEY`.

The grid is **not** resized on live window resize (the autoresizing mask scales
the view, but cols/rows are fixed at init). [[src/target/macos_aarch64/app/term_view.rs:emit_term_init_helper]]

### `drawRect:` — the renderer

`void drawRect:(NSRect dirty)` (self x0, _cmd x1, rect d0..d3). Spills the dirty
rect immediately (the FP arg regs are clobbered by the first call). [[src/target/macos_aarch64/app/term_view.rs:emit_term_view_draw_rect]]

1. Fill the dirty rect black: `[[NSColor blackColor] set]; NSRectFill(rect)`.
2. If no state or no `cells`, stop (clean black surface).
3. `font = [NSFont userFixedPitchFontOfSize:N]`; build `attrs =
   [NSMutableDictionary dictionary]` with `NSFontAttributeName = font`.
4. Pre-resolve every selector and attribute key once
   (`colorWithCalibratedRed:green:blue:alpha:`, `set`, `setObject:forKey:`,
   `removeObjectForKey:`, `drawAtPoint:withAttributes:`,
   `stringWithCharacters:length:`, the fg/stroke-width/underline-style keys,
   bold `NSNumber(-3.0)`, underline `NSNumber(1)`), spilling them to the stack —
   so the per-cell loop never calls `sel_registerName` (which would clobber the
   d0..d3 colour-component args).
5. For each cell `(row, col)`:
   - background: if `cell.bg != 0`, `[bgColor set]; NSRectFill(col*cellW,
     row*cellH, cellW, cellH)`.
   - glyph: skip if glyph is 0 or 32 (space). Set the fg colour attribute from
     `cell.fg`; set/remove the faux-bold stroke-width attribute
     (`NSStrokeWidthAttributeName = -3.0`, negative = fill-stroke bold) per
     `cell.bold`; set/remove `NSUnderlineStyleAttributeName = 1` per
     `cell.underline`; build `s = [NSString stringWithCharacters:&glyph
     length:1]`; `[s drawAtPoint:(col*cellW, row*cellH) withAttributes:attrs]`.

Colour decode (`emit_color_from_packed`): `r = (p & 255)/255`, `g = ((p>>8) &
255)/255`, `b = ((p>>16) & 255)/255`, alpha 1.0; NSColor class is held in x26 and
the `colorWith…` selector spilled, so no selector lookup clobbers the components. [[src/target/macos_aarch64/app/term_view.rs:emit_color_from_packed]]

### `mfbWriteString:` — grid writer

`void mfbWriteString:(id self, SEL, NSString *str)`. Invoked on the main thread
via `performSelectorOnMainThread:withObject:waitUntilDone:`, so grid mutation +
redraw are serialized in program order with the other surface ops. Iterates
`[str characterAtIndex:i]`: [[src/target/macos_aarch64/app/term_view.rs:emit_term_write_string_helper]]

- `\n` (10) → newline, `\r` (13) → carriage return, `\t` (9) → tab handling.
- printable: wrap to col 0 / next row when `cursor_col >= cols`; scroll up when
  `cursor_row >= rows` (then clamp to `rows-1`); write the cell at
  `cells + (row*cols+col)*16` — glyph (u32), fg/bg from `TV_CUR_FG`/`TV_CUR_BG`
  (u32), bold/underline from `TV_CUR_BOLD`/`TV_CUR_UNDERLINE` (u8); advance the
  cursor.
- after the loop: `setNeedsDisplay:`.

### `term_scroll` and `term_clear`

`_mfb_macapp_term_scroll(void *state)`: `memmove(cells, cells+rowBytes,
(rows-1)*rowBytes)` then `bzero` the new bottom row, where `rowBytes =
cols*16`. Main-thread only. [[src/target/macos_aarch64/app/term_view.rs:emit_term_scroll_helper]]

`_mfb_macapp_term_clear(id termView)`: resolve state via `TVSTATE_KEY`, `bzero`
the whole grid (`rows*cols*16`) and home the cursor `(0,0)`. Pure heap mutation,
safe from the worker thread. [[src/target/macos_aarch64/app/term_view.rs:emit_term_clear_helper]]

### Content-view swap (`term::on` / `term::off`)

`term::on` (`emit_app_term_on_helper`): [[src/target/macos_aarch64/app/app_io.rs:emit_app_term_on_helper]]

1. Reset the term-state global to defaults (active=1, fg=white, bg=black,
   bold=0, underline=0, cursorVisible=1).
2. `app = [NSApplication sharedApplication]`;
   `window = objc_getAssociatedObject(app, &WINDOW_KEY)`. If nil → headless: only
   the global is updated (so `isOn`/auto-restore stay correct) and the helper
   returns.
3. `termview = objc_getAssociatedObject(app, &TERMVIEW_KEY)`; clear its grid +
   home the cursor (`term_clear`).
4. `[window performSelectorOnMainThread:@selector(setContentView:)
   withObject:termview waitUntilDone:YES]` — AppKit is main-thread only.
5. `[window performSelectorOnMainThread:@selector(makeFirstResponder:)
   withObject:termview waitUntilDone:YES]` — route keys to the surface.

Returns `RESULT_OK_TAG` (0).

`term::off` (`emit_app_term_off_helper`): no-op when already off (the gate
reads `active` off `x19`). Otherwise, with a window attached: [[src/target/macos_aarch64/app/app_io.rs:emit_app_term_off_helper]]

1. `scroll = objc_getAssociatedObject(app, &SCROLLVIEW_KEY)`;
   `setContentView:scroll` on the main thread — restores the transcript.
2. `transcript = objc_getAssociatedObject(app, &TEXTVIEW_KEY)`;
   `makeFirstResponder:transcript` — window input returns to the transcript.
3. Set the global `active = 0`.

Headless `term::off` skips the AppKit work and only clears `active`.

### `keyDown:` (input)

`void keyDown:(id self, SEL, NSEvent *event)` — the TUI-surface analogue of the
transcript's `keyDown:`. Input remains an `io::` concern: raw mode writes the
key's UTF-8 to the window input pipe immediately; line mode buffers until Return,
echoing typed characters into the surface. Runs on the main thread. The
cell/render model itself does not interpret keys. [[src/target/macos_aarch64/app/term_view.rs:emit_term_key_down_helper]]

## Linux: `GtkDrawingArea` + Cairo

The Linux backend is the analog of the macOS `TermView` but structurally
different. The drawing area is created up front, held off-window by a ref, and
swapped in as the window child on `term::on`. The Linux runtime carries a
documented gap: `io::terminalSize` / interactive resize
is absent. (Cursor rendering is implemented — a caret is drawn when the cursor is
visible.) [[src/target/linux_gtk/bootstrap.rs:emit_main_bootstrap]]

### Grid storage — one `_mfb_gtkapp_state` global, parallel static arrays

Unlike macOS's per-view `calloc`'d `TermCell[]`, Linux stores the grid inline in
the single process-wide `_mfb_gtkapp_state` struct as **three parallel arrays**
with a fixed stride, so storage is static (no per-resize realloc). [[src/target/linux_gtk/mod.rs:ST_TERM_CHARS]]

| Field (relative offsets, after the io/input state) | Meaning |
|------|---------|
| `ST_TERM_AREA` | `GtkDrawingArea*` |
| `ST_TERM_ACTIVE` | 1 while `term::` is on |
| `ST_TERM_ROW` / `ST_TERM_COL` | cursor position |
| `ST_TERM_CUR_FG` / `ST_TERM_CUR_BG` | current fg/bg (packed \| flags) |
| `ST_TERM_CUR_BOLD` / `ST_TERM_CUR_UNDERLINE` | current attrs |
| `ST_TERM_CURSOR_VISIBLE` | cursor visibility |
| `ST_TERM_COLS` / `ST_TERM_ROWS` | active extent (derived from window size) |
| `ST_TERM_CELL_W` / `ST_TERM_CELL_H` | cell px metrics |
| `ST_TERM_CHARS` | `u8[160*48]` glyph bytes |
| `ST_TERM_FG` | `u32[160*48]` fg |
| `ST_TERM_BG` | `u32[160*48]` bg |

Backing stride is `TERM_MAX_COLS=160 × TERM_MAX_ROWS=48`; only the top-left
`cols × rows` (derived from the 900×640 content area / monospace cell metrics) are
active. The char array is 1 byte/cell (not a unichar) — ASCII-oriented, unlike
the macOS u32 glyph.

### Cell colour/flag encoding

Linux packs flags into the fg/bg words rather than separate cell bytes: low 24
bits = packed RGB (`r|g<<8|b<<16`, the console convention so the arena getters
agree); **bit 24 = COLOR_SET** (explicit colour, so 0 means "use default" and
black is still distinguishable); **bit 25 (fg) = bold**; **bit 26 (fg) =
underline**. macOS instead carries bold/underline as dedicated `TermCell` bytes
and treats `bg == 0` as "no background fill". [[src/target/linux_gtk/mod.rs:COLOR_SET]]

### Renderer + ops

`_mfb_gtkapp_term_draw(area, cr, w, h, user)` is the `gtk_drawing_area_set_draw_func`
callback: paint the whole area black (`cairo_paint`), then per active cell fill
the bg rect (when COLOR_SET) and `cairo_show_text` the glyph using
`cairo_select_font_face("monospace", …, weight)` at `TERM_FONT_SIZE=16`, in the
cell's fg colour. `emit_cairo_color` divides each packed channel by 255 into
`cairo_set_source_rgb`. [[src/target/linux_gtk/term_draw.rs:emit_term_draw_helper]] [[src/target/linux_gtk/term_draw.rs:emit_cairo_color]]

The surface swap and redraw run as main-loop idle callbacks (GTK calls must run
on the main loop): `_mfb_gtkapp_term_show_idle` / `_hide_idle` / `_redraw_idle`.
The worker-side writer `_mfb_gtkapp_term_write` mutates the grid arrays and
`_mfb_gtkapp_term_scroll` shifts chars/fg/bg up one row at the bottom edge;
`_mfb_gtkapp_term_init` derives the geometry once at activate before the worker
touches the grid. [[src/target/linux_gtk/term_draw.rs:emit_term_show_idle_helper]] [[src/target/linux_gtk/term_draw.rs:emit_term_write_helper]]

Like macOS, the Linux helpers update the shared console term-state global off the
pinned arena register (`ARENA_REG = x19`) so `isOn` and the attribute getters
agree across backends. [[src/target/linux_gtk/mod.rs:ARENA_REG]]

## See Also

* ./mfb spec app macos-runtime — the AppKit `_main` bootstrap, transcript view, and associated-object state scheme
* ./mfb spec app linux-runtime — the GTK4 bootstrap, `_mfb_gtkapp_state` global, and divergences
* ./mfb spec app console-io — `io::write`/`input`/`terminalSize` over the window, and line vs raw key handling
* ./mfb spec memory program-startup — where the term-state global lives in the program-entry frame
* ./mfb spec unicode strings-model — the packed-RGB colour and glyph conventions shared with the console backend
* ./mfb spec threading os-integration — the worker pthread that drives the surface via `performSelectorOnMainThread:` / idle callbacks

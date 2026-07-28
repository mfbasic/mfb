# Unicode Wide-Character Support Plan (umbrella)

Last updated: 2026-07-27
Effort (Human): huge (>3d)
Effort (AI): huge (>3d)

Make the `term::` terminal subsystem render **double-width** glyphs (CJK
ideographs, wide/emoji) and **multi-scalar grapheme clusters** (combining
sequences, ZWJ emoji, regional-indicator flags) correctly across all six
backends — the three console backends (macOS / Linux / Windows, all sharing the
neutral `term_grid.rs` codegen) and the three GUI "app-mode" backends (macOS
AppKit `TermView`, Linux GTK4/Cairo, Windows GDI). Today every backend assumes
**one column per codepoint** (and Windows app mode assumes one column per
*byte*), so a single wide glyph shifts the rest of its row and a combining
sequence is torn across cells. The single behavioral outcome: a program that
draws `"日本語"`, `"👨‍👩‍👧‍👦"`, or `"café"` (NFD) into the grid lays each user-perceived
character in the correct number of columns, with the row after it aligned, on
every backend — measured the way notcurses is (the reference the user named).

This is the follow-up bug-392 explicitly deferred: *"The same +1 auto-advance
assumption also breaks for a real double-width glyph … on every platform
including a correct UTF-8 console. That is a pre-existing grid design constraint
… File it separately if it ever bites."* (`bugs/bug-392-windows-console-utf8-mojibake.md:118-124`).

References (read these first):

- `src/docs/spec/app/04_term-backend.md` — the shared cell model, the console
  shadow-grid header block, and the macOS/Linux app backends.
- `src/docs/spec/unicode/01_tables-and-algorithms.md` — the embedded utf8proc
  property tables, the two-stage lookup, and the grapheme state machine.
- `src/docs/spec/unicode/02_strings-model.md` — the three indexing units
  (scalar / grapheme / byte); this plan adds a fourth (display column).
- `third_party/utf8proc/utf8proc.h:294-318` — the `utf8proc_property_t` struct,
  including the already-vendored `charwidth:2` / `ambiguous_width:1` fields.
- `.ai/compiler.md` — the runtime-completion gate, register-lifetime rules, and
  validation/function-test conventions this plan's codegen must satisfy.

## Prerequisites

Everything below is written against the world where these hold. No hedges for the
world where they don't.

| Must be true | Command | Status |
|---|---|---|
| bug-392 fixed — the Windows console sets the UTF-8 code page, so single-width non-ASCII no longer mojibakes and the grid model's column assumption holds on Windows before we build wide support on top of it | `rg -n 'SetConsoleOutputCP' src/target/win_x86_64/ && ls bugs/completed-bugs/bug-392* 2>/dev/null` (expect a hit + an archived doc) | **NOT MET** (`rg -n 'SetConsoleOutputCP' src/target/win_x86_64/` → no matches; `bugs/bug-392-*.md` is still Open) |
| Full suite green at the branch point (a plan that regenerates goldens must start from a clean baseline) | `cargo test` → `0 failed`; `scripts/artifact-gate.sh` → diffs=0 | UNVERIFIED — re-run before starting A |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue and again
> before you decide to stop. bug-392 is a *precondition*, not scope this plan
> absorbs: if it is not complete, this plan cannot start, full stop. Do **not**
> fold a `SetConsoleOutputCP` call into any letter here.

## Dependency graph

```
A ← nothing            (display-width primitive: the width helper everything consumes)
B ← A                  (console grid cell model + writer/present)
C ← A + B              (console draw helpers stamp into B's cells)
D ← A                  (macOS app TermView)
E ← A                  (Linux GTK app + Pango font)
F ← A                  (Windows app GDI: real grid + font)
G ← B + C + D + E + F   (docs/spec sync + goldens + cross-platform runtime proof)
```

Execution is topological order over this graph. After A lands, B/D/E/F can run in
parallel (distinct files, distinct backends); C waits on B; G rejoins everything.
Each sub-plan is its own document: `plan-70-A-…` … `plan-70-G-…`. The letter is
the position, the slug is the meaning.

## 1. Goal

- Each **extended grapheme cluster** (UAX #29) occupies exactly one grid **cell**
  and advances the cursor by its **display width** in columns: 0 for a
  zero-width cluster (a lone combining mark, ZWSP), 2 for a wide cluster (East
  Asian Wide/Fullwidth, emoji-presentation), 1 otherwise.
- A wide cluster written at the last column of a row **wraps to the next row**
  rather than splitting across the edge.
- The console diff-presenter (`term_grid.rs::emit_grid_present`) advances its
  cursor model by the presented cluster's width so a contiguous run of changed
  cells stays column-aligned without a redundant CUP.
- A new builtin `strings::displayWidth(value) AS Integer` returns the terminal
  column width of a string, so MFB programs can lay out tables/panels correctly.
- All three console backends and all three app backends render CJK/emoji without
  tofu where the platform can supply the glyph (font work is per-backend in
  D/E/F).

### Non-goals (explicit constraints)

- **No change to `String` semantics or the scalar/grapheme/byte indexing model.**
  `mid`/`find` stay scalar-indexed; `graphemes*` stay grapheme-indexed. The new
  display-column unit is additive (`strings::displayWidth` only).
- **No change to the bytes written to files/pipes.** `io::write` to a
  non-terminal is byte-identical UTF-8 (bug-392 non-goal, preserved).
- **No locale-tailored or terminal-negotiated width.** Width is the fixed
  utf8proc `charwidth` table; East-Asian **Ambiguous** width is treated as **1**
  (narrow) — the modern default — with `ambiguous_width` carried but not acted on
  (see Open Decisions).
- **No new external library dependency.** Width comes from the already-vendored
  `third_party/utf8proc/utf8proc_data.c`; the runtime keeps its
  "no-linked-utf8proc" property (`01_tables-and-algorithms.md:38`).
- **No emoji recoloring, VS15/VS16 presentation negotiation beyond width, or
  terminal grapheme-cluster protocol (mode 2027).** Width + one-cluster-per-cell
  only.

## 2. Current State (shared; per-letter detail lives in each sub-plan)

### The width data already exists but is thrown away

The vendored `utf8proc_property_t` carries `charwidth:2` (0/1/2) and
`ambiguous_width:1` (`third_party/utf8proc/utf8proc.h:307-310`);
`utf8proc_charwidth()` returns that field verbatim
(`utf8proc.h:705-713`). But `parse_properties`
(`src/unicode/runtime_tables.rs:253-295`) reads only utf8proc field indices
1,3,4,5,6,7,9,10,{11,13,14,15},19,20 and **skips the charwidth/ambiguous
fields**; `PackedProperty` (`runtime_tables.rs:26-38`) has no width field, and no
codegen offset or emit helper exists (`src/target/shared/code/private/unicode.rs`
offsets stop at `INDIC_CONJUNCT_BREAK` = 20). So a display-width table is a
parser field + a packed-flag bit + a runtime lookup helper away — the
per-codepoint source data is already in-tree.

The `flags` u16 in `PackedProperty` uses only bits 0–3
(`runtime_tables.rs:48-51`: `COMB_IS_SECOND`/`COMP_EXCLUSION`/`IGNORABLE`/
`CONTROL_BOUNDARY`). Bits 4–6 are free, so width (2 bits) + ambiguous (1 bit)
pack into the **existing 24-byte record with no size change** — no offset-constant
churn, no change to the reference sizes asserted at
`runtime_tables.rs:parses_utf8proc_runtime_tables`, and the on-disk table stays
byte-identical except for the flags column. This is the low-risk path A takes
(see plan-70-A §Design; verified: bits 4–6 unused).

### Every backend assumes 1 column per codepoint (or per byte)

| Backend | File / site | Iteration unit | Column advance | Cluster storage |
|---|---|---|---|---|
| Console grid **write** | `src/target/shared/code/term_grid.rs:438` `emit_grid_write`, advance at `:581` | UTF-8 scalar (1–4 B packed into one u32 cell, `:519-555`) | `col += 1` unconditional | one scalar / cell (u32 = ≤4 bytes) |
| Console grid **present** | `term_grid.rs:856` `emit_grid_present`, `last_col = col+1` at `:1064` | one cell | assumes terminal auto-advances 1 col/glyph | — |
| Console **draw helpers** | `src/target/shared/code/term.rs:1486` `emit_draw_text` (`:1616`), `:1406` `emit_draw_glyph`, `:867` `emit_stamp_cell` | scalar | `col += 1` (`:1616`, `:1622`) | one scalar / cell |
| macOS app `TermView` | `src/target/macos_aarch64/app/term_view.rs:1878` `emit_term_write_string_helper`; draw `stringWithCharacters:&glyph length:1` (`:650-665`) | **UTF-16 code unit** (`characterAtIndex:`) | `col += 1` per unit | u32 unichar / cell (`CELL_GLYPH_OFFSET=0`, `mod.rs:467`) — **astral emoji split into surrogate halves → tofu** |
| Linux GTK app | `src/target/linux_gtk/term_draw.rs` `_mfb_gtkapp_term_write`; decode `emit_utf8_decode_at` (`:21`) | UTF-8 scalar → u32 codepoint | `col += 1` per scalar | u32 codepoint / cell (`ST_TERM_CHARS`, stride 160×48) |
| Windows app GDI | `src/target/win_x86_64/app/mod.rs:887-937` (term path of `emit_app_io_write_helper`) | **raw byte** (each byte → a UTF-16 unit via `store_u16`) | `col += 1` per **byte** | **no grid at all** — immediate-mode `TextOutW` into a memDC; cursor-only state |

The console cell stores packed UTF-8 bytes (up to 4) in a u32 and reconstructs
them in `append_glyph` (`term_grid.rs:186`); a blank (0) emits a space. So the
console can already hold one full 4-byte scalar per cell, but **not** a
multi-scalar cluster. The macOS/Linux app cells hold one codepoint; Windows app
has no cell store. None of the six has any width concept, a wide-trailing/
continuation marker, or cluster storage.

### Fonts (the bug-392-style audit the user asked for)

| Backend | Surface | Font selected | API | CJK/emoji fallback | Site |
|---|---|---|---|---|---|
| macOS | transcript + TUI grid (same face) | `[NSFont userFixedPitchFontOfSize:13]` (Menlo) | CoreText / `drawAtPoint:` | CoreText auto-cascades (transcript ok); **grid geometry sized from `maximumAdvancement` of one face** so a substituted wide glyph still overflows its single cell | `mod.rs:155-159`, `term_view.rs:441-447`, `:770-814` |
| Linux | TUI grid | `cairo_select_font_face("monospace")` @16 | **Cairo toy API** | **none — no cascade → tofu** | `term_draw.rs:275-289`, `mod.rs:155` |
| Linux | transcript | theme monospace | **Pango** (has cascade) | ok | `bootstrap.rs:186-197` |
| Windows | TUI grid | `GetStockObject(SYSTEM_FIXED_FONT)` (legacy bitmap fixed font) | GDI `TextOutW` | **none — no font-linking → tofu**; the `mod.rs:52` comment claims "Consolas metrics" but Consolas is never selected | `mod.rs:1146-1152` |
| Windows | transcript | default GUI font (no `WM_SETFONT`, variable-pitch) | GDI `EDIT` | OS default | `mod.rs:329-351` |

Two grids will show tofu even after width is correct: the **Linux Cairo-toy TUI
grid** and the **Windows `SYSTEM_FIXED_FONT` GDI grid** — single face, no cascade,
fixed cell geometry. Fixing those fonts is the font half of E and F.

### Measured populations

| What | Count | Command |
|---|---|---|
| Console column-advance sites to make width-aware | 2 (`emit_grid_write` write-advance `term_grid.rs:581`; `emit_grid_present` present-advance `:1064`/`:1072`) | `rg -n 'add_immediate\(col, col, 1\)\|add_immediate\(last_col, col, 1\)' src/target/shared/code/term_grid.rs → 3 hits (2 functions)` |
| Console draw-helper advance sites | 3 (`emit_draw_text` `:1616` and `:1622`; the stamp path shares `emit_stamp_cell`) | `rg -n 'add_immediate\(col, col, 1\)' src/target/shared/code/term.rs → :1616 :1622` |
| App writers to convert (scalar/byte → grapheme+width) | 3 (macOS `term_view.rs:1878`, Linux `term_draw.rs` write helper, Windows `mod.rs:887`) | one per backend, enumerated above |
| `strings::` builtins whose "width" is a scalar count (candidates for a width-aware variant) | 4 (`padLeft`,`padRight` `strings.rs:25-26`; `left`,`right`) | `rg -n 'padLeft\|padRight\|"left"\|"right"' src/builtins/strings.rs` |
| Backends with **no** cell grid at all | 1 (Windows app, `mod.rs:45-58`) | read `mod.rs:45-58` — "no cell buffer" design comment |

### Verified properties

- **`PackedProperty.flags` bits 4–6 are free** — read `runtime_tables.rs:48-51`
  and `:266-289`; only bits 0–3 are ever set. Width can pack in with no record
  resize. (Verified by reading the flag assignments, not just locating them.)
- **The console cell holds ≤1 scalar, never a cluster** — read
  `term_grid.rs:519-576` and `append_glyph:186`: the u32 packs one scalar's UTF-8
  bytes; a combining mark would need its own cell. Cluster support therefore
  needs new storage, not just a width field. (Property claim, verified by read.)
- **macOS app splits astral scalars** — read `term_view.rs:650-665`
  (`stringWithCharacters:&glyph length:1`, one UTF-16 unit) and the
  `characterAtIndex:` writer loop: a surrogate pair becomes two cells each
  holding a lone surrogate. Fixing width alone would not fix this; D must decode
  to scalars/clusters. (Verified by read.)
- **Windows app is not even codepoint-correct** — read `mod.rs:887-937`: it
  iterates raw bytes and draws each as a UTF-16 unit, so any multi-byte scalar is
  already garbage. F is a grid rewrite, not a width patch. (Verified by read.)
- **The console draw helpers advance 1 col/scalar** — read `term.rs:1613-1622`.
  (Verified by read.)
- UNVERIFIED: whether the East-Asian **Ambiguous** default should be 1 or 2 — a
  policy call, not a code fact. Tracked in Open Decisions; A carries the
  `ambiguous` bit so a later flip is a one-line codegen change, not a table
  rebuild.

## 3. Design Overview

The feature layers as: **(A) one width primitive** → **(B/C) console** and
**(D/E/F) app backends** each grow a *grapheme-cluster, width-aware cell model* →
**(G) docs + cross-platform proof**.

**The cell-model change is the heart, and it is the same shape on every
backend:**

1. A cell stores a whole **extended grapheme cluster (EGC)**, not one scalar.
   The common case (a single scalar ≤4 bytes) stays inline in the existing u32
   glyph field; a multi-scalar cluster is stored in a per-grid **EGC pool** (a
   growable byte arena) with the glyph field holding a tagged offset
   (high-bit = "pooled", low bits = offset), mirroring notcurses' `egcpool`. This
   is the notcurses-quality path the user asked for. (See Open Decisions for the
   inline-cap-only alternative.)
2. A cell carries a **width** (0/1/2). A **wide** cluster occupies its primary
   cell (glyph + width 2) followed by one **wide-trailing sentinel** cell (a
   reserved glyph value, e.g. `0xFFFF_FFFF`) that the presenter/renderer skips.
3. The **writer** segments its input into graphemes (reusing the existing UAX #29
   walker that already backs `strings::graphemes`), computes each cluster's width
   from A's helper, wraps before a wide cluster that would straddle the right
   edge, writes the primary cell + trailing sentinel(s), and advances the cursor
   by width.
4. The **presenter/renderer** emits the primary cluster's bytes, skips sentinel
   cells, and advances its own column model by the cluster width (console:
   `last_col += width`; app: the trailing cell draws nothing).

**Where design uncertainty concentrates (schedule first, cheaply):** the EGC pool
representation and its lifecycle (allocation, growth, reclamation on
overwrite/scroll/resize). plan-70-A ships the width primitive with a **standalone
`strings::displayWidth` builtin and a unit-tested display-width value** — that
falsifies the width-table plumbing *before* any grid work depends on it. plan-70-B
Phase 1 is a spike that stores width + wide-trailing sentinels for **single-scalar
wide** glyphs only (no pool), proving the 2-cell layout and the presenter advance
end-to-end on a real terminal; the EGC pool for multi-scalar clusters is a later
phase in B once the width path is proven.

**Where correctness risk / blast radius concentrates (schedule last):** the
console `emit_grid_present` diff loop (a mis-advance corrupts the whole frame,
per bug-392's cascade analysis) and the golden regeneration in G (the `.ncodesum`
artifact-gate goldens and the acceptance TUI goldens all shift). G lands last,
behind every other letter's tests, and regenerates goldens in one reviewable
commit.

**Rejected alternatives:**

- *Store width but keep one scalar per cell (no EGC pool).* Rejected as the
  final design because combining sequences (`"e" + U+0301`) and ZWJ emoji
  families still tear — exactly the "not compared to notcurses" gap the user
  called out. Kept only as plan-70-B Phase 1's intentionally-narrow spike.
- *Recompute width in the presenter from the cell's glyph bytes.* Rejected: the
  presenter would re-run a property lookup per cell every frame; storing the
  width in the cell at write time is O(1) at present and matches how fg/bg/bold
  are already cached in the cell.
- *A separate `_mfb_unicode_charwidth` runtime function symbol.* Rejected for
  parity with the existing design — all Unicode algorithms are **inlined** at
  the call site (`01_tables-and-algorithms.md:20-27`); A adds an
  `emit_unicode_property_charwidth` codegen helper, not a called function.
- *Widen the console/app cell to hold N inline bytes.* Rejected: unbounded
  clusters (ZWJ families are 20+ bytes) make any fixed inline size wrong; the
  pool is the only correct bound.

## Compatibility / Format Impact

- **Changes:** the on-disk `_mfb_unicode_properties` table's `flags` column gains
  bits 4–6 (width/ambiguous) — same 24-byte record, byte values differ. Every
  program that uses a Unicode-aware builtin re-embeds the (slightly changed)
  table, so the `.ncodesum` artifact-gate goldens for those fixtures shift (G
  regenerates them). The console shadow-grid block grows by an EGC pool region
  (header layout in `04_term-backend.md` gains a field; slot 56 "reserved" or a
  new header field holds the pool base). The macOS `TermView` `TVSTATE` and the
  Linux `_mfb_gtkapp_state` gain width + pool fields.
- **Unchanged:** `String` heap layout; scalar/grapheme/byte builtin semantics;
  bytes written to files/pipes; the packed-RGB colour convention; the
  `term::`/`io::` public call surface (only the additive `strings::displayWidth`
  is new); the VT-processing and code-page behavior (owned by bug-392).

## Phases

This umbrella has no phases of its own; each lettered sub-plan carries its own
independently-landable phases, ordered uncertainty-first / blast-radius-last. See:

- **plan-70-A** — display-width primitive (Unicode runtime + `strings::displayWidth`).
- **plan-70-B** — console grid cell model: width, wide-trailing cells, EGC pool,
  writer + present.
- **plan-70-C** — console draw helpers (`drawText`/`drawGlyph`/`drawBox`/
  `fillRect`) made cluster/width-aware.
- **plan-70-D** — macOS AppKit `TermView`: grapheme decode, astral fix, width layout.
- **plan-70-E** — Linux GTK app: width layout + Cairo-toy→Pango font migration.
- **plan-70-F** — Windows GDI app: real cell grid, grapheme decode, width, and a
  CJK-capable font with font-linking (replace `SYSTEM_FIXED_FONT`).
- **plan-70-G** — docs/spec sync, golden regeneration, and cross-platform runtime
  proof on the macOS host + Linux box + Windows box 2230.

## Validation Plan (feature-level; each letter restates its own)

- **Tests:** per-letter cargo unit/function tests for the width helper and the
  grid writer/present; acceptance TUI goldens for a CJK + emoji + combining
  fixture (G seeds them per the acceptance-golden-harness mechanics).
- **Coverage check:** the width helper and the new grid paths must be in the
  suite denominator — a CJK/emoji fixture that actually exercises `charwidth==2`
  and a pooled cluster, not just ASCII (a green gate on ASCII proves nothing
  here).
- **Runtime proof:** the `browser` example plus a new `wide-demo` fixture drawing
  `"日本語 | 👨‍👩‍👧‍👦 | café(NFD)"` in a bordered box, run on: the macOS host, a Linux
  box, and Windows box 2230 — the box borders must stay aligned around the wide
  content (the notcurses-parity bar).
- **Doc sync:** `src/docs/spec/unicode/01_tables-and-algorithms.md` (PackedProperty
  flags), `02_strings-model.md` (new display-column unit), `04_term-backend.md`
  (cell model + EGC pool), and `mfb man strings displayWidth`.
- **Acceptance:** `cargo test` + `scripts/artifact-gate.sh` (diffs=0 after G's
  golden regen) + the acceptance TUI goldens.

## Open Decisions

- **East-Asian Ambiguous width = 1 or 2?** Recommend **1** (narrow) — the modern
  terminal default; carry the `ambiguous_width` bit (A) so a policy flip is a
  one-line codegen change, not a table rebuild. (§Non-goals)
- **EGC pool now, or inline-cap-only first?** Recommend the **EGC pool** (B) for
  notcurses parity, with plan-70-B Phase 1 shipping single-scalar-wide-only as
  the de-risking spike. Alternative: cap clusters at the inline u32 and drop the
  tail of longer clusters — simpler, but leaves combining/ZWJ broken, which is
  the whole point. (§3)
- **Windows app: extend the shared neutral grid, or a Windows-local grid?**
  Recommend a **Windows-local cell grid** in `mod.rs` mirroring the macOS/Linux
  app shape (the neutral `term_grid.rs` is console-only, coupled to VT escape
  output, which the GDI blit path does not use). (plan-70-F)
- **A width-aware `padRight`/`left` variant, or only `strings::displayWidth`?**
  Recommend shipping **only `displayWidth`** in A (additive, minimal surface) and
  leaving column-aware padding to a follow-up once `displayWidth` exists; folding
  a width mode into `padRight` changes an existing builtin's contract. (§Non-goals)

## Corrections

<Filled in during execution.>

## Summary

The engineering risk is concentrated in two places: the console `emit_grid_present`
diff loop, where a width mis-advance corrupts the entire frame (the same failure
mode bug-392 documented), and the Windows GDI app backend, which has no cell grid
at all and must be built from scratch before width is even meaningful. The
enabling asset is that the per-codepoint width data is **already vendored** in
utf8proc and can be plumbed into the existing property table with no record-size
change — so the hard part is not the data, it is teaching six renderers to place
one grapheme cluster in the right number of columns. bug-392 must land first;
`String` semantics, file/pipe output, and the public call surface are untouched.
